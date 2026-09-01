#!/usr/bin/env python3
"""Score harness trials with a separate OpenAI-compatible judge."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import shutil
import socket
import sys
import tempfile
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path


SCHEMA_VERSION = 2
MODEL = "deepseek-v4-pro"
SUMMARY_PROMPT_VERSION = "summary-evaluator-v4"
DISTILL_PROMPT_VERSION = "distill-evaluator-v2"
DEFAULT_API_URL = "https://api.deepseek.com/chat/completions"
RUBRIC_PATH = Path(__file__).with_name("RUBRIC.md")
DISTILL_RUBRIC_PATH = Path(__file__).with_name("DISTILL_RUBRIC.md")
MAX_DOCUMENTS = 64
MAX_INPUT_BYTES = 512 * 1024
MAX_OUTPUT_TOKENS = 8192
MAX_TOTAL_OUTPUT_TOKENS = 128 * 1024
REQUEST_TIMEOUT_SECONDS = 60
MIN_INDIVIDUAL_SCORE = 4
MIN_MEAN_SCORE = 4.6
CRITERIA = ("faithfulness", "coverage", "usefulness", "page_cleanliness")
DISTILL_CRITERIA = (
    "faithfulness",
    "cross_source_integration",
    "usefulness",
    "structure",
    "source_attribution",
)
FAITHFULNESS_GROUPS = (
    "source_facts",
    "author_experience_measurements",
    "recommendations_opinions",
    "predictions_forecasts",
)
urlopen = None


class EvaluationError(RuntimeError):
    pass


def read_json(path: Path, description: str) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise EvaluationError(f"read {description}: {error}") from error


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_text(path: Path, description: str) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise EvaluationError(f"read {description}: {error}") from error


def safe_filename(value: object, label: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value or Path(value).name != value or Path(value).suffix.lower() != ".md":
        raise EvaluationError(f"{label} must be a Markdown file name")
    return value


def load_state(workspace: Path) -> dict:
    state_path = workspace / "state.json"
    if state_path.is_symlink() or not state_path.is_file():
        raise EvaluationError("state.json must be a regular file")
    state = read_json(state_path, "state.json")
    if not isinstance(state, dict) or not isinstance(state.get("sources"), list):
        raise EvaluationError("state.json must contain a sources array")
    return state


def snapshot_time(value: object) -> datetime:
    if not isinstance(value, str) or not value:
        raise EvaluationError("snapshot written_at must be an RFC 3339 timestamp")
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        timestamp = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise EvaluationError("snapshot written_at must be an RFC 3339 timestamp") from error
    if timestamp.tzinfo is None or timestamp.utcoffset() is None:
        raise EvaluationError("snapshot written_at must include a timezone")
    return timestamp.astimezone(timezone.utc)


def latest_snapshot(source: dict) -> dict:
    snapshots = source.get("snapshots")
    if not isinstance(snapshots, list) or not snapshots:
        raise EvaluationError(f"source has no snapshots: {source.get('source_key')}")
    if any(not isinstance(snapshot, dict) for snapshot in snapshots):
        raise EvaluationError(f"source has an invalid snapshot: {source.get('source_key')}")
    return max(enumerate(snapshots), key=lambda item: (snapshot_time(item[1].get("written_at")), item[0]))[1]


def load_pairs(workspace: Path) -> tuple[list[dict], list[dict]]:
    state = load_state(workspace)
    raw_dir = workspace
    summary_dir = workspace / "summaries"
    if summary_dir.is_symlink():
        raise EvaluationError("summaries directory is a symlink")
    pairs = []
    missing = []
    source_keys = set()
    for source in state["sources"]:
        if not isinstance(source, dict) or not isinstance(source.get("source_key"), str) or not source["source_key"]:
            raise EvaluationError("state contains an invalid source record")
        source_key = source["source_key"]
        if source_key in source_keys:
            raise EvaluationError(f"state contains duplicate source: {source_key}")
        source_keys.add(source_key)
        latest = latest_snapshot(source)
        raw_filename = safe_filename(latest.get("filename"), "snapshot filename")
        raw_path = raw_dir / raw_filename
        summary = source.get("summary")
        if not isinstance(summary, dict):
            missing.append({"source_key": source_key, "raw_filename": raw_filename, "reason": "missing summary record"})
            continue
        summary_filename = safe_filename(summary.get("filename"), "summary filename")
        if summary.get("derived_from") != raw_filename:
            missing.append({"source_key": source_key, "raw_filename": raw_filename, "summary_filename": summary_filename, "reason": "stale summary record"})
            continue
        summary_path = summary_dir / summary_filename
        if raw_path.is_symlink() or not raw_path.is_file():
            missing.append({"source_key": source_key, "raw_filename": raw_filename, "summary_filename": summary_filename, "reason": "missing raw document"})
            continue
        if summary_path.is_symlink() or not summary_path.is_file():
            missing.append({"source_key": source_key, "raw_filename": raw_filename, "summary_filename": summary_filename, "reason": "missing summary file"})
            continue
        pairs.append(
            {
                "source_key": source_key,
                "raw_filename": raw_filename,
                "summary_filename": summary_filename,
                "raw": read_text(raw_path, f"raw document {raw_filename}"),
                "summary": read_text(summary_path, f"summary {summary_filename}"),
            }
        )
    return pairs, missing


def load_distill_documents(workspace: Path) -> list[dict]:
    state = load_state(workspace)
    if (workspace / "distillations").is_symlink() or (workspace / "summaries").is_symlink():
        raise EvaluationError("workspace document directory is a symlink")
    raw_owners = {}
    summary_owners = {}
    current_raw = {}
    current_summaries = {}
    source_keys = set()
    for source in state["sources"]:
        if not isinstance(source, dict):
            raise EvaluationError("state contains an invalid source record")
        source_key = source.get("source_key")
        if not isinstance(source_key, str) or not source_key:
            raise EvaluationError("state contains an invalid source record")
        if source_key in source_keys:
            raise EvaluationError(f"state contains duplicate source: {source_key}")
        source_keys.add(source_key)
        latest = latest_snapshot(source)
        for snapshot in source["snapshots"]:
            filename = safe_filename(snapshot.get("filename"), "snapshot filename")
            if filename in raw_owners:
                raise EvaluationError(f"duplicate raw document: {filename}")
            raw_owners[filename] = source_key
        current_raw[source_key] = safe_filename(latest.get("filename"), "snapshot filename")
        summary = source.get("summary")
        if isinstance(summary, dict):
            filename = safe_filename(summary.get("filename"), "summary filename")
            if filename in summary_owners:
                raise EvaluationError(f"duplicate summary document: {filename}")
            summary_owners[filename] = source_key
            if summary.get("derived_from") == current_raw[source_key]:
                current_summaries[filename] = source_key
    records = state.get("distillation_documents", [])
    if not isinstance(records, list):
        raise EvaluationError("state distillation_documents must be an array")
    if len(records) > MAX_DOCUMENTS:
        raise EvaluationError(f"document limit exceeded: {len(records)} > {MAX_DOCUMENTS}")
    result = []
    for record in records:
        if not isinstance(record, dict):
            raise EvaluationError("distillation record must be an object")
        filename = safe_filename(record.get("filename"), "distillation filename")
        artifact_path = workspace / "distillations" / filename
        if artifact_path.is_symlink() or not artifact_path.is_file():
            raise EvaluationError(f"missing distillation document: {filename}")
        inputs = record.get("derived_from")
        if not isinstance(inputs, list) or not inputs:
            raise EvaluationError(f"distillation has no provenance: {filename}")
        evidence = []
        source_keys = set()
        for item in inputs:
            if not isinstance(item, dict) or item.get("kind") not in ("raw", "summary"):
                raise EvaluationError(f"invalid provenance in {filename}")
            source_key = item.get("source_key")
            input_filename = safe_filename(item.get("filename"), "provenance filename")
            owners = raw_owners if item["kind"] == "raw" else summary_owners
            if owners.get(input_filename) != source_key:
                raise EvaluationError(f"provenance owner mismatch in {filename}")
            if item["kind"] == "raw" and current_raw.get(source_key) != input_filename:
                raise EvaluationError(f"provenance is not current raw input in {filename}")
            if item["kind"] == "summary" and current_summaries.get(input_filename) != source_key:
                raise EvaluationError(f"provenance is not current summary input in {filename}")
            digest = item.get("content_digest")
            if not isinstance(digest, str) or len(digest) != 64:
                raise EvaluationError(f"invalid provenance digest in {filename}")
            directory = workspace / "summaries" if item["kind"] == "summary" else workspace
            path = directory / input_filename
            if path.is_symlink() or not path.is_file():
                raise EvaluationError(f"missing provenance document: {input_filename}")
            content = path.read_bytes()
            if hashlib.sha256(content).hexdigest() != digest.lower():
                raise EvaluationError(f"provenance digest changed: {input_filename}")
            evidence.append(
                {
                    "source_key": source_key,
                    "kind": item["kind"],
                    "filename": input_filename,
                    "content": content.decode("utf-8"),
                }
            )
            source_keys.add(source_key)
        if len(source_keys) < 2:
            raise EvaluationError(f"distillation uses fewer than two source identities: {filename}")
        result.append({"filename": filename, "artifact": read_text(artifact_path, f"distillation {filename}"), "inputs": evidence})
    return result


def validate_score(value: object, criteria: tuple[str, ...], faithfulness_groups: bool = False) -> dict:
    if not isinstance(value, dict):
        raise EvaluationError("model output must be a JSON object")
    normalized = {}
    for criterion in criteria:
        section = value.get(criterion)
        if not isinstance(section, dict):
            raise EvaluationError(f"{criterion} must be an object")
        score = section.get("score")
        if not isinstance(score, int) or isinstance(score, bool) or not 1 <= score <= 5:
            raise EvaluationError(f"{criterion}.score must be an integer from 1 to 5")
        evidence = section.get("evidence")
        if faithfulness_groups and criterion == "faithfulness":
            if not isinstance(evidence, dict):
                raise EvaluationError("faithfulness.evidence must contain four groups")
            grouped = {}
            for group in FAITHFULNESS_GROUPS:
                item = evidence.get(group)
                if not isinstance(item, str) or not item.strip():
                    raise EvaluationError(f"faithfulness.evidence.{group} must be non-empty")
                grouped[group] = item
            evidence = grouped
        elif not isinstance(evidence, str) or not evidence.strip():
            raise EvaluationError(f"{criterion}.evidence must be non-empty")
        normalized[criterion] = {"score": score, "evidence": evidence}
    return normalized


def summary_prompt(rubric: str, pair: dict) -> str:
    return f"""You are evaluating a Markdown summary against its raw source.
