#!/usr/bin/env python3
"""Evaluate bo summaries with a separate, opt-in model request."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import socket
import sys
import tempfile
from typing import List, Optional, Tuple, Union
import urllib.error
import urllib.request
from pathlib import Path


SCHEMA_VERSION = 1
MODEL = "deepseek-v4-pro"
PROMPT_VERSION = "summary-evaluator-v3"
DEFAULT_API_URL = "https://api.deepseek.com/chat/completions"
RUBRIC_PATH = Path(__file__).with_name("RUBRIC.md")

MAX_DOCUMENTS = 32
MAX_INPUT_BYTES = 256 * 1024
MAX_OUTPUT_TOKENS = 2_048
REQUEST_TIMEOUT_SECONDS = 60
MAX_TOTAL_OUTPUT_TOKENS = 16_384

LIMITS = {
    "documents": MAX_DOCUMENTS,
    "input_bytes_per_pair": MAX_INPUT_BYTES,
    "output_tokens_per_request": MAX_OUTPUT_TOKENS,
    "request_timeout_seconds": REQUEST_TIMEOUT_SECONDS,
    "total_output_tokens": MAX_TOTAL_OUTPUT_TOKENS,
}

CRITERIA = ("faithfulness", "coverage", "usefulness", "page_cleanliness")
FAITHFULNESS_GROUPS = (
    "source_facts",
    "author_experience_measurements",
    "recommendations_opinions",
    "predictions_forecasts",
)
FAITHFULNESS_ALIASES = {
    "source_facts": ("source_facts",),
    "author_experience_measurements": (
        "author_experience_measurements",
        "author_experience_or_measurements",
        "author_experience/measurements",
    ),
    "recommendations_opinions": (
        "recommendations_opinions",
        "recommendations_or_opinions",
        "recommendations/opinions",
    ),
    "predictions_forecasts": (
        "predictions_forecasts",
        "predictions_or_forecasts",
        "predictions/forecasts",
    ),
}

# Tests can replace this name without reaching into urllib.request.
urlopen = None


class EvaluationError(RuntimeError):
    pass


def _metadata(run_id: str, rubric_sha256: str) -> dict:
    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "rubric_sha256": rubric_sha256,
        "model": MODEL,
        "prompt_version": PROMPT_VERSION,
        "limits": dict(LIMITS),
    }


def _read_text(path: Path, description: str) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise EvaluationError(f"reading {description} failed: {error}") from error
    except UnicodeError as error:
        raise EvaluationError(f"reading {description} failed: {error}") from error


def _read_json(path: Path, description: str):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise EvaluationError(f"reading {description} failed: {error}") from error
    except (UnicodeError, json.JSONDecodeError) as error:
        raise EvaluationError(f"parsing {description} failed: {error}") from error


def _is_integer(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _nonempty_evidence(value, label: str):
    if isinstance(value, str):
        if value.strip():
            return value
    elif isinstance(value, list) and value and all(
        isinstance(item, str) and item.strip() for item in value
    ):
        return value
    raise EvaluationError(f"{label} must be non-empty evidence")


def validate_structured_output(value: object) -> dict:
    """Validate and normalize one model response."""
    if not isinstance(value, dict):
        raise EvaluationError("model output must be a JSON object")

    normalized = {}
    for criterion in CRITERIA:
        section = value.get(criterion)
        if not isinstance(section, dict):
            raise EvaluationError(f"{criterion} must be an object")
        score = section.get("score")
        if not _is_integer(score) or not 1 <= score <= 5:
            raise EvaluationError(f"{criterion}.score must be an integer from 1 to 5")
        evidence = section.get("evidence")
        if criterion != "faithfulness":
            normalized[criterion] = {
                "score": score,
                "evidence": _nonempty_evidence(evidence, f"{criterion}.evidence"),
            }
            continue

        if not isinstance(evidence, dict):
            raise EvaluationError("faithfulness.evidence must group four evidence types")
        grouped = {}
        for group in FAITHFULNESS_GROUPS:
            item = next(
                (evidence[name] for name in FAITHFULNESS_ALIASES[group] if name in evidence),
                None,
            )
            grouped[group] = _nonempty_evidence(item, f"faithfulness.evidence.{group}")
        normalized[criterion] = {"score": score, "evidence": grouped}
    return normalized


def _safe_summary_filename(filename: object) -> str:
    if not isinstance(filename, str) or not filename:
        raise EvaluationError("summary filename must be a Markdown file name")
    path = Path(filename)
    if (
        path.name != filename
        or "/" in filename
        or "\\" in filename
        or path.suffix.lower() != ".md"
    ):
        raise EvaluationError("summary filename must be a Markdown file name")
    return filename


def load_pairs(run_dir: Path) -> list[dict]:
    """Load the newest raw/summary pair for each bo source identity."""
    run_dir = Path(run_dir)
    state = _read_json(run_dir / "state.json", "state.json")
    if not isinstance(state, dict):
        raise EvaluationError("state.json must contain an object")
    raw_records = state.get("raw")
    summary_records = state.get("summaries")
    if not isinstance(raw_records, list) or not isinstance(summary_records, list):
        raise EvaluationError("state.json must contain raw and summaries arrays")

    raw_by_filename = {}
    for index, record in enumerate(raw_records):
        if not isinstance(record, dict):
            raise EvaluationError("state raw records must be objects")
        filename = record.get("filename")
        source_key = record.get("url")
        written_at = record.get("written_at")
        if (
            not isinstance(filename, str)
            or Path(filename).name != filename
            or not filename
            or not isinstance(source_key, str)
            or not source_key
            or not _is_integer(written_at)
            or written_at < 0
        ):
            raise EvaluationError("state raw record has invalid filename, url, or written_at")
        raw_by_filename.setdefault(filename, (source_key, written_at, index))

    summaries_by_source = {}
    for record in summary_records:
        if not isinstance(record, dict):
            raise EvaluationError("state summary records must be objects")
        source_key = record.get("source_key")
        if not isinstance(source_key, str) or not source_key:
            raise EvaluationError("state summary record has an invalid source_key")
        summaries_by_source.setdefault(source_key, record)

    raw_dir = run_dir / "raw"
    summaries_dir = run_dir / "summaries"
    if not raw_dir.is_dir():
        raise EvaluationError(f"raw directory does not exist: {raw_dir}")
    if not summaries_dir.is_dir():
        raise EvaluationError(f"summaries directory does not exist: {summaries_dir}")

    sources = {}
    try:
        raw_files = sorted(raw_dir.iterdir(), key=lambda path: path.name)
    except OSError as error:
        raise EvaluationError(f"reading raw directory failed: {error}") from error
    for raw_path in raw_files:
        if raw_path.suffix.lower() != ".md":
            continue
        if raw_path.is_symlink() or not raw_path.is_file():
            raise EvaluationError(f"raw document is not a regular file: {raw_path.name}")
        record = raw_by_filename.get(raw_path.name)
        if record is None:
            source_key, written_at, state_index = f"raw:{raw_path.name}", 0, 0
        else:
            source_key, written_at, state_index = record
        candidate = (written_at, state_index)
        current = sources.get(source_key)
        if current is None or candidate > current["order"]:
            sources[source_key] = {
                "source_key": source_key,
                "raw_filename": raw_path.name,
                "raw_path": raw_path,
                "order": candidate,
            }

    if not sources:
        raise EvaluationError(f"no raw Markdown documents in {raw_dir}")

    pairs = []
    for source_key in sorted(sources):
        source = sources[source_key]
        summary_record = summaries_by_source.get(source_key)
        if summary_record is None:
            raise EvaluationError(f"missing summary record: {source_key}")
        summary_filename = _safe_summary_filename(summary_record.get("filename"))
        summary_path = summaries_dir / summary_filename
        if summary_path.is_symlink() or not summary_path.is_file():
            raise EvaluationError(f"missing summary file: {summary_filename}")
        raw = _read_text(source["raw_path"], f"raw document {source['raw_filename']}")
        summary = _read_text(summary_path, f"summary {summary_filename}")
        pairs.append(
            {
                "source_key": source_key,
                "raw_filename": source["raw_filename"],
                "summary_filename": summary_filename,
                "raw": raw,
                "summary": summary,
            }
        )
    return pairs


def build_prompt(rubric: str, pair: dict) -> str:
    return f"""You are evaluating a Markdown summary against its raw source.
