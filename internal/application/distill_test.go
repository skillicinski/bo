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
		toolResponse("write", "write_distillation", `{"title":"Shared facts","introduction":"Both sources report related facts.","sections":[{"heading":"Common facts","paragraph":"The sources describe the same subject.","bullets":["One source reports one fact.","The other source reports another fact."],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || result.Skipped || result.Filename != "shared-facts.md" {
		t.Fatalf("Distill = %#v, %v", result, err)
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

func TestDistillRejectsUnreadReferencesAndAllowsSkip(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/one", "one.md", time.Unix(1, 0).UTC(), []byte("one fact\n"))
	commitRaw(t, store, "https://example.test/two", "two.md", time.Unix(2, 0).UTC(), []byte("two fact\n"))
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-one", "read_document", `{"filename":"one.md"}`),
		toolResponse("write", "write_distillation", `{"title":"Shared facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["one","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`),
		toolResponse("skip", "skip_distill", `{"reason":"The second source was not read."}`),
	}}
	result, err := application.Distill(context.Background(), store, provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || !result.Skipped || result.Reason == "" || result.Filename != "" {
		t.Fatalf("Distill = %#v, %v", result, err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.DistillationDocuments) != 0 {
		t.Fatalf("state = %#v, %v", state, err)
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
		toolResponse("write", "write_distillation", `{"title":"Current facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["new","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one-new.md"},{"source_key":"https://example.test/two","kind":"summary","filename":"two-summary.md"}]}]}`),
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