Use only the supplied raw document and summary. Score every criterion from 1 to
5. Justify each score with concrete source-grounded evidence. For faithfulness,
separate source facts, author experience or measurements, recommendations or
opinions, and predictions or forecasts; use \"not present\" when absent.
Return JSON only in this shape:
{{
  \"faithfulness\": {{\"score\": 1, \"evidence\": {{\"source_facts\": \"...\", \"author_experience_measurements\": \"...\", \"recommendations_opinions\": \"...\", \"predictions_forecasts\": \"...\"}}}},
  \"coverage\": {{\"score\": 1, \"evidence\": \"...\"}},
  \"usefulness\": {{\"score\": 1, \"evidence\": \"...\"}},
  \"page_cleanliness\": {{\"score\": 1, \"evidence\": \"...\"}}
}}

Rubric:
{rubric}

Source identity: {pair['source_key']}
Raw document ({pair['raw_filename']}):
---
{pair['raw']}
---
Summary ({pair['summary_filename']}):
---
{pair['summary']}
---
"""


def distill_prompt(rubric: str, document: dict) -> str:
    inputs = "\n\n".join(
        f"Source identity: {item['source_key']}\n{item['kind']} document ({item['filename']}):\n---\n{item['content']}\n---"
        for item in document["inputs"]
    )
    return f"""You are evaluating a cross-source Markdown distillation.
