package application

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
)

func synthTools(contextState *agentContext, summarized map[string]bool) []agent.Tool {
	objectParameters := func(properties map[string]any, required []string) map[string]any {
		return map[string]any{"type": "object", "properties": properties, "required": required, "additionalProperties": false}
	}
	execute := func(_ context.Context, call agent.ToolCall) (string, error) {
		return executeToolCall(contextState, call, summarized)
	}
	return []agent.Tool{
		{Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "bash", Description: "Run one bounded facade command: ls [path], cd path, cat raw.md, or grep literal [path]. This is not a shell.", Parameters: objectParameters(map[string]any{"command": map[string]any{"type": "string"}}, []string{"command"})}}, Execute: execute},
		{Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "read_state", Description: "Read the authoritative state for the target directory.", Parameters: objectParameters(map[string]any{}, []string{})}}, Execute: execute},
		{Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "read_summary", Description: "Read the existing Markdown summary for one source identity.", Parameters: objectParameters(map[string]any{"source_key": map[string]any{"type": "string"}}, []string{"source_key"})}}, Execute: execute},
		{Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "write_summary", Description: "Write or replace the Markdown summary for one source identity using its newest raw snapshot.", Parameters: objectParameters(map[string]any{"source_key": map[string]any{"type": "string"}, "markdown": map[string]any{"type": "string"}}, []string{"source_key", "markdown"})}}, Execute: execute},
	}
}

type agentSource struct {
	LatestFilename   string
	LatestWrittenAt  uint64
	LatestStateIndex int
}

type agentContext struct {
	ctx            context.Context
	root, target   string
	storage        Storage
	documents      map[string]string
	sources        map[string]agentSource
	state          domain.State
	generation     Generation
	cwd            string
	maxOutputBytes int
	stateRead      bool
}

func systemPrompt(context *agentContext, documentNames []string) string {
	return fmt.Sprintf("You are bo's bounded document-summary agent for %s. Call read_state before any other tool. The state object is authoritative: each exact raw URL is one source identity, and raw:filename identifies a Markdown file with no state record. For each source identity, use the newest raw snapshot by written_at as evidence. If a summary record exists, call read_summary with its source_key before replacing it. Never modify or delete raw files. Summarize only facts present in each source. Preserve epistemic status: clearly attribute author experience or measurements (for example, 'the author reports'), recommendations or opinions (for example, 'the article recommends'), and predictions or forecasts (for example, 'the author predicts'); do not present those as general facts. Preserve qualifications and uncertainty while staying concise. Write one concise Markdown summary per source identity with write_summary using source_key, not a raw filename. Use only the provided bounded tools; bash is a strict facade, not a shell. The raw filenames discovered at start are: %s.", context.target, strings.Join(documentNames, ", "))
}

func DiscoverDocuments(root, target string) (map[string]string, error) {
	entries, err := os.ReadDir(target)
	if err != nil {
		return nil, fmt.Errorf("reading %s failed: %w", target, err)
	}
	documents := map[string]string{}
	for _, entry := range entries {
		name := entry.Name()
		if !strings.EqualFold(filepath.Ext(name), ".md") {
			continue
		}
		path := filepath.Join(target, name)
		resolved, err := filepath.EvalSymlinks(path)
		if err != nil {
			return nil, fmt.Errorf("resolving %s failed: %w", path, err)
		}
		if err := ensureInside(resolved, root); err != nil {
			return nil, err
		}
		if err := ensureInside(resolved, target); err != nil {
			return nil, err
		}
		info, err := os.Stat(resolved)
		if err != nil {
			return nil, fmt.Errorf("reading %s failed: %w", resolved, err)
		}
		if info.Mode().IsRegular() {
			documents[name] = resolved
		}
	}
	return documents, nil
}

