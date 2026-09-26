#!/usr/bin/env python3
"""Small, fixture-backed harness for bo agent evaluations."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
EVALS = ROOT / "evals"
FIXTURES = EVALS / "fixtures"
CORPORA = EVALS / "corpora"
RESULTS = EVALS / "results"
EVAL_BINARY = ROOT / "tmp" / "bo-eval"
CLI_BINARY = ROOT / "npm" / "bin" / "bo"
WORKSPACE_NAME = "eval"
WORKFLOWS = ("summarize", "distill", "end-to-end")
PUBLIC_COMMANDS = {
    "seed",
    "snap",
    "state",
    "synth",
    "distill",
    "write_summary",
    "write_distillation",
}


class HarnessError(RuntimeError):
    pass


def now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def read_json(path: Path, description: str) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise HarnessError(f"read {description}: {error}") from error


def resolve_path(value: str, default_dir: Path | None = None) -> Path:
    path = Path(value)
    if not path.is_absolute() and default_dir is not None and "/" not in value and "\\" not in value:
        path = default_dir / path
    elif not path.is_absolute():
        path = ROOT / path
    return path.resolve()


def fixture_path(value: str) -> Path:
    return resolve_path(value, FIXTURES)


def corpus_path(value: str | None) -> Path:
    return resolve_path(value or "default.txt", CORPORA)


def read_manifest(path: Path) -> list[str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        raise HarnessError(f"read corpus {path}: {error}") from error
    sources: list[str] = []
    for line_number, raw_line in enumerate(lines, 1):
        value = raw_line.strip()
        if not value or value.startswith("#"):
            continue
        if value.lower().startswith(("http://", "https://")):
            sources.append(value)
            continue
        if Path(value).is_absolute() or Path(value).suffix.lower() != ".md":
            raise HarnessError(
                f"corpus {path}:{line_number} must contain an HTTP(S) URL or repository-relative Markdown path"
            )
        source = (ROOT / value).resolve()
        try:
            source.relative_to(ROOT)
        except ValueError as error:
            raise HarnessError(f"corpus {path}:{line_number} escapes the repository: {value}") from error
        if not source.is_file():
            raise HarnessError(f"corpus {path}:{line_number} does not exist: {value}")
        sources.append(value)
    if not sources:
        raise HarnessError(f"corpus has no sources: {path}")
    return sources


def ensure_binary(path: Path, package: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        ["go", "build", "-trimpath", "-o", str(path), package],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "unknown build error"
        raise HarnessError(f"build {package}: {detail}")
    return path


def capture(args: argparse.Namespace) -> int:
    corpus = corpus_path(args.corpus)
    sources = read_manifest(corpus)
    target = fixture_path(args.fixture)
    FIXTURES.mkdir(parents=True, exist_ok=True)
    try:
        target.relative_to(FIXTURES)
    except ValueError as error:
        raise HarnessError("capture fixture must be inside evals/fixtures") from error
    if target == FIXTURES:
        raise HarnessError("capture fixture must name a directory")
    if target.is_symlink():
        raise HarnessError(f"refusing to replace symlink fixture: {target}")
    if target.exists():
        if not args.force:
            raise HarnessError(f"fixture already exists: {target} (use --force to recapture it)")
        if not target.is_dir():
            raise HarnessError(f"refusing to replace non-directory fixture: {target}")

    preserved_expected = target / "expected" if target.exists() and (target / "expected").is_dir() and not (target / "expected").is_symlink() else None
    preserved_expected_distillations = None
    if target.exists():
        try:
            previous_task = read_json(target / "task.json", "existing fixture task.json")
        except HarnessError:
            previous_task = None
        if isinstance(previous_task, dict):
            previous_success = previous_task.get("success")
            if isinstance(previous_success, dict) and "expected_distillations" in previous_success:
                preserved_expected_distillations = previous_success["expected_distillations"]

    binary = ensure_binary(EVAL_BINARY, "./evals/cmd/bo-eval")
    temporary_home = Path(tempfile.mkdtemp(prefix="bo-eval-capture-"))
    temporary_fixture = Path(tempfile.mkdtemp(prefix=".fixture-", dir=FIXTURES))
    try:
        env = os.environ.copy()
        env["HOME"] = str(temporary_home)
        result = subprocess.run(
            [str(binary), "capture", "--name", "capture", "--corpus", str(corpus)],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=1200,
        )
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip() or "capture failed"
            raise HarnessError(detail)
        captured_workspace = temporary_home / ".bo" / "capture"
        if not captured_workspace.is_dir():
            raise HarnessError("capture did not produce a workspace")
        task = {
            "schema_version": 1,
            "name": target.name,
            "workflow": "end-to-end",
            "instructions": "Run the configured bo workflow against the seeded workspace. Preserve raw source documents.",
            "success": {
                "min_source_identities": 2,
                "require_distillation": True,
            },
        }
        if preserved_expected_distillations is not None:
            task["success"]["expected_distillations"] = preserved_expected_distillations
        write_json(temporary_fixture / "task.json", task)
        (temporary_fixture / "corpus.txt").write_text("\n".join(sources) + "\n", encoding="utf-8")
        shutil.copytree(captured_workspace, temporary_fixture / "workspace")
        if preserved_expected is not None:
            shutil.copytree(preserved_expected, temporary_fixture / "expected")
        if target.exists():
            shutil.rmtree(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        os.replace(temporary_fixture, target)
        temporary_fixture = None
    finally:
        shutil.rmtree(temporary_home, ignore_errors=True)
        if temporary_fixture is not None:
            shutil.rmtree(temporary_fixture, ignore_errors=True)
    print(f"fixture captured: {target}")
    print(f"sources: {len(sources)}")
    return 0


def load_task(fixture: Path) -> dict:
    task = read_json(fixture / "task.json", "fixture task.json")
    if not isinstance(task, dict):
        raise HarnessError("fixture task.json must contain an object")
    if task.get("schema_version") != 1:
        raise HarnessError("fixture task.json has an unsupported schema_version")
    if not isinstance(task.get("name"), str) or not task["name"]:
        raise HarnessError("fixture task.json name must be non-empty")
    if not isinstance(task.get("instructions"), str) or not task["instructions"].strip():
        raise HarnessError("fixture task.json instructions must be non-empty")
    workflow = task.get("workflow", "end-to-end")
    if workflow not in WORKFLOWS:
        raise HarnessError(f"fixture workflow is invalid: {workflow}")
    success = task.get("success", {})
    if not isinstance(success, dict):
        raise HarnessError("fixture success must contain an object")
    minimum = success.get("min_source_identities", 2)
    if not isinstance(minimum, int) or isinstance(minimum, bool) or minimum < 0:
        raise HarnessError("fixture success.min_source_identities must be a non-negative integer")
    require_distillation = success.get("require_distillation", False)
    if not isinstance(require_distillation, bool):
        raise HarnessError("fixture success.require_distillation must be boolean")
    expected_distillations = success.get("expected_distillations", [])
    if not isinstance(expected_distillations, list):
        raise HarnessError("fixture success.expected_distillations must be an array")
    topics = set()
    for index, expected in enumerate(expected_distillations):
        if not isinstance(expected, dict):
            raise HarnessError(f"fixture success.expected_distillations[{index}] must be an object")
        topic = expected.get("topic")
        if not safe_topic(topic):
            raise HarnessError(f"fixture success.expected_distillations[{index}].topic must be canonical kebab-case")
        if topic in topics:
            raise HarnessError(f"fixture success.expected_distillations has duplicate topic: {topic}")
        topics.add(topic)
        reference = expected.get("reference")
        if not isinstance(reference, str) or not reference:
            raise HarnessError(f"fixture success.expected_distillations[{index}].reference must be non-empty")
        source_keys = expected.get("source_keys")
        if not isinstance(source_keys, list) or len(source_keys) < 2 or any(not isinstance(key, str) or not key for key in source_keys):
            raise HarnessError(f"fixture success.expected_distillations[{index}].source_keys must contain at least two source identities")
        if len(set(source_keys)) != len(source_keys):
            raise HarnessError(f"fixture success.expected_distillations[{index}].source_keys must be unique")
    return {
        "schema_version": 1,
        "name": task["name"],
        "instructions": task["instructions"],
        "workflow": workflow,
        "success": {
            "min_source_identities": minimum,
            "require_distillation": require_distillation,
            "expected_distillations": expected_distillations,
        },
    }


def validate_workspace(workspace: Path) -> dict:
    if not workspace.is_dir() or workspace.is_symlink():
        raise HarnessError(f"fixture workspace is not a directory: {workspace}")
    state_path = workspace / "state.json"
    if state_path.is_symlink() or not state_path.is_file():
        raise HarnessError("workspace state.json must be a regular file")
    for name in ("summaries", "distillations"):
        directory = workspace / name
        if directory.exists() and (directory.is_symlink() or not directory.is_dir()):
            raise HarnessError(f"workspace {name} must be a directory")
    state = read_json(state_path, "fixture state.json")
    if not isinstance(state, dict) or not isinstance(state.get("sources"), list):
        raise HarnessError("fixture state.json must contain a sources array")
    if not state["sources"]:
        raise HarnessError("fixture state.json must contain at least one source")
    raw = sorted(workspace.glob("*.md"))
    if not raw or any(path.is_symlink() or not path.is_file() for path in raw):
        raise HarnessError("fixture workspace must contain regular raw Markdown files")
    filenames = {path.name for path in raw}
    source_keys: set[str] = set()
    raw_owners: dict[str, str] = {}
    summary_names: set[str] = set()
    for source in state["sources"]:
        if not isinstance(source, dict) or not isinstance(source.get("source_key"), str) or not source["source_key"]:
            raise HarnessError("fixture state has an invalid source record")
        if source["source_key"] in source_keys:
            raise HarnessError(f"fixture state has duplicate source: {source['source_key']}")
        source_keys.add(source["source_key"])
        snapshots = source.get("snapshots")
        if not isinstance(snapshots, list) or not snapshots:
            raise HarnessError(f"fixture source has no snapshots: {source['source_key']}")
        source_filenames: set[str] = set()
        for snapshot in snapshots:
            if not isinstance(snapshot, dict) or not safe_markdown_name(snapshot.get("filename")) or snapshot.get("filename") not in filenames:
                raise HarnessError(f"fixture source references a missing raw document: {source['source_key']}")
            snapshot_time(snapshot.get("written_at"))
            filename = snapshot["filename"]
            if filename in source_filenames or filename in raw_owners:
                raise HarnessError(f"fixture state has duplicate raw document: {filename}")
            source_filenames.add(filename)
            raw_owners[filename] = source["source_key"]
        summary = source.get("summary")
        if summary is not None:
            if not isinstance(summary, dict) or not safe_markdown_name(summary.get("filename")):
                raise HarnessError(f"fixture source has an invalid summary: {source['source_key']}")
            if summary["filename"] in summary_names:
                raise HarnessError(f"fixture state has duplicate summary: {summary['filename']}")
            if summary.get("derived_from") not in source_filenames:
                raise HarnessError(f"fixture summary references an unknown raw document: {source['source_key']}")
            summary_names.add(summary["filename"])
    return state


def safe_markdown_name(value: object) -> bool:
    return isinstance(value, str) and "\\" not in value and Path(value).name == value and Path(value).suffix.lower() == ".md"


def safe_topic(value: object) -> bool:
    if not isinstance(value, str) or not value:
        return False
    parts = value.split("-")
    return value == value.lower() and all(part and all(character.isalnum() for character in part) for part in parts)


def validate_expected_distillations(fixture: Path, state: dict, expected: list[dict]) -> None:
    source_keys = {source.get("source_key") for source in state["sources"] if isinstance(source, dict)}
    fixture = fixture.resolve()
    for item in expected:
        reference = Path(item["reference"])
        if reference.is_absolute() or "\\" in item["reference"] or any(part in ("", ".", "..") for part in reference.parts) or not safe_markdown_name(reference.name):
            raise HarnessError(f"fixture expected distillation reference is unsafe: {item['reference']}")
        candidate = fixture / reference
        if candidate.is_symlink():
            raise HarnessError(f"fixture expected distillation reference is a symlink: {item['reference']}")
        path = candidate.resolve()
        try:
            path.relative_to(fixture)
        except ValueError as error:
            raise HarnessError(f"fixture expected distillation reference escapes fixture: {item['reference']}") from error
        if not path.is_file() or not path.read_text(encoding="utf-8").strip():
            raise HarnessError(f"fixture expected distillation reference is missing or empty: {item['reference']}")
        missing = [key for key in item["source_keys"] if key not in source_keys]
        if missing:
            raise HarnessError(f"fixture expected distillation {item['topic']} names unknown sources: {', '.join(missing)}")


def snapshot_time(value: object) -> datetime:
    if not isinstance(value, str) or not value:
        raise HarnessError("snapshot written_at must be an RFC 3339 timestamp")
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        timestamp = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise HarnessError("snapshot written_at must be an RFC 3339 timestamp") from error
    if timestamp.tzinfo is None or timestamp.utcoffset() is None:
        raise HarnessError("snapshot written_at must include a timezone")
    return timestamp.astimezone(timezone.utc)


def file_hashes(directory: Path) -> dict[str, str]:
    if not directory.is_dir():
        return {}
    result = {}
    for path in sorted(directory.glob("*.md")):
        if path.is_symlink() or not path.is_file():
            continue
        try:
            result[path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError as error:
            raise HarnessError(f"read Markdown document {path}: {error}") from error
    return result


def best_effort_file_hashes(directory: Path) -> dict[str, str]:
    try:
        return file_hashes(directory)
    except HarnessError:
        return {}


def latest_snapshot(source: dict) -> dict:
    snapshots = source["snapshots"]
    if any(not isinstance(snapshot, dict) for snapshot in snapshots):
        raise HarnessError(f"source has an invalid snapshot: {source.get('source_key')}")
    return max(enumerate(snapshots), key=lambda item: (snapshot_time(item[1].get("written_at")), item[0]))[1]


def summary_check(workspace: Path, state: dict) -> tuple[bool, str]:
    summary_dir = workspace / "summaries"
    if summary_dir.is_symlink():
        return False, "summaries directory is a symlink"
    missing = []
    expected = set()
    for source in state["sources"]:
        latest = latest_snapshot(source)
        summary = source.get("summary")
        if not isinstance(summary, dict) or summary.get("derived_from") != latest.get("filename"):
            missing.append(source["source_key"])
            continue
        filename = summary.get("filename")
        path = summary_dir / filename if safe_markdown_name(filename) else summary_dir
        try:
            nonempty = bool(path.read_text(encoding="utf-8").strip()) if safe_markdown_name(filename) else False
        except (OSError, UnicodeError):
            nonempty = False
        if not safe_markdown_name(filename) or path.name != filename or path.is_symlink() or not path.is_file() or not nonempty:
            missing.append(source["source_key"])
        else:
            expected.add(filename)
    if missing:
        return False, f"missing or stale summaries for {len(missing)} source identities"
    actual = {path.name for path in summary_dir.glob("*.md")}
    if actual != expected:
        return False, f"summary inventory mismatch: expected {len(expected)}, found {len(actual)}"
    return True, f"{len(state['sources'])} current summaries"


def distillation_check(workspace: Path, state: dict, required: bool, expected: list[dict] | None = None) -> tuple[bool, str]:
    records = state.get("distillation_documents", [])
    if not isinstance(records, list):
        return False, "state distillation_documents is not an array"
    if required and not records:
        return False, "task requires at least one distillation document"
    distillations_dir = workspace / "distillations"
    if distillations_dir.is_symlink():
        return False, "distillations directory is a symlink"
    raw_owners = {}
    summary_owners = {}
    current_raw = {}
    current_summaries = {}
    for source in state["sources"]:
        latest = latest_snapshot(source)
        current_raw[source["source_key"]] = latest["filename"]
        for snapshot in source["snapshots"]:
            raw_owners[snapshot["filename"]] = source["source_key"]
        summary = source.get("summary")
        if isinstance(summary, dict):
            summary_owners[summary.get("filename")] = source["source_key"]
            if summary.get("derived_from") == latest["filename"]:
                current_summaries[summary.get("filename")] = source["source_key"]
    expected_files = set()
    for record in records:
        if not isinstance(record, dict) or record.get("kind") != "distillation":
            return False, "invalid distillation record"
        filename = record.get("filename")
        if not safe_markdown_name(filename):
            return False, f"invalid distillation filename: {filename}"
        if filename in expected_files:
            return False, f"duplicate distillation record: {filename}"
        expected_files.add(filename)
        path = workspace / "distillations" / filename
        try:
            nonempty = bool(path.read_text(encoding="utf-8").strip()) if safe_markdown_name(filename) else False
        except (OSError, UnicodeError):
            nonempty = False
        if not safe_markdown_name(filename) or path.is_symlink() or not path.is_file() or not nonempty:
            return False, f"missing distillation document: {record.get('filename')}"
        inputs = record.get("derived_from")
        if not isinstance(inputs, list) or not inputs:
            return False, f"distillation has no provenance: {record.get('filename')}"
        owners = set()
        for item in inputs:
            if not isinstance(item, dict) or item.get("kind") not in ("raw", "summary"):
                return False, f"invalid provenance in {record.get('filename')}"
            filename = item.get("filename")
            if not safe_markdown_name(filename):
                return False, f"invalid provenance filename in {record.get('filename')}"
            owner_map = raw_owners if item["kind"] == "raw" else summary_owners
            if owner_map.get(filename) != item.get("source_key"):
                return False, f"provenance owner mismatch in {record.get('filename')}"
            if item["kind"] == "raw" and current_raw.get(item.get("source_key")) != filename:
                return False, f"provenance is not current raw input in {record.get('filename')}"
            if item["kind"] == "summary" and current_summaries.get(filename) != item.get("source_key"):
                return False, f"provenance is not current summary input in {record.get('filename')}"
            path = workspace / ("summaries" if item["kind"] == "summary" else "") / str(filename)
            if path.is_symlink() or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest().lower() != str(item.get("content_digest", "")).lower():
                return False, f"provenance digest mismatch in {record.get('filename')}"
            owners.add(item["source_key"])
        if len(owners) < 2:
            return False, f"distillation uses fewer than two source identities: {record.get('filename')}"
    actual = {path.name for path in distillations_dir.glob("*.md")}
    if actual != expected_files:
        return False, f"distillation inventory mismatch: expected {len(expected_files)}, found {len(actual)}"
    for item in expected or []:
        record = next((record for record in records if record.get("topic") == item["topic"]), None)
        if record is None:
            return False, f"missing expected distillation topic: {item['topic']}"
        owners = {
            provenance.get("source_key")
            for provenance in record.get("derived_from", [])
            if isinstance(provenance, dict)
        }
        missing = [source_key for source_key in item["source_keys"] if source_key not in owners]
        if missing:
            return False, f"distillation topic {item['topic']} is missing expected source identities: {', '.join(missing)}"
    return True, f"{len(records)} distillation documents"


def read_events(workspace: Path) -> tuple[list[dict], str | None]:
    path = workspace / "log.jsonl"
    if not path.is_file() or path.is_symlink():
        return [], "operation log is missing"
    events = []
    line_number = 0
    try:
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip():
                value = json.loads(line)
                if not isinstance(value, dict):
                    raise ValueError("event is not an object")
                for field in ("operation_id", "timestamp", "actor", "command", "outcome"):
                    if not isinstance(value.get(field), str) or not value[field]:
                        raise ValueError(f"event field {field} is invalid")
                if not isinstance(value.get("attempt"), int) or isinstance(value["attempt"], bool) or value["attempt"] < 1:
                    raise ValueError("event field attempt is invalid")
                if value["command"] not in PUBLIC_COMMANDS:
                    raise ValueError(f"event command is invalid: {value['command']}")
                if value["outcome"] not in ("committed", "failed"):
                    raise ValueError(f"event outcome is invalid: {value['outcome']}")
                if value["outcome"] == "committed" and value.get("error") is not None:
                    raise ValueError("committed event must not contain an error")
                if value["outcome"] == "failed" and not isinstance(value.get("error"), dict):
                    raise ValueError("failed event must contain an error")
                snapshot_time(value["timestamp"])
                events.append(value)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as error:
        return events, f"operation log line {line_number} is invalid: {error}"
    return events, None if events else "operation log is empty"


def check(name: str, passed: bool, detail: str) -> dict:
    return {"name": name, "status": "passed" if passed else "failed", "detail": detail}


def runtime_json(stdout_path: Path) -> dict | None:
    try:
        value = json.loads(stdout_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def runtime_telemetry(runtime_results: list[dict | None]) -> list[dict]:
    telemetry = []
    for index, runtime_result in enumerate(runtime_results, 1):
        entry = {"stage": index, "workflow": None, "provider": None, "telemetry": []}
        if not isinstance(runtime_result, dict):
            entry["status"] = "missing_runtime_result"
            telemetry.append(entry)
            continue
        entry["workflow"] = runtime_result.get("workflow")
        entry["provider"] = runtime_result.get("provider")
        result = runtime_result.get("result")
        if not isinstance(result, dict) or not isinstance(result.get("telemetry"), list):
            entry["status"] = "missing"
            telemetry.append(entry)
            continue
        entry["telemetry"] = result["telemetry"]
        entry["status"] = "present"
        telemetry.append(entry)
    return telemetry


def stage_status(workflow: str, exit_code: int | None, summary_ok: bool, distill_ok: bool) -> dict[str, int]:
    if workflow == "summarize":
        return {"summarize": 0 if exit_code == 0 and summary_ok else 1}
    if workflow == "distill":
        return {"distill": 0 if exit_code == 0 and distill_ok else 1}
    return {
        "summarize": 0 if summary_ok else 1,
        "distill": 0 if exit_code == 0 and distill_ok else 1,
    }


def failure_class(exit_code: int | None, timed_out: bool, checks: list[dict], result: dict | None) -> str | None:
    if exit_code == 0 and all(item["status"] == "passed" for item in checks):
        return None
    if timed_out:
        return "runtime"
    if exit_code is None:
        return "execution"
    if result and isinstance(result.get("error"), dict):
        kind = result["error"].get("kind", "")
        if str(kind).startswith("provider_"):
            return "provider"
        if kind in ("deadline", "canceled"):
            return "runtime"
    if any(item["name"] in ("state", "operation_log", "raw_unchanged", "summary", "summary_events", "distillation", "distillation_events", "summaries_unchanged", "distillations_unchanged", "source_count") and item["status"] == "failed" for item in checks):
        return "artifact"
    return "execution"


def trial_command(runtime: str, workflow: str, provider: str, command: list[str], binary: Path | None) -> list[str]:
    if runtime == "package":
        assert binary is not None
        return [str(binary), "run", "--name", WORKSPACE_NAME, "--workflow", workflow, "--provider", provider]
    if runtime == "cli":
        if binary is None:
            raise HarnessError("CLI binary is not available")
        args = [str(binary), "synth", WORKSPACE_NAME]
        if workflow != "end-to-end":
            args.append(workflow)
        args.extend(["--provider", provider])
        return args
    if not command:
        raise HarnessError("command runtime requires a command after --")
    return command


def trial_commands(runtime: str, workflow: str, provider: str, command: list[str], binary: Path | None) -> list[list[str]]:
    if workflow == "end-to-end" and runtime in ("package", "cli"):
        return [
            trial_command(runtime, "summarize", provider, command, binary),
            trial_command(runtime, "distill", provider, command, binary),
        ]
    return [trial_command(runtime, workflow, provider, command, binary)]


def external_summary_stability(events: list[dict], required: bool = True) -> tuple[bool, str]:
    distill_index = next(
        (index for index, event in enumerate(events) if event.get("command") == "write_distillation"),
        None,
    )
    if distill_index is None:
        return (
            (True, "no optional distillation write event was recorded")
            if not required
            else (False, "no distillation write event was recorded")
        )
    if required and not any(event.get("command") == "write_distillation" and event.get("outcome") == "committed" for event in events):
        return False, "no committed distillation write event was recorded"
    if any(event.get("command") == "write_summary" for event in events[distill_index + 1 :]):
        return False, "summary write event occurred after distillation began"
    return True, "no summary write event occurred after distillation began"


def run_process(command: list[str], cwd: Path, environment: dict[str, str], stdout: Path, stderr: Path, timeout: int) -> tuple[int | None, bool, str | None]:
    with stdout.open("wb") as stdout_file, stderr.open("wb") as stderr_file:
        try:
            process = subprocess.Popen(
                command,
                cwd=cwd,
                env=environment,
                stdout=stdout_file,
                stderr=stderr_file,
                start_new_session=os.name == "posix",
            )
        except OSError as error:
            return None, False, str(error)
        try:
            return process.wait(timeout=timeout), False, None
        except subprocess.TimeoutExpired:
            if os.name == "posix":
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            else:
                process.kill()
            process.wait()
            return None, True, None


def run_trial(
    trial_dir: Path,
    fixture: Path,
    task: dict,
    workflow: str,
    runtime: str,
    provider: str,
    command: list[str],
    timeout: int,
    binary: Path | None,
    baseline_raw: dict[str, str],
    baseline_summaries: dict[str, str],
    baseline_distillations: dict[str, str],
    source_count: int,
) -> dict:
    trial_dir.mkdir(parents=True, exist_ok=False)
    home = trial_dir / "home"
    workspace = home / ".bo" / WORKSPACE_NAME
    shutil.copytree(fixture / "workspace", workspace)
    task_path = trial_dir / "task.json"
    write_json(task_path, task)
    stdout_path = trial_dir / "stdout.log"
    stderr_path = trial_dir / "stderr.log"
    actual_commands = trial_commands(runtime, workflow, provider, command, binary)
    started = now()
    print(f"{trial_dir.name} started", flush=True)
    environment = os.environ.copy()
    for name in (
        "BO_EVAL_RUN_DIR",
        "BO_EVAL_TRIAL_DIR",
        "BO_EVAL_WORKSPACE_NAME",
        "BO_EVAL_WORKFLOW",
        "BO_EVAL_PROVIDER",
    ):
        environment.pop(name, None)
    environment.update(
        {
            "HOME": str(home),
            "BO_EVAL_WORKSPACE": str(workspace),
            "BO_EVAL_TASK": str(task_path),
        }
    )
    timed_out = False
    exit_code: int | None = None
    process_error: str | None = None
    state = None
    final_raw: dict[str, str] = {}
    events: list[dict] = []
    stage_outputs: list[tuple[Path, Path]] = []
    summary_baseline: dict[str, str] | None = None
    summary_baseline_error: str | None = None
    for index, actual_command in enumerate(actual_commands, 1):
        stage_stdout = stdout_path if len(actual_commands) == 1 else trial_dir / f"stage-{index}.stdout.log"
        stage_stderr = stderr_path if len(actual_commands) == 1 else trial_dir / f"stage-{index}.stderr.log"
        stage_outputs.append((stage_stdout, stage_stderr))
        exit_code, timed_out, process_error = run_process(
            actual_command, ROOT, environment, stage_stdout, stage_stderr, timeout
        )
        if process_error:
            process_error = f"stage {index}: {process_error}"
        if workflow == "end-to-end" and index == 1 and exit_code == 0:
            try:
                summary_baseline = file_hashes(workspace / "summaries")
            except HarnessError as error:
                summary_baseline_error = str(error)
        if timed_out or process_error or exit_code != 0:
            break
    if len(actual_commands) > 1:
        with stdout_path.open("wb") as combined_stdout, stderr_path.open("wb") as combined_stderr:
            for index, (stage_stdout, stage_stderr) in enumerate(stage_outputs, 1):
                if index > 1:
                    combined_stdout.write(f"\n--- stage {index} stdout ---\n".encode())
                    combined_stderr.write(f"\n--- stage {index} stderr ---\n".encode())
                with stage_stdout.open("rb") as source:
                    shutil.copyfileobj(source, combined_stdout)
                with stage_stderr.open("rb") as source:
                    shutil.copyfileobj(source, combined_stderr)
    if process_error:
        with stderr_path.open("ab") as stderr:
            stderr.write((process_error + "\n").encode())
    runtime_results = [runtime_json(stage_stdout) for stage_stdout, _ in stage_outputs]
    runtime_result = next((result for result in reversed(runtime_results) if result is not None), None)
    telemetry = runtime_telemetry(runtime_results)
    exit_detail = "process exited 0" if exit_code == 0 else "process timed out" if timed_out else process_error or f"process exit: {exit_code}"
    checks: list[dict] = [check("exit_code", exit_code == 0, exit_detail)]
    summary_ok = False
    distill_ok = workflow == "summarize"
    summary_stable = workflow != "end-to-end"
    if workspace.is_dir():
        events, event_error = read_events(workspace)
        try:
            state = validate_workspace(workspace)
            checks.append(check("state", True, "state and raw documents are valid"))
        except HarnessError as error:
            state = None
            checks.append(check("state", False, str(error)))
        raw_hash_error = None
        try:
            final_raw = file_hashes(workspace)
        except HarnessError as error:
            raw_hash_error = str(error)
        checks.append(check("raw_unchanged", raw_hash_error is None and final_raw == baseline_raw, "raw Markdown hashes match the fixture" if raw_hash_error is None and final_raw == baseline_raw else raw_hash_error or "raw Markdown changed"))
        if state is not None:
            if workflow in ("summarize", "end-to-end"):
                try:
                    summary_ok, detail = summary_check(workspace, state)
                except (OSError, UnicodeError, KeyError, TypeError) as error:
                    summary_ok, detail = False, f"summary check failed: {error}"
                checks.append(check("summary", summary_ok, detail))
                if len(baseline_summaries) < source_count:
                    summary_events = any(
                        event.get("command") == "write_summary" and event.get("outcome") == "committed"
                        for event in events
                    )
                    checks.append(check("summary_events", summary_events, "committed summary write events were recorded" if summary_events else "no committed summary write event was recorded"))
            else:
                summary_ok = True
            if workflow == "distill":
                try:
                    final_summaries = file_hashes(workspace / "summaries")
                    summary_hash_error = None
                except HarnessError as error:
                    final_summaries = {}
                    summary_hash_error = str(error)
                checks.append(check("summaries_unchanged", summary_hash_error is None and final_summaries == baseline_summaries, "summary Markdown hashes match the fixture" if summary_hash_error is None and final_summaries == baseline_summaries else summary_hash_error or "summary Markdown changed"))
            if workflow == "summarize":
                try:
                    final_distillations = file_hashes(workspace / "distillations")
                    distillation_hash_error = None
                except HarnessError as error:
                    final_distillations = {}
                    distillation_hash_error = str(error)
                checks.append(check("distillations_unchanged", distillation_hash_error is None and final_distillations == baseline_distillations, "distillation hashes match the fixture" if distillation_hash_error is None and final_distillations == baseline_distillations else distillation_hash_error or "distillation documents changed"))
            try:
                distill_ok, detail = distillation_check(
                    workspace,
                    state,
                    workflow == "end-to-end" and task["success"]["require_distillation"],
                    task["success"]["expected_distillations"] if workflow in ("distill", "end-to-end") else None,
                )
            except (OSError, UnicodeError, KeyError, TypeError) as error:
                distill_ok, detail = False, f"distillation check failed: {error}"
            if workflow in ("distill", "end-to-end"):
                checks.append(check("distillation", distill_ok, detail))
            if workflow == "end-to-end" and task["success"]["require_distillation"]:
                distillation_events = any(
                    event.get("command") == "write_distillation" and event.get("outcome") == "committed"
                    for event in events
                )
                checks.append(check("distillation_events", distillation_events, "committed distillation write events were recorded" if distillation_events else "no committed distillation write event was recorded"))
                distill_ok = distill_ok and distillation_events
            if workflow == "summarize":
                distill_ok = True
            if workflow == "end-to-end":
                if runtime in ("package", "cli"):
                    try:
                        final_summaries = file_hashes(workspace / "summaries")
                        summary_stable = summary_baseline_error is None and summary_baseline is not None and final_summaries == summary_baseline
                        detail = "summary Markdown hashes match before and after distillation" if summary_stable else summary_baseline_error or "summary Markdown changed during distillation"
                    except HarnessError as error:
                        summary_stable, detail = False, str(error)
                else:
                    summary_stable, detail = external_summary_stability(events, task["success"]["require_distillation"])
                checks.append(check("summaries_unchanged", summary_stable, detail))
                distill_ok = distill_ok and summary_stable
            if workflow == "end-to-end" and len(state["sources"]) < max(source_count, task["success"]["min_source_identities"]):
                checks.append(check("source_count", False, f"only {len(state['sources'])} source identities"))
            elif workflow == "end-to-end":
                checks.append(check("source_count", True, f"{len(state['sources'])} source identities"))
        checks.append(check("operation_log", event_error is None, event_error or f"{len(events)} operation events"))
    else:
        events = []
        checks.append(check("state", False, "runtime did not leave the expected workspace"))
        checks.append(check("operation_log", False, "runtime did not leave the expected workspace"))
    statuses = stage_status(workflow, exit_code, summary_ok, distill_ok)
    status = "passed" if all(item["status"] == "passed" for item in checks) else "failed"
    trajectory = {
        "schema_version": 2,
        "command": actual_commands[0] if len(actual_commands) == 1 else actual_commands,
        "stdout": "stdout.log",
        "stderr": "stderr.log",
        "events": events,
        "telemetry": telemetry,
        "workspace": "home/.bo/eval",
    }
    if len(actual_commands) > 1:
        trajectory["stage_outputs"] = [
            {"stdout": str(stdout.relative_to(trial_dir)), "stderr": str(stderr.relative_to(trial_dir))}
            for stdout, stderr in stage_outputs
        ]
    write_json(trial_dir / "trajectory.json", trajectory)
    finished = now()
    record = {
        "schema_version": 1,
        "trial_id": trial_dir.name,
        "status": status,
        "failure_class": failure_class(exit_code, timed_out, checks, runtime_result),
        "runtime": runtime,
        "provider": provider,
        "workflow": workflow,
        "started_at": started,
        "finished_at": finished,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "stage_status": statuses,
        "checks": checks,
        "telemetry": telemetry,
        "outcome": {
            "workspace": "home/.bo/eval",
            "source_count": len(state.get("sources", [])) if isinstance(state, dict) else 0,
            "summary_count": len(best_effort_file_hashes(workspace / "summaries")) if workspace.is_dir() else 0,
            "distillation_count": len(state.get("distillation_documents", [])) if isinstance(state, dict) and isinstance(state.get("distillation_documents", []), list) else 0,
            "raw_hashes": final_raw,
            "summary_hashes": best_effort_file_hashes(workspace / "summaries") if workspace.is_dir() else {},
        },
    }
    if runtime_result is not None:
        record["runtime_result"] = runtime_result
        nonempty_runtime_results = [result for result in runtime_results if result is not None]
        if len(nonempty_runtime_results) > 1:
            record["runtime_results"] = nonempty_runtime_results
        metrics = [
            result["result"]["metrics"]
            for result in nonempty_runtime_results
            if isinstance(result.get("result"), dict) and isinstance(result["result"].get("metrics"), dict)
        ]
        if metrics:
            record["metrics"] = {
                "turns": sum(item.get("turns", 0) for item in metrics),
                "tool_calls": sum(item.get("tool_calls", 0) for item in metrics),
                "duration": sum(item.get("duration", 0) for item in metrics),
                "usage": {
                    field: sum(item.get("usage", {}).get(field, 0) for item in metrics if isinstance(item.get("usage"), dict))
                    for field in ("prompt_tokens", "completion_tokens", "total_tokens")
                },
            }
    if process_error:
        record["process_error"] = process_error
    write_json(trial_dir / "trial.json", record)
    print(f"{trial_dir.name} {status} (summarize={statuses.get('summarize', 0)}, distill={statuses.get('distill', 0)})", flush=True)
    return record


def make_run_id(workflow: str) -> str:
    RESULTS.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    base = f"{workflow}-{stamp}-{os.getpid()}"
    candidate = RESULTS / base
    suffix = 1
    while candidate.exists():
        candidate = RESULTS / f"{base}-{suffix}"
        suffix += 1
    candidate.mkdir(parents=True)
    return candidate.name


def records_have_check(records: list[dict], name: str) -> bool:
    checks = [check for record in records for check in record.get("checks", []) if check.get("name") == name]
    return bool(checks) and all(check.get("status") == "passed" for check in checks)


def run_eval(args: argparse.Namespace) -> int:
    if args.tools != "all":
        raise HarnessError("custom tool lists are not part of the public-API harness; use --tools all")
    if args.trials <= 0 or args.trials > 100:
        raise HarnessError("--trials must be between 1 and 100")
    if args.jobs <= 0 or args.jobs > args.trials:
        raise HarnessError("--jobs must be between 1 and --trials")
    if args.timeout <= 0:
        raise HarnessError("--timeout-seconds must be positive")
    if args.fixture != "default" and args.corpus is not None:
        raise HarnessError("use either --fixture or --corpus, not both")
    fixture = fixture_path(args.fixture if args.corpus is None else Path(args.corpus).stem)
    if args.corpus is not None and not fixture.is_dir():
        raise HarnessError(f"fixture not found for corpus {args.corpus}: {fixture}; run capture first")
    if not fixture.is_dir():
        raise HarnessError(f"fixture not found: {fixture}; run `evals/harness capture` first")
    task = load_task(fixture)
    workflow = args.workflow or task["workflow"]
    if workflow not in WORKFLOWS:
        raise HarnessError(f"unsupported workflow: {workflow}")
    task = {**task, "workflow": workflow, "provider": args.provider}
    fixture_state = validate_workspace(fixture / "workspace")
    validate_expected_distillations(fixture, fixture_state, task["success"]["expected_distillations"])
    baseline_raw = file_hashes(fixture / "workspace")
    baseline_summaries = file_hashes(fixture / "workspace" / "summaries")
    baseline_distillations = file_hashes(fixture / "workspace" / "distillations")
    source_count = len(fixture_state["sources"])
    command = list(args.command or [])
    if command and command[0] == "--":
        command = command[1:]
    if args.runtime == "command" and not command:
        raise HarnessError("command runtime requires a command after --")
    binary: Path | None = None
    if args.runtime == "package":
        binary = ensure_binary(EVAL_BINARY, "./evals/cmd/bo-eval")
    elif args.runtime == "cli":
        binary = CLI_BINARY if CLI_BINARY.is_file() else ensure_binary(ROOT / "tmp" / "bo-cli", "./cmd/bo")
    run_id = make_run_id(workflow)
    run_dir = RESULTS / run_id
    write_json(run_dir / "task.json", task)
    if (fixture / "corpus.txt").is_file():
        shutil.copy2(fixture / "corpus.txt", run_dir / "corpus.txt")
    metadata = {
        "schema_version": 2,
        "run_id": run_id,
        "status": "running",
        "task": task.get("name", fixture.name),
        "fixture": str(fixture.relative_to(ROOT)) if fixture.is_relative_to(ROOT) else str(fixture),
        "workflow": workflow,
        "runtime": args.runtime,
        "provider": args.provider,
        "trials_requested": args.trials,
        "jobs": args.jobs,
        "started_at": now(),
    }
    write_json(run_dir / "run.json", metadata)
    trial_root = run_dir / "trials"
    trial_root.mkdir()
    records = []
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = {
            executor.submit(
                run_trial,
                trial_root / f"trial-{index:03d}",
                fixture,
                task,
                workflow,
                args.runtime,
                args.provider,
                command,
                args.timeout,
                binary,
                baseline_raw,
                baseline_summaries,
                baseline_distillations,
                source_count,
            ): trial_root / f"trial-{index:03d}"
            for index in range(1, args.trials + 1)
        }
        for future in as_completed(futures):
            trial_dir = futures[future]
            try:
                records.append(future.result())
            except Exception as error:
                trial_dir.mkdir(parents=True, exist_ok=True)
                record = {
                    "schema_version": 1,
                    "trial_id": trial_dir.name,
                    "status": "failed",
                    "failure_class": "harness",
                    "workflow": workflow,
                    "runtime": args.runtime,
                    "provider": args.provider,
                    "stage_status": {stage: 1 for stage in ("summarize", "distill") if stage == workflow or workflow == "end-to-end"},
                    "error": str(error),
                }
                write_json(trial_dir / "trial.json", record)
                records.append(record)
    records.sort(key=lambda item: item["trial_id"])
    passed = [record["status"] == "passed" for record in records]
    source_counts = [record.get("outcome", {}).get("source_count", 0) for record in records]
    distillation_counts = [record.get("outcome", {}).get("distillation_count", 0) for record in records]
    stage_statuses = {}
    for stage in WORKFLOWS:
        values = [record["stage_status"].get(stage) for record in records if stage in record["stage_status"]]
        if values:
            stage_statuses[stage] = 0 if all(value == 0 for value in values) else 1
    metadata.update(
        {
            "status": "passed" if all(passed) else "failed",
            "failure_class": next((record["failure_class"] for record in records if record["failure_class"]), None),
            "finished_at": now(),
            "trials": records,
            "pass_at_k": any(passed),
            "pass_caret_k": all(passed),
            "stage_status": stage_statuses,
            "successful_source_identities": min(source_counts, default=0),
            "distillation_documents": min(distillation_counts, default=0),
            "raw_documents_unchanged": records_have_check(records, "raw_unchanged"),
            "missing_summaries": any(
                check.get("name") == "summary" and check.get("status") != "passed"
                for record in records
                for check in record.get("checks", [])
            ),
            "summary_documents_unchanged": (
                records_have_check(records, "summaries_unchanged")
                if workflow in ("distill", "end-to-end")
                else None
            ),
        }
    )
    write_json(run_dir / "run.json", metadata)
    print(f"run: {run_dir}")
    for stage, status in stage_statuses.items():
        print(f"{stage} status: {status}")
    print(f"successful source identities: {metadata['successful_source_identities']}")
    print(f"distillation documents: {metadata['distillation_documents']}")
    print(f"raw unchanged: {1 if metadata['raw_documents_unchanged'] else 0}")
    if metadata["summary_documents_unchanged"] is not None:
        print(f"summary unchanged: {1 if metadata['summary_documents_unchanged'] else 0}")
    print(f"missing summaries: {1 if metadata['missing_summaries'] else 0}")
    print(f"status: {0 if metadata['status'] == 'passed' else 1}")
    return 0 if metadata["status"] == "passed" else 1


def grade(args: argparse.Namespace) -> int:
    run_dir = resolve_path(args.run_dir)
    if not run_dir.is_dir():
        raise HarnessError(f"result directory does not exist: {run_dir}")
    command = [sys.executable, str(EVALS / "evaluate.py"), str(run_dir), "--jobs", str(args.jobs)]
    if args.force:
        command.append("--force")
    result = subprocess.run(command, cwd=ROOT)
    return result.returncode


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="evals/harness", description="Run bo evaluations against seeded fixtures.")
    subcommands = root.add_subparsers(dest="subcommand")
    capture_parser = subcommands.add_parser("capture", help="snap a corpus into a new fixture")
    capture_parser.add_argument("--corpus", default=None, help="corpus name or repository path")
    capture_parser.add_argument("--fixture", default="default", help="fixture name or path")
    capture_parser.add_argument("--force", action="store_true", help="replace an existing fixture")
    run_parser = subcommands.add_parser("run", help="run isolated fixture trials")
    run_parser.add_argument("--fixture", default="default", help="fixture name or path")
    run_parser.add_argument("--corpus", default=None, help="select the fixture named after this corpus")
    run_parser.add_argument("--workflow", choices=WORKFLOWS)
    run_parser.add_argument("--runtime", choices=("package", "cli", "command"), default="package")
    run_parser.add_argument("--provider", choices=("deepseek", "gemini"), default="deepseek")
    run_parser.add_argument("--trials", type=int, default=1)
    run_parser.add_argument("--jobs", type=int, default=1)
    run_parser.add_argument("--timeout-seconds", dest="timeout", type=int, default=900)
    run_parser.add_argument("--tools", default="all", help="compatibility option; only all is supported")
    run_parser.add_argument("command", nargs=argparse.REMAINDER)
    grade_parser = subcommands.add_parser("grade", help="score a completed run")
    grade_parser.add_argument("run_dir")
    grade_parser.add_argument("--force", action="store_true")
    grade_parser.add_argument("--jobs", type=int, default=4)
    return root


def main(argv: list[str] | None = None) -> int:
    raw = list(sys.argv[1:] if argv is None else argv)
    if raw and raw[0] in ("-h", "--help"):
        parser().parse_args(raw)
        return 0
    if not raw or raw[0] not in ("capture", "run", "grade"):
        raw.insert(0, "run")
    args = parser().parse_args(raw)
    try:
        if args.subcommand == "capture":
            return capture(args)
        if args.subcommand == "grade":
            if args.jobs <= 0:
                raise HarnessError("--jobs must be positive")
            return grade(args)
        return run_eval(args)
    except HarnessError as error:
        print(f"harness failed: {error}", file=sys.stderr)
        return 2
    except subprocess.TimeoutExpired:
        print("harness failed: command timed out", file=sys.stderr)
        return 2
    except (OSError, UnicodeError, ValueError) as error:
        print(f"harness failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