Use only the supplied provenance documents and artifact. Score every criterion
from 1 to 5 and justify each score with concrete source-grounded evidence.
Return JSON only in this shape:
{{
  \"faithfulness\": {{\"score\": 1, \"evidence\": \"...\"}},
  \"cross_source_integration\": {{\"score\": 1, \"evidence\": \"...\"}},
  \"usefulness\": {{\"score\": 1, \"evidence\": \"...\"}},
  \"structure\": {{\"score\": 1, \"evidence\": \"...\"}},
  \"source_attribution\": {{\"score\": 1, \"evidence\": \"...\"}}
}}

Rubric:
{rubric}

Distillation artifact ({document['filename']}):
---
{document['artifact']}
---

Provenance documents:
{inputs}
"""


def parse_model_json(content: object) -> object:
    if isinstance(content, dict):
        return content
    if not isinstance(content, str):
        raise EvaluationError("model output must be a JSON object or string")
    value = content.strip()
    if value.startswith("```") and value.endswith("```"):
        lines = value.splitlines()
        if len(lines) < 3 or not lines[-1].strip().startswith("```"):
            raise EvaluationError("model output has an invalid JSON code fence")
        value = "\n".join(lines[1:-1]).strip()
    try:
        return json.loads(value)
    except json.JSONDecodeError as error:
        raise EvaluationError(f"model output is not valid JSON: {error}") from error


def request_evaluation(endpoint: str, api_key: str, prompt: str) -> tuple[object, int]:
    body = json.dumps(
        {
            "model": os.environ.get("BO_EVAL_MODEL", MODEL),
            "messages": [{"role": "user", "content": prompt}],
            "response_format": {"type": "json_object"},
            "stream": False,
            "max_tokens": MAX_OUTPUT_TOKENS,
        },
        ensure_ascii=False,
    ).encode("utf-8")
    last_error: Exception | None = None
    for attempt in range(3):
        try:
            request = urllib.request.Request(
                endpoint,
                data=body,
                headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
                method="POST",
            )
            opener = urlopen or urllib.request.urlopen
            response = opener(request, timeout=REQUEST_TIMEOUT_SECONDS)
            try:
                status = getattr(response, "status", None) or response.getcode()
                response_body = response.read()
            finally:
                close = getattr(response, "close", None)
                if close is not None:
                    close()
            if status is not None and not 200 <= status < 300:
                raise EvaluationError(f"API request failed with HTTP {status}")
            payload = json.loads(response_body.decode("utf-8"))
            if not isinstance(payload, dict):
                raise EvaluationError("API response must be an object")
            choices = payload.get("choices")
            if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
                raise EvaluationError("API response is missing choices[0]")
            choice = choices[0]
            if choice.get("finish_reason") in ("length", "max_tokens"):
                raise EvaluationError("API response was truncated at the output token limit")
            message = choice.get("message")
            if not isinstance(message, dict) or "content" not in message:
                raise EvaluationError("API response is missing choices[0].message.content")
            usage = payload.get("usage", {})
            tokens = usage.get("completion_tokens", 0) if isinstance(usage, dict) else 0
            if not isinstance(tokens, int) or isinstance(tokens, bool) or tokens < 0 or tokens > MAX_OUTPUT_TOKENS:
                raise EvaluationError("API response has invalid completion token usage")
            return parse_model_json(message["content"]), tokens
        except urllib.error.HTTPError as error:
            last_error = EvaluationError(f"API request failed with HTTP {error.code}")
            if error.code not in (408, 409, 429) and error.code < 500:
                break
        except (urllib.error.URLError, OSError, socket.timeout, TimeoutError, UnicodeError, json.JSONDecodeError, http.client.IncompleteRead) as error:
            last_error = EvaluationError(f"API request failed: {error}")
        except EvaluationError as error:
            last_error = error
            if "HTTP 5" not in str(error):
                break
        if attempt < 2:
            time.sleep(0.25 * (2**attempt))
    assert last_error is not None
    raise last_error


def score_documents(documents: list[dict], workflow: str, rubric: str, api_key: str, endpoint: str, jobs: int) -> tuple[list[dict | None], list[str], int]:
    criteria = CRITERIA if workflow == "summarize" else DISTILL_CRITERIA
    scored: list[dict | None] = [None] * len(documents)
    errors: list[str] = []

    def score_one(index: int, document: dict) -> tuple[int, dict, int]:
        content_size = len(document.get("raw", document.get("artifact", "")).encode("utf-8"))
        if workflow == "summarize":
            content_size += len(document["summary"].encode("utf-8"))
        else:
            content_size += sum(len(item["content"].encode("utf-8")) for item in document["inputs"])
        if content_size > MAX_INPUT_BYTES:
            raise EvaluationError(f"input limit exceeded for {document['filename' if workflow == 'distill' else 'raw_filename']}: {content_size} > {MAX_INPUT_BYTES}")
        prompt = summary_prompt(rubric, document) if workflow == "summarize" else distill_prompt(rubric, document)
        value, tokens = request_evaluation(endpoint, api_key, prompt)
        normalized = validate_score(value, criteria, faithfulness_groups=workflow == "summarize")
        return index, {**normalized, "output_tokens": tokens}, tokens

    worker_count = max(1, min(jobs, len(documents))) if documents else 1
    with ThreadPoolExecutor(max_workers=worker_count) as executor:
        futures = {
            executor.submit(score_one, index, document): index
            for index, document in enumerate(documents)
        }
        for future in as_completed(futures):
            index = futures[future]
            try:
                _, result, _ = future.result()
                scored[index] = result
            except Exception as error:
                document = documents[index]
                label = document.get("filename", document.get("raw_filename", f"document {index + 1}"))
                errors.append(f"{label}: {error}")
    tokens = sum(int(value.get("output_tokens", 0)) for value in scored if value is not None)
    if tokens > MAX_TOTAL_OUTPUT_TOKENS:
        errors.append(f"total output token budget exceeded: {tokens} > {MAX_TOTAL_OUTPUT_TOKENS}")
    return scored, errors, tokens


def quality_gate(documents: list[dict], criteria: tuple[str, ...]) -> dict:
    if not documents:
        return {"status": "skipped", "min_individual": None, "means": {}, "failures": []}
    values = {criterion: [document[criterion]["score"] for document in documents] for criterion in criteria}
    raw_means = {criterion: sum(scores) / len(scores) for criterion, scores in values.items()}
    means = {criterion: round(value, 3) for criterion, value in raw_means.items()}
    failures = []
    for criterion, scores in values.items():
        if min(scores) < MIN_INDIVIDUAL_SCORE:
            failures.append(f"{criterion} individual score below {MIN_INDIVIDUAL_SCORE}")
        if raw_means[criterion] < MIN_MEAN_SCORE:
            failures.append(f"{criterion} mean below {MIN_MEAN_SCORE}")
    return {
        "status": "passed" if not failures else "failed",
        "min_individual": min(min(scores) for scores in values.values()),
        "means": means,
        "failures": failures,
        "thresholds": {"min_individual": MIN_INDIVIDUAL_SCORE, "min_mean": MIN_MEAN_SCORE},
    }


def rubric(path: Path) -> tuple[str, str]:
    content = path.read_bytes()
    try:
        return content.decode("utf-8"), hashlib.sha256(content).hexdigest()
    except UnicodeError as error:
        raise EvaluationError(f"read rubric {path}: {error}") from error


def stage_metadata(run_id: str, workflow: str, rubric_hash: str, prompt_version: str) -> dict:
    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "workflow": workflow,
        "model": os.environ.get("BO_EVAL_MODEL", MODEL),
        "prompt_version": prompt_version,
        "rubric_sha256": rubric_hash,
        "limits": {
            "documents": MAX_DOCUMENTS,
            "input_bytes_per_document": MAX_INPUT_BYTES,
            "output_tokens_per_request": MAX_OUTPUT_TOKENS,
            "total_output_tokens": MAX_TOTAL_OUTPUT_TOKENS,
            "request_timeout_seconds": REQUEST_TIMEOUT_SECONDS,
        },
    }


def trial_workspaces(run_dir: Path, execution: dict) -> list[tuple[str, Path]]:
    trials = execution.get("trials")
    if not isinstance(trials, list) or not trials:
        raise EvaluationError("run.json must contain trials")
    result = []
    for trial in trials:
        if not isinstance(trial, dict) or not isinstance(trial.get("trial_id"), str):
            raise EvaluationError("run.json contains an invalid trial")
        trial_id = trial["trial_id"]
        if not trial_id or trial_id in (".", "..") or "\\" in trial_id or Path(trial_id).name != trial_id:
            raise EvaluationError(f"run.json contains an unsafe trial id: {trial_id}")
        workspace = run_dir / "trials" / trial_id / "home" / ".bo" / "eval"
        result.append((trial_id, workspace))
    return result


def document_result_name(trial_id: str, filename: str) -> str:
    return f"{trial_id}-{filename}.json"


def evaluate_stage(run_dir: Path, execution: dict, workflow: str, api_key: str, endpoint: str, jobs: int, required: bool) -> tuple[dict, list[tuple[str, dict]]]:
    rubric_path = RUBRIC_PATH if workflow == "summarize" else DISTILL_RUBRIC_PATH
    rubric_text, rubric_hash = rubric(rubric_path)
    metadata = stage_metadata(run_dir.name, workflow, rubric_hash, SUMMARY_PROMPT_VERSION if workflow == "summarize" else DISTILL_PROMPT_VERSION)
    all_documents = []
    errors = []
    document_count = 0
    for trial_id, workspace in trial_workspaces(run_dir, execution):
        if not workspace.is_dir() or workspace.is_symlink():
            errors.append(f"{trial_id}: workspace is missing")
            continue
        try:
            documents, missing = load_pairs(workspace) if workflow == "summarize" else (load_distill_documents(workspace), [])
            document_count += len(documents) + len(missing)
            for item in missing:
                errors.append(f"{trial_id}: {item['source_key']}: {item['reason']}")
            for document in documents:
                all_documents.append((trial_id, document))
        except EvaluationError as error:
            errors.append(f"{trial_id}: {error}")
    if document_count > MAX_DOCUMENTS * max(1, len(trial_workspaces(run_dir, execution))):
        errors.append(f"document limit exceeded: {document_count}")
    documents = [document for _, document in all_documents]
    scored, score_errors, output_tokens = score_documents(documents, workflow, rubric_text, api_key, endpoint, jobs) if documents and api_key else ([], ["BO_EVAL_API_KEY is not set"] if not api_key else [], 0)
    errors.extend(score_errors)
    output_documents = []
    for (trial_id, document), score in zip(all_documents, scored):
        if score is None:
            continue
        output_documents.append(
            (
                document_result_name(trial_id, document["filename"] if workflow == "distill" else document["raw_filename"]),
                {
                    **metadata,
                    "trial_id": trial_id,
                    "source_key": document.get("source_key"),
                    "filename": document.get("filename", document.get("raw_filename")),
                    "raw_filename": document.get("raw_filename"),
                    "summary_filename": document.get("summary_filename"),
                    "provenance": [{key: item[key] for key in ("source_key", "kind", "filename")} for item in document.get("inputs", [])],
                    **score,
                },
            )
        )
    gate = quality_gate([value for _, value in output_documents], CRITERIA if workflow == "summarize" else DISTILL_CRITERIA)
    if required and not documents:
        errors.append("task requires at least one distillation document")
    if not documents and not required and not errors:
        status = "skipped"
    else:
        status = "passed" if not errors and gate["status"] == "passed" else "failed"
    stage = {
        **metadata,
        "status": status,
        "document_count": document_count,
        "scored_document_count": len(output_documents),
        "output_tokens": output_tokens,
        "scores": gate["means"],
        "quality_gate": gate,
        "errors": errors,
    }
    return stage, output_documents


def execution_metadata(run_dir: Path) -> dict:
    execution = read_json(run_dir / "run.json", "run.json")
    if not isinstance(execution, dict):
        raise EvaluationError("run.json must contain an object")
    if execution.get("status") not in ("passed", "failed"):
        raise EvaluationError("run.json has an invalid status")
    if execution.get("workflow") not in ("summarize", "distill", "end-to-end"):
        raise EvaluationError("run.json has an invalid workflow")
    return execution


def required_distillation(run_dir: Path, execution: dict) -> bool:
    if execution["workflow"] != "end-to-end":
        return False
    try:
        task = read_json(run_dir / "task.json", "task.json")
    except EvaluationError:
        return True
    return isinstance(task, dict) and isinstance(task.get("success"), dict) and bool(task["success"].get("require_distillation", True))


def publish(run_dir: Path, stages: dict[str, tuple[dict, list[tuple[str, dict]]]], aggregate: dict, force: bool) -> None:
    output = run_dir / "evaluation"
    if output.exists() and not force:
        raise EvaluationError(f"evaluation directory already exists: {output} (use --force to replace it)")
    temporary = Path(tempfile.mkdtemp(prefix=".evaluation-", dir=run_dir))
    try:
        for workflow, (stage, documents) in stages.items():
            stage_dir = temporary / workflow
            stage_dir.mkdir()
            document_dir = stage_dir / "documents"
            document_dir.mkdir()
            for filename, document in documents:
                write_json(document_dir / filename, document)
            write_json(stage_dir / "aggregate.json", stage)
        write_json(temporary / "aggregate.json", aggregate)
        if output.exists():
            shutil.rmtree(output)
        os.replace(temporary, output)
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def evaluate(run_path: str | Path, api_key: str | None = None, api_url: str | None = None, jobs: int = 4, force: bool = False) -> dict:
    run_dir = Path(run_path).resolve()
    if not run_dir.is_dir():
        raise EvaluationError(f"result directory does not exist: {run_dir}")
    if jobs <= 0:
        raise EvaluationError("jobs must be positive")
    execution = execution_metadata(run_dir)
    key = api_key if api_key is not None else os.environ.get("BO_EVAL_API_KEY", "")
    endpoint = api_url or os.environ.get("BO_EVAL_API_URL") or DEFAULT_API_URL
    selected = ("summarize", "distill") if execution["workflow"] == "end-to-end" else (execution["workflow"],)
    stages = {}
    stage_documents = {}
    for workflow in selected:
        stage, documents = evaluate_stage(
            run_dir,
            execution,
            workflow,
            key,
            endpoint,
            jobs,
            required_distillation(run_dir, execution) if workflow == "distill" else False,
        )
        stages[workflow] = stage
        stage_documents[workflow] = documents
    failed_stages = [stage for stage in stages.values() if stage["status"] == "failed"]
    quality_status = "failed" if failed_stages else "passed"
    aggregate = {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_dir.name,
        "workflow": execution["workflow"],
        "status": "passed" if execution["status"] == "passed" and quality_status == "passed" else "failed",
        "quality_status": quality_status,
        "execution": execution,
        "stages": stages,
    }
    publish(run_dir, {workflow: (stages[workflow], stage_documents[workflow]) for workflow in selected}, aggregate, force)
    return aggregate


def main(argv: list[str] | None = None) -> int:
    command = argparse.ArgumentParser(description="Score a bo-eval harness run.")
    command.add_argument("run_dir")
    command.add_argument("--force", action="store_true")
    command.add_argument("--jobs", type=int, default=int(os.environ.get("BO_EVAL_JOBS", "4")))
    args = command.parse_args(sys.argv[1:] if argv is None else argv)
    try:
        aggregate = evaluate(args.run_dir, jobs=args.jobs, force=args.force)
    except EvaluationError as error:
        print(f"evaluation failed: {error}", file=sys.stderr)
        return 1
    print(f"evaluation written: {Path(args.run_dir) / 'evaluation'}")
    if aggregate["status"] != "passed":
        print("evaluation failed: execution or quality gate failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