func sourceGroups(documents map[string]string, state domain.State) map[string]agentSource {
	names := make([]string, 0, len(documents))
	for name := range documents {
		names = append(names, name)
	}
	sort.Strings(names)
	sources := map[string]agentSource{}
	for _, filename := range names {
		sourceKey, writtenAt, stateIndex := "raw:"+filename, uint64(0), 0
		for index, record := range state.Raw {
			if record.Filename == filename && (stateIndex == 0 && writtenAt == 0 || record.WrittenAt > writtenAt || record.WrittenAt == writtenAt && index > stateIndex) {
				sourceKey, writtenAt, stateIndex = record.URL, record.WrittenAt, index
			}
		}
		current, ok := sources[sourceKey]
		if !ok || writtenAt > current.LatestWrittenAt || writtenAt == current.LatestWrittenAt && stateIndex > current.LatestStateIndex {
			sources[sourceKey] = agentSource{LatestFilename: filename, LatestWrittenAt: writtenAt, LatestStateIndex: stateIndex}
		}
	}
	return sources
}

func executeToolCall(context *agentContext, call agent.ToolCall, summarized map[string]bool) (string, error) {
	name := call.Function.Name
	if name != "read_state" && !context.stateRead {
		return "", fmt.Errorf("read_state must be called before other tools")
	}
	var arguments map[string]json.RawMessage
	if err := json.Unmarshal([]byte(call.Function.Arguments), &arguments); err != nil {
		return "", fmt.Errorf("%s arguments are malformed JSON: %v", name, err)
	}
	if arguments == nil {
		return "", fmt.Errorf("%s arguments must be a JSON object", name)
	}
	switch name {
	case "read_state":
		if len(arguments) != 0 {
			return "", fmt.Errorf("read_state arguments must be empty")
		}
		context.stateRead = true
		data, err := json.MarshalIndent(context.state, "", "  ")
		if err != nil {
			return "", fmt.Errorf("serializing state failed: %v", err)
		}
		return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
	case "bash":
		if len(arguments) != 1 {
			return "", fmt.Errorf("bash arguments must contain only command")
		}
		command, err := stringArgument(arguments, "command")
		if err != nil {
			return "", fmt.Errorf("bash.command must be a string")
		}
		return executeBash(context, command)
	case "read_summary":
		if len(arguments) != 1 {
			return "", fmt.Errorf("read_summary arguments must contain only source_key")
		}
		sourceKey, err := stringArgument(arguments, "source_key")
		if err != nil {
			return "", fmt.Errorf("read_summary.source_key must be a string")
		}
		return readSummary(context, sourceKey)
	case "write_summary":
		if len(arguments) != 2 {
			return "", fmt.Errorf("write_summary arguments must contain only source_key and markdown")
		}
		sourceKey, err := stringArgument(arguments, "source_key")
		if err != nil {
			return "", fmt.Errorf("write_summary.source_key must be a string")
		}
		markdown, err := stringArgument(arguments, "markdown")
		if err != nil {
			return "", fmt.Errorf("write_summary.markdown must be a string")
		}
		if err := writeSummary(context, sourceKey, markdown); err != nil {
			return "", err
		}
		summarized[sourceKey] = true
		return "summary written: " + sourceKey, nil
	default:
		return "", fmt.Errorf("unsupported tool: %s", name)
	}
}

func stringArgument(arguments map[string]json.RawMessage, name string) (string, error) {
	var value string
	if raw, ok := arguments[name]; !ok || json.Unmarshal(raw, &value) != nil {
		return "", fmt.Errorf("not a string")
	}
	return value, nil
}

