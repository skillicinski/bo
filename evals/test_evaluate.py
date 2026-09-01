import http.client
import json
import tempfile
import unittest
from pathlib import Path

import evaluate


class FakeResponse:
    status = 200

    def __init__(self, body):
        self.body = body

    def getcode(self):
        return self.status

    def read(self):
        return self.body

    def close(self):
        pass


def make_run(root: Path) -> Path:
    run = root / "run"
    workspace = run / "trials" / "trial-001" / "home" / ".bo" / "eval"
    workspace.mkdir(parents=True)
    (workspace / "one.md").write_text("one source\n", encoding="utf-8")
    (workspace / "summaries").mkdir()
    (workspace / "summaries" / "one.md").write_text("summary\n", encoding="utf-8")
    state = {
        "sources": [{
            "source_key": "one",
            "snapshots": [{"filename": "one.md", "written_at": "2026-01-01T00:00:00Z"}],
            "summary": {"filename": "one.md", "derived_from": "one.md"},
        }]
    }
    (workspace / "state.json").write_text(json.dumps(state), encoding="utf-8")
    (run / "run.json").parent.mkdir(parents=True, exist_ok=True)
    (run / "run.json").write_text(json.dumps({
        "schema_version": 2,
        "run_id": "run",
        "workflow": "summarize",
        "status": "passed",
        "trials": [{"trial_id": "trial-001", "status": "passed"}],
    }), encoding="utf-8")
    return run


class EvaluatorTests(unittest.TestCase):
    def test_quality_gate_applies_both_thresholds(self):
        document = {criterion: {"score": 5, "evidence": "x"} for criterion in evaluate.CRITERIA}
        document["coverage"]["score"] = 3
        gate = evaluate.quality_gate([document], evaluate.CRITERIA)
        self.assertEqual(gate["status"], "failed")
        self.assertIn("coverage individual score below 4", gate["failures"])
        self.assertIn("coverage mean below 4.6", gate["failures"])

    def test_missing_api_key_publishes_failed_evaluation(self):
        with tempfile.TemporaryDirectory() as directory:
            aggregate = evaluate.evaluate(make_run(Path(directory)), api_key="", jobs=1)
            self.assertEqual(aggregate["status"], "failed")
            self.assertEqual(aggregate["execution"]["status"], "passed")
            self.assertTrue((Path(directory) / "run" / "evaluation" / "aggregate.json").is_file())
            self.assertEqual(aggregate["stages"]["summarize"]["status"], "failed")

    def test_request_accepts_missing_usage(self):
        old_urlopen = evaluate.urlopen
        try:
            body = json.dumps({
                "choices": [{
                    "finish_reason": "stop",
                    "message": {"content": "```json\n{\"ok\": true}\n```"},
                }]
            }).encode()
            evaluate.urlopen = lambda request, timeout: FakeResponse(body)
            value, tokens = evaluate.request_evaluation("https://example.test", "key", "prompt")
            self.assertEqual(value, {"ok": True})
            self.assertEqual(tokens, 0)
        finally:
            evaluate.urlopen = old_urlopen

    def test_request_retries_incomplete_responses(self):
        old_urlopen = evaluate.urlopen
        calls = []
        body = json.dumps({"choices": [{"finish_reason": "stop", "message": {"content": "{}"}}]}).encode()
        try:
            def fake_urlopen(request, timeout):
                calls.append(1)
                if len(calls) == 1:
                    raise http.client.IncompleteRead(b"partial")
                return FakeResponse(body)

            evaluate.urlopen = fake_urlopen
            value, tokens = evaluate.request_evaluation("https://example.test", "key", "prompt")
        finally:
            evaluate.urlopen = old_urlopen
        self.assertEqual(value, {})
        self.assertEqual(tokens, 0)
        self.assertEqual(len(calls), 2)

    def test_score_failures_keep_document_alignment(self):
        documents = [
            {"source_key": "one", "raw_filename": "one.md", "summary_filename": "one-summary.md", "raw": "one", "summary": "one summary"},
            {"source_key": "two", "raw_filename": "two.md", "summary_filename": "two-summary.md", "raw": "two", "summary": "two summary"},
        ]
        score = {
            "faithfulness": {
                "score": 5,
                "evidence": {
                    "source_facts": "facts",
                    "author_experience_measurements": "not present",
                    "recommendations_opinions": "not present",
                    "predictions_forecasts": "not present",
                },
            },
            "coverage": {"score": 5, "evidence": "covered"},
            "usefulness": {"score": 5, "evidence": "useful"},
            "page_cleanliness": {"score": 5, "evidence": "clean"},
        }
        old_request = evaluate.request_evaluation
        try:
            def fake_request(endpoint, api_key, prompt):
                if "Source identity: one" in prompt:
                    raise evaluate.EvaluationError("first document failed")
                return score, 1

            evaluate.request_evaluation = fake_request
            scored, errors, tokens = evaluate.score_documents(documents, "summarize", "rubric", "key", "endpoint", 2)
        finally:
            evaluate.request_evaluation = old_request
        self.assertIsNone(scored[0])
        self.assertEqual(scored[1]["faithfulness"]["score"], 5)
        self.assertIn("one.md", errors[0])
        self.assertEqual(tokens, 1)


if __name__ == "__main__":
    unittest.main()
