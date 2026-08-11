use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde_json::{json, Value};

const MODEL: &str = "deepseek-v4-flash";
const DEFAULT_ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
const DEFAULT_MAX_TURNS: usize = 32;
const DEFAULT_MAX_TOOL_CALLS: usize = 64;
const DEFAULT_MAX_TOOL_OUTPUT_BYTES: usize = 65_536;
const DEFAULT_MAX_RESPONSE_TOKENS: usize = 4_096;
const DEFAULT_TIMEOUT_SECONDS: usize = 120;

#[derive(Clone, Debug)]
struct Config {
    max_turns: usize,
    max_tool_calls: usize,
    max_tool_output_bytes: usize,
    max_response_tokens: usize,
    timeout_seconds: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_turns: DEFAULT_MAX_TURNS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            max_tool_output_bytes: DEFAULT_MAX_TOOL_OUTPUT_BYTES,
            max_response_tokens: DEFAULT_MAX_RESPONSE_TOKENS,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

pub fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let name = args
        .next()
        .filter(|name| !name.starts_with('-'))
        .ok_or_else(|| usage().to_string())?;
    crate::validate_name(&name)?;
    let config = parse_options(args)?;
    let api_key = api_key_from_env()?;
    let home = crate::home_dir()?;
    let root = home.join(".bo");
    let root = root
        .canonicalize()
        .map_err(|error| format!("canonicalizing {} failed: {error}", root.display()))?;
    let target = root.join(&name);
    let target = target
        .canonicalize()
        .map_err(|_| format!("target directory does not exist: {}", target.display()))?;
    ensure_inside(&target, &root)?;
    if !target.is_dir() {
        return Err(format!("target is not a directory: {}", target.display()));
    }

    let endpoint = env::var("BO_API_URL").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
    let written = run_agent(&root, &target, &api_key, &endpoint, &config)?;
    println!("{written} summaries written");
    Ok(())
}

fn usage() -> &'static str {
    "usage: bo agent <dir> [--max-turns N] [--max-tool-calls N] [--max-tool-output-bytes N] [--max-response-tokens N] [--timeout-seconds N]"
}

fn api_key_from_env() -> Result<String, String> {
    env::var("BO_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "BO_API_KEY is not set".to_string())
}

fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
    let mut config = Config::default();
    while let Some(option) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("missing value for {option}"))
        };
        let number = |value: String| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{option} requires a positive integer"))
                .and_then(|number| {
                    (number > 0)
                        .then_some(number)
                        .ok_or_else(|| format!("{option} requires a positive integer"))
                })
        };
        match option.as_str() {
            "--max-turns" => config.max_turns = number(value()?)?,
            "--max-tool-calls" => config.max_tool_calls = number(value()?)?,
            "--max-tool-output-bytes" => config.max_tool_output_bytes = number(value()?)?,
            "--max-response-tokens" => config.max_response_tokens = number(value()?)?,
            "--timeout-seconds" => config.timeout_seconds = number(value()?)?,
            _ => return Err(usage().to_string()),
        }
    }
    Ok(config)
}