func executeBash(context *agentContext, command string) (string, error) {
	if command == "" || strings.ContainsAny(command, "|&;><$`(){}\n\r") {
		return "", fmt.Errorf("unsupported shell syntax")
	}
	parts := strings.Fields(command)
	if len(parts) == 0 {
		return "", fmt.Errorf("unsupported shell syntax")
	}
	switch {
	case len(parts) == 1 && parts[0] == "ls":
		return listDirectory(context, context.cwd)
	case len(parts) == 2 && parts[0] == "ls":
		path, err := context.resolve(parts[1])
		if err != nil {
			return "", err
		}
		return listDirectory(context, path)
	case len(parts) == 2 && parts[0] == "cd":
		path, err := context.resolve(parts[1])
		if err != nil {
			return "", err
		}
		info, err := os.Stat(path)
		if err != nil || !info.IsDir() {
			return "", fmt.Errorf("not a directory: %s", path)
		}
		context.cwd = path
		return "directory: " + path, nil
	case len(parts) == 2 && parts[0] == "cat":
		path, err := context.resolve(parts[1])
		if err != nil {
			return "", err
		}
		for _, raw := range context.documents {
			if raw == path {
				return readBounded(path, context.maxOutputBytes)
			}
		}
		return "", fmt.Errorf("cat is limited to raw Markdown documents")
	case len(parts) == 2 && parts[0] == "grep":
		return grep(context, parts[1], "")
	case len(parts) == 3 && parts[0] == "grep":
		return grep(context, parts[1], parts[2])
	default:
		return "", fmt.Errorf("unsupported command grammar")
	}
}

func (context *agentContext) resolve(input string) (string, error) {
	if input == "" || strings.ContainsRune(input, 0) {
		return "", fmt.Errorf("path is empty or contains NUL")
	}
	path := input
	switch {
	case input == "~/.bo":
		path = context.root
	case strings.HasPrefix(input, "~/.bo/"):
		path = filepath.Join(context.root, strings.TrimPrefix(input, "~/.bo/"))
	case !filepath.IsAbs(input):
		path = filepath.Join(context.cwd, input)
	}
	path, err := filepath.EvalSymlinks(filepath.Clean(path))
	if err != nil {
		return "", fmt.Errorf("resolving %s failed: %v", input, err)
	}
	if err := ensureInside(path, context.root); err != nil {
		return "", err
	}
	return path, nil
}

func listDirectory(context *agentContext, path string) (string, error) {
	info, err := os.Stat(path)
	if err != nil {
		return "", fmt.Errorf("not a file or directory: %s", path)
	}
	if info.Mode().IsRegular() {
		return filepath.Base(path) + "\n", nil
	}
	if !info.IsDir() {
		return "", fmt.Errorf("not a file or directory: %s", path)
	}
	entries, err := os.ReadDir(path)
	if err != nil {
		return "", fmt.Errorf("listing %s failed: %v", path, err)
	}
	names := make([]string, 0, len(entries))
	for _, entry := range entries {
		resolved, err := filepath.EvalSymlinks(filepath.Join(path, entry.Name()))
		if err != nil {
			return "", fmt.Errorf("resolving %s failed: %v", filepath.Join(path, entry.Name()), err)
		}
		if err := ensureInside(resolved, context.root); err != nil {
			return "", err
		}
		names = append(names, entry.Name())
	}
	sort.Strings(names)
	return agent.BoundedOutput(strings.Join(names, "\n")+"\n", context.maxOutputBytes), nil
}

func grep(context *agentContext, pattern, pathInput string) (string, error) {
	paths := make([]struct{ name, path string }, 0)
	if pathInput == "" {
		names := make([]string, 0, len(context.documents))
		for name := range context.documents {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			paths = append(paths, struct{ name, path string }{name, context.documents[name]})
		}
	} else {
		path, err := context.resolve(pathInput)
		if err != nil {
			return "", err
		}
		info, err := os.Stat(path)
		if err != nil {
			return "", fmt.Errorf("not a file or directory: %s", path)
		}
		for name, raw := range context.documents {
			if info.IsDir() {
				if err := ensureInside(raw, path); err == nil {
					paths = append(paths, struct{ name, path string }{name, raw})
				}
			} else if raw == path {
				paths = append(paths, struct{ name, path string }{name, raw})
			}
		}
		sort.Slice(paths, func(i, j int) bool { return paths[i].name < paths[j].name })
	}
	var output strings.Builder
	for _, candidate := range paths {
		file, err := os.Open(candidate.path)
		if err != nil {
			return "", fmt.Errorf("reading %s failed: %v", candidate.path, err)
		}
		reader := bufio.NewReader(file)
		lineNumber := 0
		for {
			line, readErr := reader.ReadString('\n')
			if len(line) == 0 && readErr == io.EOF {
				break
			}
			lineNumber++
			line = strings.TrimSuffix(strings.TrimSuffix(line, "\n"), "\r")
			if strings.Contains(line, pattern) {
				fmt.Fprintf(&output, "%s:%d:%s\n", candidate.name, lineNumber, line)
			}
			if readErr == io.EOF {
				break
			}
			if readErr != nil {
				file.Close()
				return "", fmt.Errorf("reading %s failed: %v", candidate.path, readErr)
			}
		}
		file.Close()
	}
	if output.Len() == 0 {
		output.WriteString("(no matches)\n")
	}
	return agent.BoundedOutput(output.String(), context.maxOutputBytes), nil
}

