import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

import harness


def make_workspace(root: Path) -> Path:
    workspace = root / "workspace"
    workspace.mkdir(parents=True)
    (workspace / "one.md").write_text("one source\n", encoding="utf-8")
    (workspace / "two.md").write_text("two source\n", encoding="utf-8")
    state = {
        "sources": [
            {"source_key": "one", "snapshots": [{"filename": "one.md", "written_at": "2026-01-01T00:00:00Z"}]},
            {"source_key": "two", "snapshots": [{"filename": "two.md", "written_at": "2026-01-01T00:00:01Z"}]},
        ]
    }
    (workspace / "state.json").write_text(json.dumps(state), encoding="utf-8")
    (workspace / "log.jsonl").write_text(
        '{"operation_id":"test","attempt":1,"timestamp":"2026-01-01T00:00:00Z","actor":"test","command":"synth","outcome":"committed"}\n'
        '{"operation_id":"summary","attempt":1,"timestamp":"2026-01-01T00:00:01Z","actor":"test","command":"write_summary","outcome":"committed"}\n',
        encoding="utf-8",
    )
    return workspace


class HarnessTests(unittest.TestCase):
    def test_manifest_rejects_content_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.txt"
            path.write_text("A title, not a source\n", encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness.read_manifest(path)

    def test_command_trial_records_environment_and_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "fixture"
            workspace = make_workspace(fixture)
            trial = root / "run" / "trials" / "trial-001"
            code = (
                "import json, os; from pathlib import Path; "
                "assert all(name not in os.environ for name in ('BO_EVAL_RUN_DIR','BO_EVAL_TRIAL_DIR','BO_EVAL_WORKSPACE_NAME','BO_EVAL_WORKFLOW','BO_EVAL_PROVIDER')); "
                "p=Path(os.environ['BO_EVAL_WORKSPACE']); "
                "state=json.loads((p/'state.json').read_text()); "
                "(p/'summaries').mkdir(); "
                "[ (p/'summaries'/s['snapshots'][0]['filename']).write_text('summary') and s.update({'summary': {'filename': s['snapshots'][0]['filename'], 'derived_from': s['snapshots'][0]['filename']}}) for s in state['sources'] ]; "
                "(p/'state.json').write_text(json.dumps(state)); "
                "Path(p, 'seen').write_text(os.environ['BO_EVAL_TASK']); "
                "print(json.dumps({'command':'run','workflow':'summarize','provider':'deepseek','result':{'telemetry':[{'workflow':'summarize','source_key':'one','terminal_reason':'assistant_message','tool_calls':[{'name':'read_document'}]}]}}))"
            )
            task = {"success": {"min_source_identities": 2, "require_distillation": False}}
            record = harness.run_trial(
                trial,
                fixture,
                task,
                "summarize",
                "command",
                "deepseek",
                [sys.executable, "-c", code],
                30,
                None,
                harness.file_hashes(workspace),
                {},
                {},
                2,
            )
            self.assertEqual(record["status"], "passed")
            self.assertIn("task.json", (trial / "home" / ".bo" / "eval" / "seen").read_text())
            self.assertEqual(record["stage_status"], {"summarize": 0})
            telemetry = json.loads((trial / "trajectory.json").read_text())["telemetry"]
            self.assertEqual(telemetry[0]["status"], "present")
            self.assertEqual(telemetry[0]["telemetry"][0]["tool_calls"][0]["name"], "read_document")
            self.assertEqual(record["telemetry"], telemetry)

    def test_command_trial_fails_when_raw_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "fixture"
            workspace = make_workspace(fixture)
            trial = root / "run" / "trials" / "trial-001"
            code = "from pathlib import Path; Path(__import__('os').environ['BO_EVAL_WORKSPACE'], 'one.md').write_text('changed')"
            record = harness.run_trial(
                trial,
                fixture,
                {"success": {"min_source_identities": 2, "require_distillation": False}},
                "summarize",
                "command",
                "deepseek",
                [sys.executable, "-c", code],
                30,
                None,
                harness.file_hashes(workspace),
                {},
                {},
                2,
            )
            self.assertEqual(record["status"], "failed")
            self.assertEqual(record["failure_class"], "artifact")
            self.assertEqual(record["stage_status"]["summarize"], 1)

    def test_latest_snapshot_compares_timestamps(self):
        source = {
            "source_key": "one",
            "snapshots": [
                {"filename": "older.md", "written_at": "2026-01-01T00:30:00+01:00"},
                {"filename": "newer.md", "written_at": "2026-01-01T00:00:00Z"},
            ],
        }
        self.assertEqual(harness.latest_snapshot(source)["filename"], "newer.md")

    def test_distillation_rejects_stale_summary_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace = Path(directory) / "workspace"
            workspace.mkdir()
            (workspace / "old.md").write_text("old\n", encoding="utf-8")
            (workspace / "new.md").write_text("new\n", encoding="utf-8")
            (workspace / "two.md").write_text("two\n", encoding="utf-8")
            (workspace / "summaries").mkdir()
            one_summary = workspace / "summaries" / "one.md"
            two_summary = workspace / "summaries" / "two.md"
            one_summary.write_text("old summary\n", encoding="utf-8")
            two_summary.write_text("current summary\n", encoding="utf-8")
            (workspace / "distillations").mkdir()
            (workspace / "distillations" / "shared.md").write_text("shared\n", encoding="utf-8")
            state = {
                "sources": [
                    {
                        "source_key": "one",
                        "snapshots": [
                            {"filename": "old.md", "written_at": "2026-01-01T00:00:00Z"},
                            {"filename": "new.md", "written_at": "2026-01-01T00:00:01Z"},
                        ],
                        "summary": {"filename": "one.md", "derived_from": "old.md"},
                    },
                    {
                        "source_key": "two",
                        "snapshots": [{"filename": "two.md", "written_at": "2026-01-01T00:00:00Z"}],
                        "summary": {"filename": "two.md", "derived_from": "two.md"},
                    },
                ],
                "distillation_documents": [{
                    "kind": "distillation",
                    "filename": "shared.md",
                    "derived_from": [
                        {"source_key": "one", "kind": "summary", "filename": "one.md", "content_digest": hashlib.sha256(one_summary.read_bytes()).hexdigest()},
                        {"source_key": "two", "kind": "summary", "filename": "two.md", "content_digest": hashlib.sha256(two_summary.read_bytes()).hexdigest()},
                    ],
                }],
            }
            passed, detail = harness.distillation_check(workspace, state, True)
            self.assertFalse(passed)
            self.assertIn("current summary", detail)

    def test_distillation_check_requires_expected_topics_and_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace = make_workspace(Path(directory))
            (workspace / "distillations").mkdir()
            (workspace / "distillations" / "shared.md").write_text("shared\n", encoding="utf-8")
            state = json.loads((workspace / "state.json").read_text(encoding="utf-8"))
            state["distillation_documents"] = [{
                "kind": "distillation",
                "topic": "shared-facts",
                "filename": "shared.md",
                "derived_from": [
                    {"source_key": "one", "kind": "raw", "filename": "one.md", "content_digest": hashlib.sha256(b"one source\n").hexdigest()},
                    {"source_key": "two", "kind": "raw", "filename": "two.md", "content_digest": hashlib.sha256(b"two source\n").hexdigest()},
                ],
            }]
            passed, detail = harness.distillation_check(
                workspace,
                state,
                True,
                [{"topic": "shared-facts", "source_keys": ["one", "two"]}],
            )
            self.assertTrue(passed, detail)
            passed, detail = harness.distillation_check(
                workspace,
                state,
                True,
                [{"topic": "other-facts", "source_keys": ["one", "two"]}],
            )
            self.assertFalse(passed)
            self.assertIn("missing expected distillation topic", detail)

    def test_external_end_to_end_requires_stable_summary_events(self):
        events = [
            {"command": "write_summary", "outcome": "committed"},
            {"command": "write_distillation", "outcome": "committed"},
        ]
        self.assertTrue(harness.external_summary_stability(events)[0])
        events.append({"command": "write_summary"})
        self.assertFalse(harness.external_summary_stability(events)[0])

    def test_operation_log_rejects_non_public_commands(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace = make_workspace(Path(directory))
            (workspace / "log.jsonl").write_text(
                '{"operation_id":"test","attempt":1,"timestamp":"2026-01-01T00:00:00Z","actor":"test","command":"private","outcome":"committed"}\n',
                encoding="utf-8",
            )
            events, error = harness.read_events(workspace)
            self.assertEqual(events, [])
            self.assertIn("event command is invalid", error)

    def test_timeout_is_reported_without_leaving_the_process_running(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            returncode, timed_out, error = harness.run_process(
                [sys.executable, "-c", "import time; time.sleep(5)"],
                root,
                os.environ.copy(),
                root / "stdout.log",
                root / "stderr.log",
                1,
            )
            self.assertIsNone(returncode)
            self.assertTrue(timed_out)
            self.assertIsNone(error)


if __name__ == "__main__":
    unittest.main()