fn run_agent(
    root: &Path,
    target: &Path,
    api_key: &str,
    endpoint: &str,
    config: &Config,
) -> Result<usize, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("canonicalizing {} failed: {error}", root.display()))?;
    let target = target
        .canonicalize()
        .map_err(|error| format!("canonicalizing {} failed: {error}", target.display()))?;
    ensure_inside(&target, &root)?;
    let documents = discover_documents(&root, &target)?;
    if documents.is_empty() {
        return Err(format!("no raw Markdown documents in {}", target.display()));
    }
    let state = crate::load_state(&target)?;
    let sources = source_groups(&documents, &state);
    let mut context = ToolContext {
        root,
        target: target.clone(),
        documents,
        sources,
        state,
        cwd: target,
        max_output_bytes: config.max_tool_output_bytes,
        state_read: false,
    };
    let prompt = system_prompt(&context);
    let document_names: Vec<_> = context.documents.keys().cloned().collect();
    let mut messages = vec![
        json!({"role": "system", "content": prompt}),
        json!({
            "role": "user",
            "content": format!(
                "Call read_state first. Then inspect the latest raw snapshot for every source identity and write one concise Markdown summary per source. Raw documents: {}",
                document_names.join(", ")
            )
        }),
    ];
    let client = Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds as u64))
        .build()
        .map_err(|error| format!("creating API client failed: {error}"))?;
    let mut summarized = HashSet::new();
    let mut turns = 0;
    let mut tool_calls = 0;
    let mut correction_sent = false;

    loop {
        if turns >= config.max_turns {
            return Err(format!(
                "max turns reached ({}) with {} of {} summaries written",
                config.max_turns,
                summarized.len(),
                context.sources.len()
            ));
        }
        turns += 1;
        let message = request_completion(
            &client,
            endpoint,
            api_key,
            &messages,
            config.max_response_tokens,
        )?;
        let calls = message
            .get("tool_calls")
            .filter(|calls| !calls.is_null())
            .map(|calls| {
                calls
                    .as_array()
                    .ok_or_else(|| "assistant tool_calls is not an array".to_string())
            })
            .transpose()?;

        if let Some(calls) = calls.filter(|calls| !calls.is_empty()) {
            messages.push(message.clone());
            for call in calls {
                if tool_calls >= config.max_tool_calls {
                    return Err(format!(
                        "max tool calls reached ({}) with {} of {} summaries written",
                        config.max_tool_calls,
                        summarized.len(),
                        context.sources.len()
                    ));
                }
                tool_calls += 1;
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "assistant tool call has no id".to_string())?;
                let result = execute_tool_call(&mut context, call, &mut summarized);
                let content = match result {
                    Ok(content) => content,
                    Err(error) => format!("ERROR: {error}"),
                };
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": bounded_output(&content, config.max_tool_output_bytes),
                }));
            }
            continue;
        }

        let missing: Vec<_> = context
            .sources
            .keys()
            .filter(|source_key| !summarized.contains(*source_key))
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(summarized.len());
        }
        if correction_sent {
            return Err(format!(
                "model stopped with missing summaries: {}",
                missing.join(", ")
            ));
        }
        correction_sent = true;
        messages.push(message);
        messages.push(json!({
            "role": "user",
            "content": format!(
                "You stopped before completing the task. Use the bounded tools now and write successful summaries for every missing source identity: {}",
                missing.join(", ")
            )
        }));
    }
}

fn request_completion(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    messages: &[Value],
    max_response_tokens: usize,
) -> Result<Value, String> {
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&json!({
            "model": MODEL,
            "messages": messages,
            "tools": tools(),
            "tool_choice": "auto",
            "stream": false,
            "max_tokens": max_response_tokens,
            "thinking": {"type": "disabled"}
        }))
        .send()
        .map_err(|error| format!("API request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("reading API response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "DeepSeek HTTP {status}: {}",
            bounded_output(&body, 512)
        ));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("malformed API response: {error}"))?;
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
        .ok_or_else(|| "malformed API response: missing choices[0].message".to_string())
}

fn tools() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Run one bounded facade command: ls [path], cd path, cat raw.md, or grep literal [path]. This is not a shell.",
                "parameters": {
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"],
                    "additionalProperties": false
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_state",
                "description": "Read the authoritative state for the target directory.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_summary",
                "description": "Read the existing Markdown summary for one source identity.",
                "parameters": {
                    "type": "object",
                    "properties": {"source_key": {"type": "string"}},
                    "required": ["source_key"],
                    "additionalProperties": false
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_summary",
                "description": "Write or replace the Markdown summary for one source identity using its newest raw snapshot.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "source_key": {"type": "string"},
                        "markdown": {"type": "string"}
                    },
                    "required": ["source_key", "markdown"],
                    "additionalProperties": false
                }
            }
        }
    ])
}

fn system_prompt(context: &ToolContext) -> String {
    format!(
        "You are bo's bounded document-summary agent for {}. Call read_state before any other tool. The state object is authoritative: each exact raw URL is one source identity, and raw:filename identifies a Markdown file with no state record. For each source identity, use the newest raw snapshot by written_at as evidence. If a summary record exists, call read_summary with its source_key before replacing it. Never modify or delete raw files. Summarize only facts present in each source. Preserve epistemic status: clearly attribute author experience or measurements (for example, 'the author reports'), recommendations or opinions (for example, 'the article recommends'), and predictions or forecasts (for example, 'the author predicts'); do not present those as general facts. Preserve qualifications and uncertainty while staying concise. Write one concise Markdown summary per source identity with write_summary using source_key, not a raw filename. Use only the provided bounded tools; bash is a strict facade, not a shell. The raw filenames discovered at start are: {}.",
        context.target.display(),
        context.documents.keys().cloned().collect::<Vec<_>>().join(", ")
    )
}

