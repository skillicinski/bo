import json
import os
import shutil
import socket
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

from evals import evaluate


def valid_result(score=4):
    return {
        "faithfulness": {
            "score": score,
            "evidence": {
                "source_facts": ["source fact"],
                "author_experience_measurements": ["reported measurement"],
                "recommendations_opinions": ["recommendation"],
                "predictions_forecasts": ["forecast"],
            },
        },
        "coverage": {"score": score, "evidence": ["main subject"]},
        "usefulness": {"score": score, "evidence": ["concise"]},
        "page_cleanliness": {"score": score, "evidence": ["no boilerplate"]},
    }


class FakeResponse:
    status = 200

    def __init__(self, payload):
        self.payload = json.dumps(payload).encode("utf-8")

    def read(self):
        return self.payload

    def getcode(self):
        return self.status

    def close(self):
        pass


class FakeOpener:
    def __init__(self, payload=None, error=None):
        self.payload = payload or {
            "choices": [{"message": {"content": json.dumps(valid_result())}}],
            "usage": {"completion_tokens": 10},
        }
        self.error = error
        self.requests = []

    def __call__(self, request, timeout):
        self.requests.append((request, timeout))
        if self.error:
            raise self.error
        return FakeResponse(self.payload)


class EvaluateTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="bo-evaluate-test-"))
        self.addCleanup(shutil.rmtree, self.root)
        self.original_urlopen = evaluate.urlopen
        self.addCleanup(setattr, evaluate, "urlopen", self.original_urlopen)

    def make_run(self, count=1, raw_size=None, same_source=False, run_id="run-1"):
        run = self.root / run_id
        raw_dir = run / "raw"
        summary_dir = run / "summaries"
        raw_dir.mkdir(parents=True)
        summary_dir.mkdir()
        raw_records = []
        summary_records = []
        for index in range(count):
            filename = f"article-{index}.md"
            source = "https://example.test/article" if same_source else f"https://example.test/{index}"
            raw = "raw source\n" if raw_size is None else "x" * raw_size
            raw_path = raw_dir / filename
            raw_path.write_text(raw, encoding="utf-8")
            summary_filename = f"summary-{index}.md"
            (summary_dir / summary_filename).write_text("summary\n", encoding="utf-8")
            raw_records.append({"filename": filename, "url": source, "written_at": index + 1})
            summary_records.append({"filename": summary_filename, "source_key": source})
        (run / "state.json").write_text(
            json.dumps({"raw": raw_records, "summaries": summary_records}), encoding="utf-8"
        )
        return run

    def use_valid_opener(self, tokens=10):
        opener = FakeOpener(
            {
                "choices": [{"message": {"content": json.dumps(valid_result())}}],
                "usage": {"completion_tokens": tokens},
            }
        )
        evaluate.urlopen = opener
        return opener

    def test_valid_and_malformed_structured_output(self):
        normalized = evaluate.validate_structured_output(valid_result())
        self.assertEqual(normalized["faithfulness"]["score"], 4)
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.validate_structured_output([])
        malformed = valid_result()
        malformed["coverage"] = {"score": 4}
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.validate_structured_output(malformed)

    def test_invalid_scores_missing_evidence_and_epistemic_groups(self):
        invalid_score = valid_result()
        invalid_score["usefulness"]["score"] = 6
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.validate_structured_output(invalid_score)

        missing_evidence = valid_result()
        missing_evidence["coverage"]["evidence"] = []
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.validate_structured_output(missing_evidence)

        missing_group = valid_result()
        del missing_group["faithfulness"]["evidence"]["predictions_forecasts"]
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.validate_structured_output(missing_group)

    def test_latest_raw_snapshot_is_selected_per_source(self):
        run = self.make_run(count=2, same_source=True)
        state = json.loads((run / "state.json").read_text())
        state["raw"][0]["written_at"] = 10
        state["raw"][1]["written_at"] = 20
        state["summaries"] = [{"filename": "summary-1.md", "source_key": "https://example.test/article"}]
        (run / "state.json").write_text(json.dumps(state))
        pairs = evaluate.load_pairs(run)
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["raw_filename"], "article-1.md")

    def test_request_contract_and_rubric_metadata(self):
        run = self.make_run()
        opener = self.use_valid_opener()
        aggregate = evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        request, timeout = opener.requests[0]
        body = json.loads(request.data)
        self.assertEqual(body["model"], "deepseek-v4-pro")
        self.assertEqual(body["response_format"], {"type": "json_object"})
        self.assertEqual(body["thinking"], {"type": "disabled"})
        self.assertEqual(body["max_tokens"], 2048)
        self.assertEqual(timeout, 60)
        self.assertEqual(aggregate["status"], "success")
        self.assertEqual(aggregate["document_count"], 1)
        self.assertEqual(aggregate["rubric_sha256"], __import__("hashlib").sha256(
            evaluate.RUBRIC_PATH.read_bytes()
        ).hexdigest())
        self.assertTrue((run / "evaluation" / "aggregate.json").is_file())
        self.assertEqual(len(list((run / "evaluation" / "documents").glob("*.json"))), 1)
        self.assertFalse(list(run.glob(".evaluation-*")))

    def test_realistic_deepseek_response_is_parsed(self):
        payload = {
            "id": "c1f3f4e8-8c4f-4f1d-9d43-8e9cf9e1d6d0",
            "object": "chat.completion",
            "created": 1786305800,
            "model": "deepseek-v4-pro",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": json.dumps(valid_result()),
                        "reasoning_content": None,
                    },
                    "finish_reason": "stop",
                }
            ],
            "usage": {
                "prompt_tokens": 700,
                "completion_tokens": 120,
                "total_tokens": 820,
            },
        }
        opener = FakeOpener(payload)
        evaluate.urlopen = opener
        value, tokens = evaluate.request_evaluation(
            "http://example.test", "evaluation-key", "evaluate this pair"
        )
        self.assertEqual(tokens, 120)
        self.assertEqual(evaluate.validate_structured_output(value)["coverage"]["score"], 4)

    def test_missing_usage_and_truncated_output_fail(self):
        for payload in [
            {"choices": [{"message": {"content": json.dumps(valid_result())}}]},
            {
                "choices": [
                    {
                        "finish_reason": "length",
                        "message": {"content": json.dumps(valid_result())},
                    }
                ],
                "usage": {"completion_tokens": 10},
            },
        ]:
            evaluate.urlopen = FakeOpener(payload)
            with self.assertRaises(evaluate.EvaluationError):
                evaluate.request_evaluation("http://example.test", "evaluation-key", "prompt")

    def test_document_limit_publishes_failed_aggregate(self):
        run = self.make_run(count=evaluate.MAX_DOCUMENTS + 1)
        opener = self.use_valid_opener()
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        self.assertFalse(opener.requests)
        self.assert_failed_without_documents(run)

    def test_input_size_limit_publishes_failed_aggregate(self):
        run = self.make_run(raw_size=evaluate.MAX_INPUT_BYTES + 1)
        opener = self.use_valid_opener()
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        self.assertFalse(opener.requests)
        self.assert_failed_without_documents(run)

    def test_output_token_limit_publishes_failed_aggregate(self):
        run = self.make_run()
        opener = self.use_valid_opener(tokens=evaluate.MAX_OUTPUT_TOKENS + 1)
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        self.assertEqual(len(opener.requests), 1)
        self.assert_failed_without_documents(run)

    def test_timeout_and_http_failures_are_atomic(self):
        for index, error in enumerate([socket.timeout("timed out"), urllib.error.URLError("offline")]):
            run = self.make_run(run_id=f"run-{index}")
            opener = FakeOpener(error=error)
            evaluate.urlopen = opener
            with self.assertRaises(evaluate.EvaluationError):
                evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
            self.assert_failed_without_documents(run)

    def test_total_output_budget_has_no_partial_documents(self):
        run = self.make_run(count=17)
        opener = self.use_valid_opener(tokens=evaluate.MAX_OUTPUT_TOKENS)
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        self.assertEqual(
            len(opener.requests),
            evaluate.MAX_TOTAL_OUTPUT_TOKENS // evaluate.MAX_OUTPUT_TOKENS,
        )
        self.assert_failed_without_documents(run)

    def test_key_isolation_uses_only_evaluator_key(self):
        run = self.make_run()
        opener = self.use_valid_opener()
        with patch.dict(os.environ, {"BO_EVAL_API_KEY": "evaluation-key", "DEEPSEEK_API_KEY": "wrong-key"}):
            self.assertEqual(evaluate.main([str(run)]), 0)
        request, _ = opener.requests[0]
        self.assertEqual(request.headers["Authorization"], "Bearer evaluation-key")

    def test_validation_failure_after_request_has_no_partial_documents(self):
        run = self.make_run(count=2)
        responses = [
            {
                "choices": [{"message": {"content": json.dumps(valid_result())}}],
                "usage": {"completion_tokens": 10},
            },
            {"choices": [{"message": {"content": "not json"}}]},
        ]

        class SequenceOpener(FakeOpener):
            def __call__(self, request, timeout):
                self.requests.append((request, timeout))
                return FakeResponse(responses[len(self.requests) - 1])

        opener = SequenceOpener()
        evaluate.urlopen = opener
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        self.assertEqual(len(opener.requests), 2)
        self.assert_failed_without_documents(run)

    def test_evaluator_key_is_required_even_when_provider_key_exists(self):
        run = self.make_run()
        with patch.dict(os.environ, {"DEEPSEEK_API_KEY": "agent-key"}, clear=True):
            with self.assertRaises(evaluate.EvaluationError):
                evaluate.evaluate(run, api_url="http://example.test")
        self.assert_failed_without_documents(run)

    def assert_failed_without_documents(self, run):
        aggregate_path = run / "evaluation" / "aggregate.json"
        self.assertTrue(aggregate_path.is_file())
        aggregate = json.loads(aggregate_path.read_text())
        self.assertEqual(aggregate["status"], "failed")
        self.assertFalse((run / "evaluation" / "documents").exists())
        self.assertFalse(list(run.glob(".evaluation-*")))


if __name__ == "__main__":
    unittest.main()
