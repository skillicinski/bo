package application_test

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func TestDistillCreatesOneCrossSourceDocument(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one fact\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two fact\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("read-two", "read_document", `{"filename":"two.md"}`),
		toolResponse("write", "write_distillation", `{"topic":"shared-facts","title":"Shared facts","introduction":"Both sources report related facts.","sections":[{"heading":"Common facts","paragraph":"The sources describe the same subject.","bullets":["One source reports one fact.","The other source reports another fact."],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
		toolResponse("skip", "skip_distill", `{"reason":"No other supported themes remain."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.SynthesisOptions{MaxTurns: 4, MaxToolCalls: 4, MaxToolOutputBytes: 4096, MaxResponseTokens: 32, RuntimeTimeoutSeconds: 5}, operationOptionsFor(target))
	if err != nil || result.Skipped || result.Filename != "shared-facts.md" {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	if len(result.Telemetry) != 1 || result.Telemetry[0].Workflow != "distill" || result.Telemetry[0].TerminalReason != "done" || result.Telemetry[0].TerminalDetail != "No other supported themes remain." || len(result.Telemetry[0].ToolCalls) != 4 {
		t.Fatalf("Distill telemetry = %#v", result.Telemetry)
	}
	if result.Telemetry[0].ToolCalls[0].Name != "read_document" || result.Telemetry[0].ToolCalls[0].ArgumentsPreview != `{"filename":"one.md"}` || result.Telemetry[0].ToolCalls[3].Name != "skip_distill" {
		t.Fatalf("Distill tool telemetry = %#v", result.Telemetry[0].ToolCalls)
	}
	if len(provider.deadlines) != 4 {
		t.Fatalf("runtime deadlines = %#v", provider.deadlines)
	}
	for _, deadline := range provider.deadlines[1:] {
		if !deadline.Equal(provider.deadlines[0]) {
			t.Fatalf("runtime deadlines differ: %#v", provider.deadlines)
		}
	}
	data, err := store.ReadDocument(context.Background(), domain.DistillationRef(result.Filename))
	if err != nil || !strings.Contains(string(data), "[one.md](../one.md)") || !strings.Contains(string(data), "## Sources") {
		t.Fatalf("distill document = %q, %v", data, err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.DistillationDocuments) != 1 || len(state.DistillationDocuments[0].DerivedFrom) != 2 {
		t.Fatalf("state = %#v, %v", state, err)
	}
}

func TestDistillRequiresEvidenceBeforeSkip(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one fact\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two fact\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("write", "write_distillation", `{"topic":"shared-facts","title":"Shared facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["one","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
		toolResponse("premature-skip", "skip_distill", `{"reason":"The second source was not read."}`),
		toolResponse("read-two", "read_document", `{"filename":"two.md"}`),
		toolResponse("skip", "skip_distill", `{"reason":"No other supported themes remain."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || !result.Skipped || result.Reason == "" || result.Filename != "" || len(provider.requests) != 5 {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	if !strings.Contains(fmt.Sprint(provider.requests[3].Messages), "at least two distinct source identities") {
		t.Fatalf("premature skip was accepted: %#v", provider.requests)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.DistillationDocuments) != 0 {
		t.Fatalf("state = %#v, %v", state, err)
	}
}

func TestDistillRequiresTwoCurrentSummaryReadsBeforeSkip(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one fact\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two fact\n"))
	commitSummary(t, store, application.SummaryCommit{
		SourceKey: "https://example.test/one", Filename: "one-summary.md", DerivedFrom: "one.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(3, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("one summary\n"),
	})
	commitSummary(t, store, application.SummaryCommit{
		SourceKey: "https://example.test/two", Filename: "two-summary.md", DerivedFrom: "two.md",
		RawWrittenAt: time.Unix(2, 0).UTC(), CreatedAt: time.Unix(4, 0).UTC(), UpdatedAt: time.Unix(4, 0).UTC(), Contents: []byte("two summary\n"),
	})
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-one-summary", "read_summary", `{"source_key":"https://example.test/one"}`),
		toolResponse("premature-skip", "skip_distill", `{"reason":"No theme found."}`),
		toolResponse("read-two-summary", "read_summary", `{"source_key":"https://example.test/two"}`),
		toolResponse("skip", "skip_distill", `{"reason":"No other supported themes remain."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || !result.Skipped || len(provider.requests) != 4 {
		t.Fatalf("Distill = %#v, requests = %d, error = %v", result, len(provider.requests), err)
	}
	if !strings.Contains(fmt.Sprint(provider.requests[0].Messages), "one-summary.md") || !strings.Contains(fmt.Sprint(provider.requests[0].Messages), "read current summaries for at least two distinct source identities") {
		t.Fatalf("summary survey instruction is missing: %#v", provider.requests[0])
	}
	if !strings.Contains(fmt.Sprint(provider.requests[2].Messages), "current summaries") {
		t.Fatalf("summary guard was not reported: %#v", provider.requests)
	}
}

func TestDistillExcludesStaleSummariesAndUsesCurrentSummaryInputs(t *testing.T) {
	store, target := seededStore(t)
	oldRaw := commitRaw(t, store, "https://example.test/one", "one-old.md", time.Unix(1, 0).UTC(), []byte("old\n"))
	commitRaw(t, store, "https://example.test/one", "one-new.md", time.Unix(2, 0).UTC(), []byte("new\n"))
	commitSummary(t, store, application.SummaryCommit{
		SourceKey: "https://example.test/one", Filename: "one-summary.md", DerivedFrom: oldRaw.Name,
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(3, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("stale summary\n"),
	})
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(4, 0).UTC(), []byte("two\n"))
	commitSummary(t, store, application.SummaryCommit{
		SourceKey: "https://example.test/two", Filename: "two-summary.md", DerivedFrom: "two.md",
		RawWrittenAt: time.Unix(4, 0).UTC(), CreatedAt: time.Unix(5, 0).UTC(), UpdatedAt: time.Unix(5, 0).UTC(), Contents: []byte("current summary\n"),
	})
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("stale-summary", "read_summary", `{"source_key":"https://example.test/one"}`),
		toolResponse("read-new", "read_document", `{"filename":"one-new.md"}`),
		toolResponse("current-summary", "read_summary", `{"source_key":"https://example.test/two"}`),
		toolResponse("write", "write_distillation", `{"topic":"current-facts","title":"Current facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["new","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one-new.md"},{"source_key":"https://example.test/two","kind":"summary","filename":"two-summary.md"}]}]}`),
		toolResponse("skip", "skip_distill", `{"reason":"No other supported themes remain."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || result.Filename != "current-facts.md" {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	if len(provider.requests) < 2 || !strings.Contains(fmt.Sprint(provider.requests[1].Messages), "no current summary exists") {
		t.Fatalf("stale summary was exposed: %#v", provider.requests)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.DistillationDocuments) != 1 {
		t.Fatalf("state = %#v, %v", state, err)
	}
	inputs := state.DistillationDocuments[0].DerivedFrom
	if len(inputs) != 2 || inputs[0].Filename != "one-new.md" || inputs[1].Filename != "two-summary.md" {
		t.Fatalf("inputs = %#v", inputs)
	}
}

func TestDistillRequiresTerminalTool(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{{Message: agent.ChatMessage{Role: "assistant"}}}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if !internalerrors.IsKind(err, internalerrors.KindProviderMalformed) || result.Filename != "" || result.Skipped {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	state, _, stateErr := store.ReadState(context.Background())
	if stateErr != nil || len(state.DistillationDocuments) != 0 {
		t.Fatalf("state = %#v, %v", state, stateErr)
	}
}

func TestDistillWithToolsRejectsMissingTerminalTool(t *testing.T) {
	_, err := application.DistillWithTools(context.Background(), nil, nil, application.DefaultSynthesisOptions(), []string{"read_document"}, application.OperationOptions{})
	if err == nil || !strings.Contains(err.Error(), "skip_distill") {
		t.Fatalf("missing terminal tool error = %v", err)
	}
}

func TestDistillRejectsMissingTopic(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one fact\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two fact\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("read-two", "read_document", `{"filename":"two.md"}`),
		toolResponse("write", "write_distillation", `{"title":"Shared facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["one","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
		toolResponse("skip", "skip_distill", `{"reason":"The topic was missing."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || !result.Skipped || len(provider.requests) != 4 {
		t.Fatalf("Distill = %#v, requests = %d, error = %v", result, len(provider.requests), err)
	}
	if !strings.Contains(fmt.Sprint(provider.requests[3].Messages), "topic must be non-empty") {
		t.Fatalf("missing topic was accepted: %#v", provider.requests)
	}
}

func TestDistillSkipsEquivalentInputs(t *testing.T) {
	store, target := seededStore(t)
	one := []byte("one fact\n")
	two := []byte("two fact\n")
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), one)
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), two)
	commitExistingDistillation(t, store, "shared.md", time.Unix(3, 0).UTC(), []domain.DistillationInput{
		{SourceKey: "https://example.test/one", Kind: domain.DocumentKindRaw, Filename: "one.md", ContentDigest: application.NewRevision(one).String()},
		{SourceKey: "https://example.test/two", Kind: domain.DocumentKindRaw, Filename: "two.md", ContentDigest: application.NewRevision(two).String()},
	}, []byte("old distillation\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("read-two", "read_document", `{"filename":"two.md"}`),
		toolResponse("write", "write_distillation", `{"topic":"shared-facts","title":"Shared facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["one","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
		toolResponse("skip", "skip_distill", `{"reason":"No other supported themes remain."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || !result.Skipped || result.Filename != "" || len(result.Committed) != 0 {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.DistillationDocuments) != 1 || len(provider.requests) != 4 {
		t.Fatalf("state = %#v, requests = %d, error = %v", state, len(provider.requests), err)
	}
}

func TestDistillEditsExistingDocument(t *testing.T) {
	store, target := seededStore(t)
	one := []byte("one fact\n")
	two := []byte("two fact\n")
	three := []byte("three fact\n")
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), one)
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), two)
	commitRaw(t, store, "https://example.test/three", "three.md", time.Unix(3, 0).UTC(), three)
	createdAt := time.Unix(4, 0).UTC()
	commitExistingDistillation(t, store, "shared.md", createdAt, []domain.DistillationInput{
		{SourceKey: "https://example.test/one", Kind: domain.DocumentKindRaw, Filename: "one.md", ContentDigest: application.NewRevision(one).String()},
		{SourceKey: "https://example.test/two", Kind: domain.DocumentKindRaw, Filename: "two.md", ContentDigest: application.NewRevision(two).String()},
	}, []byte("old distillation\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-existing", "read_distillation", `{"filename":"shared.md"}`),
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("read-three", "read_document", `{"filename":"three.md"}`),
		toolResponse("edit", "edit_distillation", `{"topic":"shared-facts","filename":"shared.md","title":"Shared facts","introduction":"Updated facts.","sections":[{"heading":"Facts","paragraph":"The sources report related facts.","bullets":["One reports one fact.","Three reports another fact."],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/three","kind":"raw","filename":"three.md"}]}]}`),
		toolResponse("skip", "skip_distill", `{"reason":"No other supported themes remain."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || result.Skipped || result.Filename != "shared.md" || len(result.Committed) != 1 {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.DistillationDocuments) != 1 || !state.DistillationDocuments[0].CreatedAt.Equal(createdAt) || len(state.DistillationDocuments[0].DerivedFrom) != 2 {
		t.Fatalf("state = %#v, %v", state, err)
	}
	contents, err := store.ReadDocument(context.Background(), domain.DistillationRef("shared.md"))
	if err != nil || !strings.Contains(string(contents), "Updated facts.") {
		t.Fatalf("contents = %q, %v", contents, err)
	}
}

func TestDistillRejectsOversizedEditCandidate(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one fact\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two fact\n"))
	commitRaw(t, store, "https://example.test/three", "three.md", time.Unix(3, 0).UTC(), []byte("three fact\n"))
	oldContents := []byte(strings.Repeat("old content ", 200))
	commitExistingDistillation(t, store, "shared.md", time.Unix(4, 0).UTC(), []domain.DistillationInput{
		{SourceKey: "https://example.test/one", Kind: domain.DocumentKindRaw, Filename: "one.md", ContentDigest: application.NewRevision([]byte("one fact\n")).String()},
		{SourceKey: "https://example.test/three", Kind: domain.DocumentKindRaw, Filename: "three.md", ContentDigest: application.NewRevision([]byte("three fact\n")).String()},
	}, oldContents)
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-existing", "read_distillation", `{"filename":"shared.md"}`),
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("read-two", "read_document", `{"filename":"two.md"}`),
		toolResponse("edit", "edit_distillation", `{"topic":"shared-facts","filename":"shared.md","title":"Shared facts","introduction":"Updated facts.","sections":[{"heading":"Facts","paragraph":"The sources report related facts.","bullets":["One reports one fact.","Two reports another fact."],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.SynthesisOptions{MaxTurns: 4, MaxToolCalls: 4, MaxToolOutputBytes: 1024, MaxResponseTokens: 64, RuntimeTimeoutSeconds: 5}, operationOptionsFor(target))
	if err == nil || result.Filename != "" {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	if len(provider.requests) < 2 || !strings.Contains(fmt.Sprint(provider.requests[1].Messages), "too large to edit safely") {
		t.Fatalf("oversized candidate was not rejected: %#v", provider.requests)
	}
	contents, err := store.ReadDocument(context.Background(), domain.DistillationRef("shared.md"))
	if err != nil || string(contents) != string(oldContents) {
		t.Fatalf("contents = %q, %v", contents, err)
	}
}

func commitExistingDistillation(t *testing.T, store application.Workspace, filename string, timestamp time.Time, inputs []domain.DistillationInput, contents []byte) {
	t.Helper()
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitDistillation(context.Background(), application.DistillationCommit{
		Kind: domain.DocumentKindDistillation, Filename: filename, Topic: "shared-facts", CreatedAt: timestamp, UpdatedAt: timestamp,
		DerivedFrom: inputs, Contents: contents,
		Event: domain.Operation{OperationID: "test-distillation-" + filename, Attempt: 1, Timestamp: timestamp.Format(time.RFC3339Nano), Actor: "test", Command: domain.CommandWriteDistillation, Outcome: domain.OutcomeCommitted, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindDistillation, Filename: filename}},
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
}