struct Source {
    latest_filename: String,
    latest_written_at: u128,
    latest_state_index: usize,
}

struct ToolContext {
    root: PathBuf,
    target: PathBuf,
    documents: BTreeMap<String, PathBuf>,
    sources: BTreeMap<String, Source>,
    state: crate::State,
    cwd: PathBuf,
    max_output_bytes: usize,
    state_read: bool,
}

fn discover_documents(root: &Path, target: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut documents = BTreeMap::new();
    let entries = fs::read_dir(target)
        .map_err(|error| format!("reading {} failed: {error}", target.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("reading target entry failed: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("reading {} failed: {error}", path.display()))?;
        let resolved = if metadata.file_type().is_symlink() {
            let resolved = path
                .canonicalize()
                .map_err(|error| format!("resolving {} failed: {error}", path.display()))?;
            ensure_inside(&resolved, root)?;
            resolved
        } else {
            path.clone()
        };
        if !resolved.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        let resolved = resolved
            .canonicalize()
            .map_err(|error| format!("resolving {} failed: {error}", path.display()))?;
        ensure_inside(&resolved, root)?;
        ensure_inside(&resolved, target)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("non-UTF-8 raw filename: {}", path.display()))?;
        documents.insert(name, resolved);
    }
    Ok(documents)
}

fn source_groups(
    documents: &BTreeMap<String, PathBuf>,
    state: &crate::State,
) -> BTreeMap<String, Source> {
    let mut sources = BTreeMap::new();
    for filename in documents.keys() {
        let (source_key, written_at, state_index) = state
            .raw
            .iter()
            .enumerate()
            .find(|(_, record)| record.filename == *filename)
            .map(|(index, record)| (record.url.clone(), record.written_at, index))
            .unwrap_or_else(|| (format!("raw:{filename}"), 0, 0));
        let replace = sources.get(&source_key).is_none_or(|source: &Source| {
            (written_at, state_index) > (source.latest_written_at, source.latest_state_index)
        });
        if replace {
            sources.insert(
                source_key,
                Source {
                    latest_filename: filename.clone(),
                    latest_written_at: written_at,
                    latest_state_index: state_index,
                },
            );
        }
    }
    sources
}

fn execute_tool_call(
    context: &mut ToolContext,
    call: &Value,
    summarized: &mut HashSet<String>,
) -> Result<String, String> {
    let function = call
        .get("function")
        .and_then(Value::as_object)
        .ok_or_else(|| "tool call is missing function".to_string())?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tool call is missing function.name".to_string())?;
    if name != "read_state" && !context.state_read {
        return Err("read_state must be called before other tools".to_string());
    }
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} arguments are not a JSON string"))?;
    let arguments: Value = serde_json::from_str(arguments)
        .map_err(|error| format!("{name} arguments are malformed JSON: {error}"))?;
    let arguments = arguments
        .as_object()
        .ok_or_else(|| format!("{name} arguments must be a JSON object"))?;
    match name {
        "read_state" => {
            if !arguments.is_empty() {
                return Err("read_state arguments must be empty".to_string());
            }
            context.state_read = true;
            serde_json::to_string_pretty(&context.state)
                .map(|state| bounded_output(&state, context.max_output_bytes))
                .map_err(|error| format!("serializing state failed: {error}"))
        }
        "bash" => {
            if arguments.len() != 1 {
                return Err("bash arguments must contain only command".to_string());
            }
            let command = arguments
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| "bash.command must be a string".to_string())?;
            execute_bash(context, command)
        }
        "read_summary" => {
            if arguments.len() != 1 {
                return Err("read_summary arguments must contain only source_key".to_string());
            }
            let source_key = arguments
                .get("source_key")
                .and_then(Value::as_str)
                .ok_or_else(|| "read_summary.source_key must be a string".to_string())?;
            read_summary(context, source_key)
        }
        "write_summary" => {
            if arguments.len() != 2 {
                return Err(
                    "write_summary arguments must contain only source_key and markdown".to_string(),
                );
            }
            let source_key = arguments
                .get("source_key")
                .and_then(Value::as_str)
                .ok_or_else(|| "write_summary.source_key must be a string".to_string())?;
            let markdown = arguments
                .get("markdown")
                .and_then(Value::as_str)
                .ok_or_else(|| "write_summary.markdown must be a string".to_string())?;
            write_summary(context, source_key, markdown)?;
            summarized.insert(source_key.to_string());
            Ok(format!("summary written: {source_key}"))
        }
        _ => Err(format!("unsupported tool: {name}")),
    }
}