func readBounded(path string, limit int) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", fmt.Errorf("reading %s failed: %v", path, err)
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, int64(limit)+4))
	if err != nil {
		return "", fmt.Errorf("reading %s failed: %v", path, err)
	}
	return agent.BoundedOutput(string(data), limit), nil
}

func readSummary(context *agentContext, sourceKey string) (string, error) {
	var record *domain.SummaryRecord
	for index := range context.state.Summaries {
		if context.state.Summaries[index].SourceKey == sourceKey {
			record = &context.state.Summaries[index]
			break
		}
	}
	if record == nil {
		return "", fmt.Errorf("no summary exists for source: %s", sourceKey)
	}
	data, err := context.storage.ReadDocument(context.ctx, domain.SummaryRef(record.Filename))
	if err != nil {
		return "", err
	}
	return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
}

func writeSummary(context *agentContext, sourceKey, markdown string) error {
	source, ok := context.sources[sourceKey]
	if !ok {
		return fmt.Errorf("unknown source: %s", sourceKey)
	}
	if strings.TrimSpace(markdown) == "" {
		return fmt.Errorf("summary Markdown must be non-empty")
	}
	if len(markdown) > context.maxOutputBytes {
		return fmt.Errorf("summary exceeds max tool output bytes (%d)", context.maxOutputBytes)
	}
	var existing *domain.SummaryRecord
	for index := range context.state.Summaries {
		if context.state.Summaries[index].SourceKey == sourceKey {
			existing = &context.state.Summaries[index]
			break
		}
	}
	filename := source.LatestFilename
	createdAt := uint64(0)
	if existing != nil {
		filename = existing.Filename
		createdAt = existing.CreatedAt
	}
	if err := context.storage.ReplaceSummary(context.ctx, domain.SummaryRef(filename), []byte(markdown)); err != nil {
		return err
	}
	now, err := nowNanos()
	if err != nil {
		return err
	}
	updatedAt := now
	if existing != nil {
		updatedAt = existing.UpdatedAt + 1
		if now > updatedAt {
			updatedAt = now
		}
	}
	record := domain.SummaryRecord{Filename: filename, SourceKey: sourceKey, DerivedFrom: source.LatestFilename, CreatedAt: createdAt, UpdatedAt: updatedAt}
	if existing == nil {
		record.CreatedAt = now
	}
	next := context.state
	next.Summaries = append([]domain.SummaryRecord{}, context.state.Summaries...)
	replaced := false
	for index := range next.Summaries {
		if next.Summaries[index].SourceKey == sourceKey {
			next.Summaries[index] = record
			replaced = true
			break
		}
	}
	if !replaced {
		next.Summaries = append(next.Summaries, record)
	}
	generation, err := context.storage.PublishState(context.ctx, next, context.generation)
	if err != nil {
		return err
	}
	context.state, context.generation = next, generation
	return nil
}

func ensureInside(path, root string) error {
	relative, err := filepath.Rel(root, path)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) || filepath.IsAbs(relative) {
		return fmt.Errorf("path escapes %s: %s", root, path)
	}
	return nil
}

func nowNanos() (uint64, error) {
	now := time.Now().UnixNano()
	if now < 0 {
		return 0, fmt.Errorf("clock returned a time before Unix epoch")
	}
	return uint64(now), nil
}