Use only the supplied raw document and summary. Score each criterion from 1 to 5.
Justify every score with concrete, source-grounded examples rather than generic
claims such as "accurate", "clear", or "covers the source". Name the specific
facts, sections, figures, examples, recommendations, omissions, or boilerplate
that support the score. Use one to three concise sentences for each evidence
value. Do not repeat the rubric or quote more than 20 consecutive words. For
faithfulness, provide separate evidence for source facts, the author's
experience or measurements, recommendations or opinions, and predictions or
forecasts; say "not present" when a category is absent.
Return JSON only with this exact shape:
{{
  "faithfulness": {{"score": 1, "evidence": {{
    "source_facts": "...",
    "author_experience_measurements": "...",
    "recommendations_opinions": "...",
    "predictions_forecasts": "..."
  }}}},
  "coverage": {{"score": 1, "evidence": "..."}},
  "usefulness": {{"score": 1, "evidence": "..."}},
  "page_cleanliness": {{"score": 1, "evidence": "..."}}
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


def _token_count(payload: dict) -> int:
    usage = payload.get("usage")
    if not isinstance(usage, dict) or "completion_tokens" not in usage:
        raise EvaluationError("API response is missing completion token usage")
    tokens = usage["completion_tokens"]
    if not _is_integer(tokens) or tokens < 0:
        raise EvaluationError("API response has invalid completion token usage")
    return tokens


def request_evaluation(endpoint: str, api_key: str, prompt: str) -> Tuple[object, int]:
    body = json.dumps(
        {
            "model": MODEL,
            "messages": [{"role": "user", "content": prompt}],
            "response_format": {"type": "json_object"},
            "thinking": {"type": "disabled"},
            "stream": False,
            "max_tokens": MAX_OUTPUT_TOKENS,
        },
        ensure_ascii=False,
    ).encode("utf-8")
    try:
        request = urllib.request.Request(
            endpoint,
            data=body,
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
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
    except urllib.error.HTTPError as error:
        raise EvaluationError(f"API request failed with HTTP {error.code}") from error
    except (urllib.error.URLError, OSError, socket.timeout, TimeoutError) as error:
        raise EvaluationError(f"API request failed: {error}") from error
    except Exception as error:
        raise EvaluationError(f"API request failed: {error}") from error
    if status is not None and not 200 <= status < 300:
        raise EvaluationError(f"API request failed with HTTP {status}")
    try:
        payload = json.loads(response_body.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise EvaluationError(f"API response is not valid JSON: {error}") from error
    if not isinstance(payload, dict):
        raise EvaluationError("API response must be a JSON object")
    choices = payload.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
        raise EvaluationError("API response is missing choices[0]")
    choice = choices[0]
    if choice.get("finish_reason") == "length":
        raise EvaluationError("API response was truncated at the output token limit")
    message = choice.get("message")
    if not isinstance(message, dict) or "content" not in message:
        raise EvaluationError("API response is missing choices[0].message.content")
    content = message["content"]
    if isinstance(content, str):
        try:
            value = json.loads(content)
        except json.JSONDecodeError as error:
            raise EvaluationError(f"model output is not valid JSON: {error}") from error
    elif isinstance(content, dict):
        value = content
    else:
        raise EvaluationError("model output must be a JSON object or JSON string")
    tokens = _token_count(payload)
    if tokens > MAX_OUTPUT_TOKENS:
        raise EvaluationError(
            f"output token limit exceeded: {tokens} > {MAX_OUTPUT_TOKENS}"
        )
    return value, tokens


def _document_filename(raw_filename: str) -> str:
    filename = re.sub(r"[^A-Za-z0-9._-]", "-", raw_filename)
    return f"{filename}.json"


def _write_json(path: Path, value: object) -> None:
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _publish(run_dir: Path, aggregate: dict, documents: list[tuple[str, dict]]) -> None:
    output = run_dir / "evaluation"
    if output.exists():
        raise EvaluationError(f"evaluation directory already exists: {output}")
    temporary = Path(tempfile.mkdtemp(prefix=".evaluation-", dir=run_dir))
    try:
        if documents:
            document_dir = temporary / "documents"
            document_dir.mkdir()
            for filename, document in documents:
                _write_json(document_dir / filename, document)
        _write_json(temporary / "aggregate.json", aggregate)
        os.rename(temporary, output)
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def _failed_aggregate(metadata: dict, error: Exception) -> dict:
    return {
        **metadata,
        "status": "failed",
        "document_count": 0,
        "output_tokens": 0,
        "error": str(error),
    }


def evaluate(
    run_path: Union[str, Path],
    api_key: Optional[str] = None,
    api_url: Optional[str] = None,
) -> dict:
    run_dir = Path(run_path)
    if not run_dir.is_dir():
        raise EvaluationError(f"result directory does not exist: {run_dir}")
    try:
        rubric_bytes = RUBRIC_PATH.read_bytes()
        rubric = rubric_bytes.decode("utf-8")
    except OSError as error:
        raise EvaluationError(f"reading RUBRIC.md failed: {error}") from error
    except UnicodeError as error:
        raise EvaluationError(f"reading RUBRIC.md failed: {error}") from error
    metadata = _metadata(run_dir.name, hashlib.sha256(rubric_bytes).hexdigest())
    if (run_dir / "evaluation").exists():
        raise EvaluationError(f"evaluation directory already exists: {run_dir / 'evaluation'}")

    try:
        key = api_key if api_key is not None else os.environ.get("BO_EVAL_API_KEY", "")
        if not key:
            raise EvaluationError("BO_EVAL_API_KEY is not set")
        endpoint = api_url or os.environ.get("BO_EVAL_API_URL") or DEFAULT_API_URL
        pairs = load_pairs(run_dir)
        if len(pairs) > MAX_DOCUMENTS:
            raise EvaluationError(
                f"document limit exceeded: {len(pairs)} > {MAX_DOCUMENTS}"
            )
        for pair in pairs:
            input_bytes = len(pair["raw"].encode("utf-8")) + len(
                pair["summary"].encode("utf-8")
            )
            if input_bytes > MAX_INPUT_BYTES:
                raise EvaluationError(
                    f"input limit exceeded for {pair['source_key']}: "
                    f"{input_bytes} > {MAX_INPUT_BYTES} bytes"
                )

        total_tokens = 0
        documents = []
        document_names = set()
        for pair in pairs:
            if total_tokens >= MAX_TOTAL_OUTPUT_TOKENS:
                raise EvaluationError(
                    f"total output token budget exceeded: {total_tokens} >= "
                    f"{MAX_TOTAL_OUTPUT_TOKENS}"
                )
            value, output_tokens = request_evaluation(
                endpoint, key, build_prompt(rubric, pair)
            )
            if (
                not _is_integer(output_tokens)
                or output_tokens < 0
                or output_tokens > MAX_OUTPUT_TOKENS
            ):
                raise EvaluationError("API response has invalid output token usage")
            result = validate_structured_output(value)
            if total_tokens + output_tokens > MAX_TOTAL_OUTPUT_TOKENS:
                raise EvaluationError(
                    "total output token budget exceeded: "
                    f"{total_tokens + output_tokens} > {MAX_TOTAL_OUTPUT_TOKENS}"
                )
            total_tokens += output_tokens
            document_filename = _document_filename(pair["raw_filename"])
            if document_filename in document_names:
                raise EvaluationError(f"document result filename collision: {document_filename}")
            document_names.add(document_filename)
            documents.append(
                (
                    document_filename,
                    {
                        **metadata,
                        "source_key": pair["source_key"],
                        "raw_filename": pair["raw_filename"],
                        "summary_filename": pair["summary_filename"],
                        "output_tokens": output_tokens,
                        **result,
                    },
                )
            )

        scores = {
            criterion: round(
                sum(document[1][criterion]["score"] for document in documents)
                / len(documents),
                3,
            )
            for criterion in CRITERIA
        }
        aggregate = {
            **metadata,
            "status": "success",
            "document_count": len(documents),
            "output_tokens": total_tokens,
            "scores": scores,
        }
        _publish(run_dir, aggregate, documents)
        return aggregate
    except EvaluationError as error:
        _publish(run_dir, _failed_aggregate(metadata, error), [])
        raise


def main(argv: Optional[List[str]] = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 1:
        print("usage: BO_EVAL_API_KEY=... python3 evals/evaluate.py evals/results/<run-id>", file=sys.stderr)
        return 2
    try:
        aggregate = evaluate(argv[0])
    except EvaluationError as error:
        print(f"evaluation failed: {error}", file=sys.stderr)
        return 1
    print(f"evaluation written: {Path(argv[0]) / 'evaluation'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