fn execute_bash(context: &mut ToolContext, command: &str) -> Result<String, String> {
    if command.is_empty()
        || command.chars().any(|character| {
            matches!(
                character,
                '|' | '&' | ';' | '>' | '<' | '$' | '`' | '(' | ')' | '{' | '}' | '\n' | '\r'
            )
        })
    {
        return Err("unsupported shell syntax".to_string());
    }
    let parts: Vec<_> = command.split_whitespace().collect();
    match parts.as_slice() {
        ["ls"] => list_directory(context, &context.cwd),
        ["ls", path] => list_directory(context, &context.resolve(path)?),
        ["cd", path] => {
            let path = context.resolve(path)?;
            if !path.is_dir() {
                return Err(format!("not a directory: {}", path.display()));
            }
            context.cwd = path.clone();
            Ok(format!("directory: {}", path.display()))
        }
        ["cat", path] => {
            let path = context.resolve(path)?;
            let raw_path = context.raw_path(&path)?;
            read_bounded(raw_path, context.max_output_bytes)
        }
        ["grep", pattern] => grep(context, pattern, None),
        ["grep", pattern, path] => grep(context, pattern, Some(path)),
        _ => Err("unsupported command grammar".to_string()),
    }
}

impl ToolContext {
    fn resolve(&self, input: &str) -> Result<PathBuf, String> {
        if input.is_empty() || input.contains('\0') {
            return Err("path is empty or contains NUL".to_string());
        }
        let path = if input == "~/.bo" {
            self.root.clone()
        } else if let Some(relative) = input.strip_prefix("~/.bo/") {
            self.root.join(relative)
        } else {
            let path = Path::new(input);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.cwd.join(path)
            }
        };
        let path = path
            .canonicalize()
            .map_err(|error| format!("resolving {input} failed: {error}"))?;
        ensure_inside(&path, &self.root)?;
        Ok(path)
    }

    fn raw_path(&self, path: &Path) -> Result<&Path, String> {
        self.documents
            .values()
            .find(|candidate| candidate.as_path() == path)
            .map(PathBuf::as_path)
            .ok_or_else(|| "cat is limited to raw Markdown documents".to_string())
    }
}

