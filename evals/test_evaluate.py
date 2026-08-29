import hashlib
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


def valid_distill_result(score=4):
    return {
        criterion: {"score": score, "evidence": [criterion.replace("_", " ")]}
        for criterion in evaluate.DISTILL_CRITERIA
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

    def make_run(self, count=1, raw_size=None, same_source=False, run_id="run-1", workflow="summarize"):
        run = self.root / run_id
        raw_dir = run / "raw"
        summary_dir = run / "summaries"
        raw_dir.mkdir(parents=True)
        summary_dir.mkdir()
        sources = {}
        for index in range(count):
            filename = f"article-{index}.md"
            source = "https://example.test/article" if same_source else f"https://example.test/{index}"
            raw = "raw source\n" if raw_size is None else "x" * raw_size
            raw_path = raw_dir / filename
            raw_path.write_text(raw, encoding="utf-8")
            summary_filename = f"summary-{index}.md"
            (summary_dir / summary_filename).write_text("summary\n", encoding="utf-8")
            source_record = sources.setdefault(
                source,
                {"source_key": source, "snapshots": [], "summary": None},
            )
            source_record["snapshots"].append(
                {
                    "filename": filename,
                    "written_at": f"2026-08-23T00:00:{index:02d}.000000000Z",
                }
            )
            source_record["summary"] = {
                "filename": summary_filename,
                "derived_from": filename,
                "created_at": f"2026-08-23T00:01:{index:02d}.000000000Z",
                "updated_at": f"2026-08-23T00:01:{index:02d}.000000000Z",
            }
        (run / "state.json").write_text(
            json.dumps({"sources": list(sources.values())}), encoding="utf-8"
        )
        (run / "workflow.txt").write_text(workflow + "\n", encoding="utf-8")
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
        state["sources"][0]["snapshots"][0]["written_at"] = "2026-08-23T00:00:10Z"
        state["sources"][0]["snapshots"][1]["written_at"] = "2026-08-23T00:00:20Z"
        state["sources"][0]["summary"]["derived_from"] = "article-1.md"
        (run / "state.json").write_text(json.dumps(state))
        pairs = evaluate.load_pairs(run)
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["raw_filename"], "article-1.md")

    def test_summary_derived_from_selects_snapshot(self):
        run = self.make_run(count=2, same_source=True)
        state = json.loads((run / "state.json").read_text())
        state["sources"][0]["summary"]["derived_from"] = "article-0.md"
        (run / "state.json").write_text(json.dumps(state))
        pairs = evaluate.load_pairs(run)
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["raw_filename"], "article-0.md")

    def test_rfc3339_timestamps_are_required(self):
        run = self.make_run()
        state = json.loads((run / "state.json").read_text())
        state["sources"][0]["snapshots"][0]["written_at"] = 1
        (run / "state.json").write_text(json.dumps(state))
        with self.assertRaises(evaluate.EvaluationError):
            evaluate.load_pairs(run)

    def test_missing_summary_publishes_partial_scores(self):
        run = self.make_run(count=2)
        state = json.loads((run / "state.json").read_text())
        missing_source = state["sources"][1]["source_key"]
        state["sources"][1]["summary"] = None
        (run / "state.json").write_text(json.dumps(state))
        opener = self.use_valid_opener()

        aggregate = evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")

        self.assertEqual(aggregate["status"], "partial")
        summarize = aggregate["stages"]["summarize"]
        self.assertEqual(summarize["document_count"], 2)
        self.assertEqual(summarize["scored_document_count"], 1)
        self.assertEqual(len(opener.requests), 1)
        self.assertEqual(summarize["missing_summaries"][0]["source_key"], missing_source)
        self.assertEqual(
            len(list((run / "evaluation" / "summarize" / "documents").glob("*.json"))),
            1,
        )

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
        summarize = aggregate["stages"]["summarize"]
        self.assertEqual(summarize["document_count"], 1)
        self.assertEqual(
            summarize["rubric_sha256"],
            __import__("hashlib").sha256(evaluate.RUBRIC_PATH.read_bytes()).hexdigest(),
        )
        self.assertTrue((run / "evaluation" / "aggregate.json").is_file())
        self.assertTrue((run / "evaluation" / "summarize" / "aggregate.json").is_file())
        self.assertEqual(len(list((run / "evaluation" / "summarize" / "documents").glob("*.json"))), 1)
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

    def test_end_to_end_evaluation_publishes_separate_stage_results(self):
        run = self.make_run(count=2, run_id="end-to-end", workflow="end-to-end")
        artifact = "# Shared\n\nSources: [article-0.md](../article-0.md), [article-1.md](../article-1.md)\n"
        (run / "distillations").mkdir()
        (run / "distillations" / "shared.md").write_text(artifact, encoding="utf-8")
        state = json.loads((run / "state.json").read_text())
        state["distillation_documents"] = [{
            "filename": "shared.md",
            "kind": "distillation",
            "derived_from": [
                {
                    "source_key": "https://example.test/0",
                    "kind": "raw",
                    "filename": "article-0.md",
                    "content_digest": hashlib.sha256(b"raw source\n").hexdigest(),
                },
                {
                    "source_key": "https://example.test/1",
                    "kind": "raw",
                    "filename": "article-1.md",
                    "content_digest": hashlib.sha256(b"raw source\n").hexdigest(),
                },
            ],
        }]
        (run / "state.json").write_text(json.dumps(state), encoding="utf-8")

        class SequenceOpener:
            def __init__(self):
                self.requests = []
                self.responses = [valid_result(), valid_result(), valid_distill_result()]

            def __call__(self, request, timeout):
                self.requests.append((request, timeout))
                value = self.responses[len(self.requests) - 1]
                return FakeResponse({
                    "choices": [{"message": {"content": json.dumps(value)}}],
                    "usage": {"completion_tokens": 10},
                })

        opener = SequenceOpener()
        evaluate.urlopen = opener

        aggregate = evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")

        self.assertEqual(aggregate["workflow"], "end-to-end")
        self.assertEqual(aggregate["status"], "success")
        self.assertEqual(set(aggregate["stages"]), {"summarize", "distill"})
        self.assertNotIn("scores", aggregate)
        self.assertEqual(aggregate["stages"]["summarize"]["status"], "success")
        self.assertEqual(aggregate["stages"]["distill"]["status"], "success")
        self.assertEqual(len(opener.requests), 3)
        self.assertTrue((run / "evaluation" / "summarize" / "documents").is_dir())
        self.assertTrue((run / "evaluation" / "distill" / "documents" / "shared.md.json").is_file())

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

    def test_distill_skip_does_not_require_evaluator_key(self):
        run = self.make_run(run_id="distill-skip")
        (run / "workflow.txt").write_text("distill\n", encoding="utf-8")

        aggregate = evaluate.evaluate(run)

        self.assertEqual(aggregate["status"], "skipped")
        self.assertEqual(aggregate["workflow"], "distill")
        self.assertTrue((run / "evaluation" / "distill" / "documents").is_dir())
        self.assertFalse(list((run / "evaluation" / "distill" / "documents").glob("*.json")))

    def test_distill_evaluates_only_recorded_provenance(self):
        run = self.root / "distill-success"
        raw_dir = run / "raw"
        distillation_dir = run / "distillations"
        raw_dir.mkdir(parents=True)
        distillation_dir.mkdir()
        one = "one source\n"
        two = "two source\n"
        (raw_dir / "one.md").write_text(one, encoding="utf-8")
        (raw_dir / "two.md").write_text(two, encoding="utf-8")
        (raw_dir / "extra.md").write_text("must not be sent\n", encoding="utf-8")
        artifact = "# Shared\n\nSources: [one.md](../one.md), [two.md](../two.md)\n"
        (distillation_dir / "shared.md").write_text(artifact, encoding="utf-8")
        state = {
            "sources": [
                {"source_key": "https://example.test/one", "snapshots": [{"filename": "one.md"}]},
                {"source_key": "https://example.test/two", "snapshots": [{"filename": "two.md"}]},
            ],
            "distillation_documents": [{
                "filename": "shared.md",
                "kind": "distillation",
                "derived_from": [
                    {"source_key": "https://example.test/one", "kind": "raw", "filename": "one.md", "content_digest": hashlib.sha256(one.encode()).hexdigest()},
                    {"source_key": "https://example.test/two", "kind": "raw", "filename": "two.md", "content_digest": hashlib.sha256(two.encode()).hexdigest()},
                ],
            }],
        }
        (run / "state.json").write_text(json.dumps(state), encoding="utf-8")
        (run / "workflow.txt").write_text("distill\n", encoding="utf-8")
        opener = FakeOpener({
            "choices": [{"message": {"content": json.dumps(valid_distill_result())}}],
            "usage": {"completion_tokens": 10},
        })
        evaluate.urlopen = opener

        aggregate = evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")

        self.assertEqual(aggregate["status"], "success")
        self.assertEqual(aggregate["workflow"], "distill")
        self.assertEqual(len(opener.requests), 1)
        prompt = opener.requests[0][0].data.decode("utf-8")
        self.assertIn("one source", prompt)
        self.assertIn("two source", prompt)
        self.assertNotIn("must not be sent", prompt)
        output = json.loads((run / "evaluation" / "distill" / "documents" / "shared.md.json").read_text())
        self.assertEqual(len(output["provenance"]), 2)

    def test_distill_digest_changes_publish_failed_aggregate(self):
        run = self.root / "distill-digest"
        raw_dir = run / "raw"
        distillation_dir = run / "distillations"
        raw_dir.mkdir(parents=True)
        distillation_dir.mkdir()
        (raw_dir / "one.md").write_text("changed\n", encoding="utf-8")
        (raw_dir / "two.md").write_text("two\n", encoding="utf-8")
        (distillation_dir / "shared.md").write_text("# Shared\n", encoding="utf-8")
        (run / "state.json").write_text(json.dumps({
            "sources": [
                {"source_key": "https://example.test/one", "snapshots": [{"filename": "one.md"}]},
                {"source_key": "https://example.test/two", "snapshots": [{"filename": "two.md"}]},
            ],
            "distillation_documents": [{
                "filename": "shared.md", "kind": "distillation", "derived_from": [
                    {"source_key": "https://example.test/one", "kind": "raw", "filename": "one.md", "content_digest": "0" * 64},
                    {"source_key": "https://example.test/two", "kind": "raw", "filename": "two.md", "content_digest": hashlib.sha256(b"two\n").hexdigest()},
                ],
            }],
        }), encoding="utf-8")
        (run / "workflow.txt").write_text("distill\n", encoding="utf-8")

        with self.assertRaises(evaluate.EvaluationError):
            evaluate.evaluate(run, api_key="evaluation-key", api_url="http://example.test")
        self.assert_failed_without_documents(run)

    def assert_failed_without_documents(self, run):
        aggregate_path = run / "evaluation" / "aggregate.json"
        self.assertTrue(aggregate_path.is_file())
        aggregate = json.loads(aggregate_path.read_text())
        self.assertEqual(aggregate["status"], "failed")
        workflows = aggregate.get("stages", {})
        self.assertTrue(workflows)
        for workflow in workflows:
            documents = run / "evaluation" / workflow / "documents"
            self.assertTrue(documents.is_dir())
            self.assertFalse(list(documents.glob("*.json")))
        self.assertFalse(list(run.glob(".evaluation-*")))


if __name__ == "__main__":
    unittest.main()