fn list_directory(context: &ToolContext, path: &Path) -> Result<String, String> {
    if path.is_file() {
        return Ok(format!(
            "{}\n",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    if !path.is_dir() {
        return Err(format!("not a file or directory: {}", path.display()));
    }
    let mut names = Vec::new();
    for entry in
        fs::read_dir(path).map_err(|error| format!("listing {} failed: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("reading directory entry failed: {error}"))?;
        let resolved = entry
            .path()
            .canonicalize()
            .map_err(|error| format!("resolving {} failed: {error}", entry.path().display()))?;
        ensure_inside(&resolved, &context.root)?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(bounded_output(
        &format!("{}\n", names.join("\n")),
        context.max_output_bytes,
    ))
}

fn grep(context: &ToolContext, pattern: &str, path: Option<&str>) -> Result<String, String> {
    let files: Vec<_> = match path {
        None => context.documents.iter().collect(),
        Some(path) => {
            let path = context.resolve(path)?;
            if path.is_dir() {
                context
                    .documents
                    .iter()
                    .filter(|(_, raw)| raw.starts_with(&path))
                    .collect()
            } else {
                let raw = context.raw_path(&path)?;
                context
                    .documents
                    .iter()
                    .filter(|(_, candidate)| candidate.as_path() == raw)
                    .collect()
            }
        }
    };
    let mut output = String::new();
    for (name, path) in files {
        let file = File::open(path)
            .map_err(|error| format!("reading {} failed: {error}", path.display()))?;
        for (line_number, line) in BufReader::new(file).lines().enumerate() {
            let line =
                line.map_err(|error| format!("reading {} failed: {error}", path.display()))?;
            if line.contains(pattern) {
                output.push_str(&format!("{name}:{}:{line}\n", line_number + 1));
            }
        }
    }
    if output.is_empty() {
        output.push_str("(no matches)\n");
    }
    Ok(bounded_output(&output, context.max_output_bytes))
}

fn read_bounded(path: &Path, limit: usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| format!("reading {} failed: {error}", path.display()))?
        .take(limit as u64 + 4)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading {} failed: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(bounded_output(&text, limit))
}

fn read_summary(context: &ToolContext, source_key: &str) -> Result<String, String> {
    let record = context
        .state
        .summaries
        .iter()
        .find(|record| record.source_key == source_key)
        .ok_or_else(|| format!("no summary exists for source: {source_key}"))?;
    let path = summary_path(context, &record.filename)?;
    read_bounded(&path, context.max_output_bytes)
}

fn write_summary(
    context: &mut ToolContext,
    source_key: &str,
    markdown: &str,
) -> Result<(), String> {
    let source = context
        .sources
        .get(source_key)
        .ok_or_else(|| format!("unknown source: {source_key}"))?;
    if markdown.trim().is_empty() {
        return Err("summary Markdown must be non-empty".to_string());
    }
    if markdown.len() > context.max_output_bytes {
        return Err(format!(
            "summary exceeds max tool output bytes ({})",
            context.max_output_bytes
        ));
    }
    let existing = context
        .state
        .summaries
        .iter()
        .find(|record| record.source_key == source_key);
    let filename = existing
        .map(|record| record.filename.clone())
        .unwrap_or_else(|| source.latest_filename.clone());
    let path = prepare_summary_path(context, &filename)?;
    write_summary_file(&path, markdown)?;

    let now = timestamp()?;
    let (created_at, updated_at) = existing
        .map(|record| (record.created_at, now.max(record.updated_at + 1)))
        .unwrap_or((now, now));
    let record = crate::SummaryRecord {
        filename,
        source_key: source_key.to_string(),
        derived_from: source.latest_filename.clone(),
        created_at,
        updated_at,
    };
    if let Some(existing) = context
        .state
        .summaries
        .iter_mut()
        .find(|record| record.source_key == source_key)
    {
        *existing = record;
    } else {
        context.state.summaries.push(record);
    }
    crate::write_state(&context.target, &context.state)
}

fn summary_path(context: &ToolContext, filename: &str) -> Result<PathBuf, String> {
    let summaries = context.target.join("summaries");
    if !summaries.is_dir() {
        return Err(format!(
            "summaries directory does not exist: {}",
            summaries.display()
        ));
    }
    canonical_summary_path(context, &summaries, filename)
}

fn prepare_summary_path(context: &ToolContext, filename: &str) -> Result<PathBuf, String> {
    let summaries = context.target.join("summaries");
    if fs::symlink_metadata(&summaries)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("summaries must not be a symlink".to_string());
    }
    fs::create_dir_all(&summaries)
        .map_err(|error| format!("creating {} failed: {error}", summaries.display()))?;
    let summaries = summaries
        .canonicalize()
        .map_err(|error| format!("canonicalizing summaries failed: {error}"))?;
    ensure_inside(&summaries, &context.target)?;
    canonical_summary_path(context, &summaries, filename)
}

fn canonical_summary_path(
    context: &ToolContext,
    summaries: &Path,
    filename: &str,
) -> Result<PathBuf, String> {
    if filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains('/')
        || filename.contains('\\')
        || !filename
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("md"))
    {
        return Err("summary filename must be a Markdown file name".to_string());
    }
    let destination = summaries.join(filename);
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        if metadata.file_type().is_symlink() {
            return Err("summary destination must not be a symlink".to_string());
        }
        if !metadata.is_file() {
            return Err("summary destination is not a file".to_string());
        }
        let resolved = destination
            .canonicalize()
            .map_err(|error| format!("canonicalizing summary failed: {error}"))?;
        ensure_inside(&resolved, summaries)?;
    }
    ensure_inside(&destination, summaries)?;
    ensure_inside(summaries, &context.target)?;
    Ok(destination)
}

fn write_summary_file(destination: &Path, markdown: &str) -> Result<(), String> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temporary =
        destination.with_file_name(format!(".bo-summary-tmp-{}-{suffix}", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(markdown.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, destination)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("writing {} failed: {error}", destination.display()));
    }
    Ok(())
}

fn timestamp() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| error.to_string())
}

fn ensure_inside(path: &Path, root: &Path) -> Result<(), String> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(format!(
            "path escapes {}: {}",
            root.display(),
            path.display()
        ))
    }
}

fn bounded_output(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let marker = format!("\n[truncated at {} bytes]\n", value.len());
    if marker.len() >= limit {
        return take_prefix(&marker, limit).to_string();
    }
    format!("{}{}", take_prefix(value, limit - marker.len()), marker)
}

fn take_prefix(value: &str, limit: usize) -> &str {
    &value[..value.floor_char_boundary(limit.min(value.len()))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_state, write_state};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    fn temp_root(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            env::temp_dir().join(format!("bo-agent-{label}-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("notes")).unwrap();
        write_state(&root.join("notes"), &crate::State::default()).unwrap();
        root
    }

    fn context(root: &Path) -> ToolContext {
        let root = root.canonicalize().unwrap();
        let target = root.join("notes").canonicalize().unwrap();
        let documents = discover_documents(&root, &target).unwrap();
        let state = crate::load_state(&target).unwrap();
        let sources = source_groups(&documents, &state);
        ToolContext {
            root,
            target: target.clone(),
            documents,
            sources,
            state,
            cwd: target,
            max_output_bytes: 256,
            state_read: false,
        }
    }

    #[test]
    fn agent_flags_accept_positive_values_and_reject_unknown_or_missing_values() {
        let config = parse_options(
            [
                "--max-turns",
                "2",
                "--max-tool-calls",
                "3",
                "--max-tool-output-bytes",
                "4",
                "--max-response-tokens",
                "5",
                "--timeout-seconds",
                "6",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();
        assert_eq!(config.max_turns, 2);
        assert_eq!(config.max_tool_calls, 3);
        assert_eq!(config.max_tool_output_bytes, 4);
        assert_eq!(config.max_response_tokens, 5);
        assert_eq!(config.timeout_seconds, 6);
        assert!(
            parse_options(["--unknown", "1"].into_iter().map(String::from))
                .unwrap_err()
                .contains("usage: bo agent")
        );
        assert!(parse_options(["--max-turns"].into_iter().map(String::from))
            .unwrap_err()
            .contains("missing value for --max-turns"));
        assert!(
            parse_options(["--max-turns", "zero"].into_iter().map(String::from))
                .unwrap_err()
                .contains("positive integer")
        );
    }

    fn response(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    fn request_body(stream: &mut TcpStream) -> Value {
        let mut headers = Vec::new();
        let mut byte = [0; 1];
        while !headers.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            headers.push(byte[0]);
        }
        let headers_text = String::from_utf8(headers.clone()).unwrap();
        let length = headers_text
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
            })
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn fake_deepseek_tool_loop_preserves_tool_messages_and_replaces_summary() {
        let root = temp_root("loop");
        let target = root.join("notes");
        fs::write(target.join("article.md"), "# Article\n\nThe source fact.\n").unwrap();
        fs::create_dir(target.join("summaries")).unwrap();
        fs::write(target.join("summaries/article.md"), "old summary").unwrap();
        write_state(
            &target,
            &crate::State {
                raw: vec![crate::RawRecord {
                    filename: "article.md".to_string(),
                    url: "https://example.test/article".to_string(),
                    written_at: 1,
                }],
                summaries: vec![crate::SummaryRecord {
                    filename: "article.md".to_string(),
                    source_key: "https://example.test/article".to_string(),
                    derived_from: "article.md".to_string(),
                    created_at: 2,
                    updated_at: 3,
                }],
            },
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/chat/completions", listener.local_addr().unwrap());
        let (done_tx, done_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let first = {
                let (mut stream, _) = listener.accept().unwrap();
                let first = request_body(&mut stream);
                response(
                    &mut stream,
                    &serde_json::to_string(&json!({"choices":[{"message":{
                        "role":"assistant", "content":null, "tool_calls":[{"id":"state-1","type":"function","function":{"name":"read_state","arguments":"{}"}}]
                    }}]}))
                        .unwrap(),
                );
                first
            };
            assert_eq!(first["model"], MODEL);
            assert_eq!(first["thinking"]["type"], "disabled");

            let messages = {
                let (mut stream, _) = listener.accept().unwrap();
                let second = request_body(&mut stream);
                let messages = second["messages"].as_array().unwrap().clone();
                response(
                    &mut stream,
                    &serde_json::to_string(&json!({"choices":[{"message":{
                        "role":"assistant", "content":null, "tool_calls":[{"id":"read-1","type":"function","function":{"name":"read_summary","arguments":r###"{"source_key":"https://example.test/article"}"###}}]
                    }}]}))
                        .unwrap(),
                );
                messages
            };
            assert_eq!(messages[2]["role"], "assistant");
            assert_eq!(messages[2]["tool_calls"][0]["id"], "state-1");
            assert_eq!(messages[3]["role"], "tool");
            assert_eq!(messages[3]["tool_call_id"], "state-1");

            let messages = {
                let (mut stream, _) = listener.accept().unwrap();
                let third = request_body(&mut stream);
                response(
                    &mut stream,
                    &serde_json::to_string(&json!({"choices":[{"message":{
                        "role":"assistant", "content":null, "tool_calls":[{"id":"write-1","type":"function","function":{"name":"write_summary","arguments":r###"{"source_key":"https://example.test/article","markdown":"# Summary\n\nThe source fact.\n"}"###}}]
                    }}]}))
                        .unwrap(),
                );
                third["messages"].as_array().unwrap().clone()
            };
            assert_eq!(messages[4]["role"], "assistant");
            assert_eq!(messages[4]["tool_calls"][0]["id"], "read-1");
            assert_eq!(messages[5]["tool_call_id"], "read-1");
            let messages = {
                let (mut stream, _) = listener.accept().unwrap();
                let fourth = request_body(&mut stream);
                response(
                    &mut stream,
                    r#"{"choices":[{"message":{"role":"assistant","content":"done"}}]}"#,
                );
                fourth["messages"].as_array().unwrap().clone()
            };
            assert_eq!(messages[6]["role"], "assistant");
            assert_eq!(messages[6]["tool_calls"][0]["id"], "write-1");
            assert_eq!(messages[7]["tool_call_id"], "write-1");
            done_tx.send(()).unwrap();
        });

        let config = Config {
            max_turns: 4,
            max_tool_calls: 3,
            ..Config::default()
        };
        assert_eq!(
            run_agent(&root, &target, "test-key", &endpoint, &config).unwrap(),
            1
        );
        assert_eq!(
            fs::read_to_string(target.join("summaries/article.md")).unwrap(),
            "# Summary\n\nThe source fact.\n"
        );
        done_rx.recv().unwrap();
        thread.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn max_turns_stops_after_preserving_completed_summaries() {
        let root = temp_root("budget");
        let target = root.join("notes");
        fs::write(target.join("article.md"), "fact\n").unwrap();
        write_state(
            &target,
            &crate::State {
                raw: vec![crate::RawRecord {
                    filename: "article.md".to_string(),
                    url: "raw:article.md".to_string(),
                    written_at: 0,
                }],
                summaries: vec![],
            },
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/chat/completions", listener.local_addr().unwrap());
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = request_body(&mut stream);
            response(
                &mut stream,
                &serde_json::to_string(&json!({"choices":[{"message":{
                    "role":"assistant", "content":null, "tool_calls":[
                        {"id":"state-1","type":"function","function":{"name":"read_state","arguments":"{}"}},
                        {"id":"write-1","type":"function","function":{"name":"write_summary","arguments":r###"{"source_key":"raw:article.md","markdown":"# Summary\n"}"###}}
                    ]
                }}]}))
                    .unwrap(),
            );
        });
        let config = Config {
            max_turns: 1,
            max_tool_calls: 2,
            ..Config::default()
        };
        let error = run_agent(&root, &target, "test-key", &endpoint, &config).unwrap_err();
        assert!(error.contains("max turns reached"));
        assert_eq!(
            fs::read_to_string(target.join("summaries/article.md")).unwrap(),
            "# Summary\n"
        );
        thread.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_groups_use_latest_snapshot_and_raw_fallbacks() {
        let root = temp_root("sources");
        let target = root.join("notes");
        fs::write(target.join("old.md"), "old\n").unwrap();
        fs::write(target.join("new.md"), "new\n").unwrap();
        fs::write(target.join("other.md"), "other\n").unwrap();
        fs::write(target.join("manual.md"), "manual\n").unwrap();
        write_state(
            &target,
            &crate::State {
                raw: vec![
                    crate::RawRecord {
                        filename: "old.md".into(),
                        url: "https://example.test/a".into(),
                        written_at: 1,
                    },
                    crate::RawRecord {
                        filename: "new.md".into(),
                        url: "https://example.test/a".into(),
                        written_at: 2,
                    },
                    crate::RawRecord {
                        filename: "other.md".into(),
                        url: "https://example.test/b".into(),
                        written_at: 1,
                    },
                ],
                summaries: vec![],
            },
        )
        .unwrap();
        let context = context(&root);
        assert_eq!(context.sources.len(), 3);
        assert_eq!(
            context.sources["https://example.test/a"].latest_filename,
            "new.md"
        );
        assert_eq!(
            context.sources["https://example.test/b"].latest_filename,
            "other.md"
        );
        assert_eq!(
            context.sources["raw:manual.md"].latest_filename,
            "manual.md"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_upsert_preserves_created_at_and_tracks_latest_raw() {
        let root = temp_root("upsert");
        let target = root.join("notes");
        fs::write(target.join("old.md"), "old\n").unwrap();
        fs::write(target.join("new.md"), "new\n").unwrap();
        fs::create_dir(target.join("summaries")).unwrap();
        fs::write(target.join("summaries/summary.md"), "old summary\n").unwrap();
        write_state(
            &target,
            &crate::State {
                raw: vec![
                    crate::RawRecord {
                        filename: "old.md".into(),
                        url: "https://example.test/a".into(),
                        written_at: 1,
                    },
                    crate::RawRecord {
                        filename: "new.md".into(),
                        url: "https://example.test/a".into(),
                        written_at: 2,
                    },
                ],
                summaries: vec![crate::SummaryRecord {
                    filename: "summary.md".into(),
                    source_key: "https://example.test/a".into(),
                    derived_from: "old.md".into(),
                    created_at: 10,
                    updated_at: 11,
                }],
            },
        )
        .unwrap();
        let mut context = context(&root);
        context.state_read = true;
        write_summary(&mut context, "https://example.test/a", "new summary\n").unwrap();
        let state = load_state(&target).unwrap();
        assert_eq!(state.summaries.len(), 1);
        assert_eq!(state.summaries[0].filename, "summary.md");
        assert_eq!(state.summaries[0].derived_from, "new.md");
        assert_eq!(state.summaries[0].created_at, 10);
        assert!(state.summaries[0].updated_at > 11);
        assert_eq!(
            fs::read_to_string(target.join("summaries/summary.md")).unwrap(),
            "new summary\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_tools_reject_shell_escape_and_non_raw_files() {
        let root = temp_root("boundary");
        let target = root.join("notes");
        fs::write(target.join("article.md"), "fact\n").unwrap();
        fs::write(target.join("secret.txt"), "secret\n").unwrap();
        let mut context = context(&root);
        let bo_root = context.root.clone();
        assert!(execute_bash(&mut context, "cat secret.txt").is_err());
        assert!(execute_bash(&mut context, "cat ../notes/secret.txt").is_err());
        assert!(execute_bash(&mut context, "cat article.md | head").is_err());
        assert!(execute_bash(&mut context, "ls /tmp").is_err());
        assert!(execute_bash(&mut context, "grep fact /tmp").is_err());
        assert!(execute_bash(&mut context, "grep fact ../..").is_err());
        assert!(execute_bash(&mut context, "grep fact")
            .unwrap()
            .contains("article.md:1:fact"));

        assert!(execute_bash(&mut context, "cd ..").is_ok());
        assert_eq!(context.cwd, bo_root);
        assert!(execute_bash(&mut context, "cat notes/article.md")
            .unwrap()
            .contains("fact"));
        assert!(execute_bash(&mut context, "cd /tmp").is_err());
        assert_eq!(context.cwd, bo_root);

        assert!(write_summary(&mut context, "../outside.md", "no").is_err());
        assert!(write_summary(&mut context, "raw:article.md", " \n").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn raw_symlinks_that_escape_the_bo_root_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink");
        let outside = env::temp_dir().join(format!("bo-agent-outside-{}", std::process::id()));
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.md"), "secret\n").unwrap();
        symlink(outside.join("secret.md"), root.join("notes/escape.md")).unwrap();

        assert!(discover_documents(&root.canonicalize().unwrap(), &root.join("notes")).is_err());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
