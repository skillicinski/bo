package application_test

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/storage/local"
)

func seededStore(t *testing.T) (*local.Store, string) {
	t.Helper()
	home := t.TempDir()
	target, err := local.Seed(home, stringPtr("notes"))
	if err != nil {
		t.Fatal(err)
	}
	store, err := local.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	return store, target
}

func stringPtr(value string) *string { return &value }

func operationOptionsFor(target string) application.OperationOptions {
	return application.OperationOptions{Log: local.NewOperationLog(filepath.Dir(filepath.Dir(target))), Actor: "test"}
}

type fakeProvider struct {
	responses []agent.CompletionResponse
	requests  []agent.CompletionRequest
}

func (p *fakeProvider) Complete(_ context.Context, request agent.CompletionRequest) (agent.CompletionResponse, error) {
	p.requests = append(p.requests, request)
	response := p.responses[0]
	p.responses = p.responses[1:]
	return response, nil
}

func toolResponse(id, name, arguments string) agent.CompletionResponse {
	return agent.CompletionResponse{Message: agent.ChatMessage{
		Role:      "assistant",
		Content:   nil,
		ToolCalls: []agent.ToolCall{{ID: id, Type: "function", Function: agent.ToolFunction{Name: name, Arguments: arguments}}},
	}}
}

func TestSynthesizeReplaysToolMessagesAndUpsertsSummary(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("# Article\n\nfact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = append(state.Sources, domain.SourceRecord{SourceKey: "https://example.test/article", Snapshots: []domain.RawRecord{{Filename: raw.Name, WrittenAt: time.Unix(1, 0).UTC()}}})
	generation, err = store.PublishState(context.Background(), state, generation)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.ReplaceSummary(context.Background(), domain.SummaryRef("article.md"), []byte("old summary")); err != nil {
		t.Fatal(err)
	}
	state.Sources[0].Summary = &domain.SummaryRecord{Filename: "article.md", DerivedFrom: "article.md", CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC()}
	if generation, err = store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("corpus-1", "read_corpus", "{}"),
		toolResponse("read-1", "read_summary", "{\"source_key\":\"https://example.test/article\"}"),
		toolResponse("edit-1", "edit_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"# Summary\\n\\nfact\\n\"}"),
	}}
	result, err := application.Synthesize(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.SynthesisOptions{MaxTurns: 4, MaxToolCalls: 3, MaxToolOutputBytes: 256, MaxResponseTokens: 16, TimeoutSeconds: 5}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("Synthesize = %#v, %v", result, err)
	}
	if len(provider.requests) != 3 {
		t.Fatalf("requests = %d", len(provider.requests))
	}
	toolNames := make([]string, 0, len(provider.requests[0].Tools))
	for _, tool := range provider.requests[0].Tools {
		toolNames = append(toolNames, tool.Function.Name)
	}
	if want := "read_corpus,read_logs,read_document,read_summary,write_summary,edit_summary"; strings.Join(toolNames, ",") != want {
		t.Fatalf("tools = %v", toolNames)
	}
	if len(provider.requests[1].Messages) != 4 || provider.requests[1].Messages[2].ToolCalls[0].ID != "corpus-1" || provider.requests[1].Messages[3].ToolCallID != "corpus-1" {
		t.Fatalf("tool replay = %#v", provider.requests[1].Messages)
	}
	data, err := store.ReadDocument(context.Background(), domain.SummaryRef("article.md"))
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "# Summary\n\nfact\n" {
		t.Fatalf("summary = %q", data)
	}
	state, _, err = store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if state.Sources[0].Summary == nil || !state.Sources[0].Summary.UpdatedAt.After(time.Unix(3, 0).UTC()) {
		t.Fatalf("summary timestamp did not advance: %#v", state.Sources[0].Summary)
	}
}

func TestSynthesizeToolsRejectRawEscape(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("fact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = append(state.Sources, domain.SourceRecord{SourceKey: "raw:article.md", Snapshots: []domain.RawRecord{{Filename: raw.Name, WrittenAt: time.Unix(1, 0).UTC()}}})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("bad-1", "read_document", "{\"filename\":\"raw/../article.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"raw:article.md\",\"markdown\":\"summary\\n\"}"),
	}}
	result, err := application.Synthesize(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions(), operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("Synthesize = %#v, %v", result, err)
	}
}

func TestSynthesizeWithReducedToolSet(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("# Article\n\nlatest fact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = append(state.Sources, domain.SourceRecord{SourceKey: "https://example.test/article", Snapshots: []domain.RawRecord{{Filename: raw.Name, WrittenAt: time.Unix(2, 0).UTC()}}})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-1", "read_document", "{\"filename\":\"article.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"latest fact\\n\"}"),
	}}
	result, err := application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.SynthesisOptions{MaxTurns: 2, MaxToolCalls: 2, MaxToolOutputBytes: 256, MaxResponseTokens: 16, TimeoutSeconds: 5}, []string{"read_document", "write_summary"}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("SynthesizeWithTools = %#v, %v", result, err)
	}
	if len(provider.requests[0].Tools) != 2 || provider.requests[0].Tools[0].Function.Name != "read_document" || provider.requests[0].Tools[1].Function.Name != "write_summary" {
		t.Fatalf("tools = %#v", provider.requests[0].Tools)
	}
	if provider.requests[1].Messages[len(provider.requests[1].Messages)-1].Content != "# Article\n\nlatest fact\n" {
		t.Fatalf("document output = %#v", provider.requests[1].Messages[len(provider.requests[1].Messages)-1])
	}
	page, err := operationOptionsFor(target).Log.Read(context.Background(), "notes", 0, 20)
	if err != nil {
		t.Fatal(err)
	}
	foundWrite := false
	for _, operation := range page.Entries {
		if operation.Command == application.CommandWriteSummary {
			foundWrite = true
			if _, ok := operation.Details["markdown"]; ok {
				t.Fatal("write_summary log contains Markdown")
			}
			if operation.Details["source_key"] != "https://example.test/article" {
				t.Fatalf("write_summary details = %#v", operation.Details)
			}
		}
	}
	if !foundWrite {
		t.Fatal("write_summary event missing")
	}
}

func TestSynthesizeRegistersUntrackedRawDocument(t *testing.T) {
	store, target := seededStore(t)
	if _, err := store.CreateRaw(context.Background(), "article.md", []byte("fact\n")); err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = []domain.SourceRecord{{SourceKey: "raw:article.md"}}
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-1", "read_document", `{"filename":"article.md"}`),
		toolResponse("write-1", "write_summary", `{"source_key":"raw:article.md","markdown":"summary\n"}`),
	}}
	result, err := application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions(), []string{"read_document", "write_summary"}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("SynthesizeWithTools = %#v, %v", result, err)
	}
	state, _, err = store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 1 || len(state.Sources[0].Snapshots) != 1 || state.Sources[0].Summary == nil {
		t.Fatalf("state = %#v", state)
	}
}

func TestSynthesizeSelectsNewestSnapshotAndPreservesRaw(t *testing.T) {
	store, target := seededStore(t)
	oldRaw, err := store.CreateRaw(context.Background(), "old.md", []byte("old\n"))
	if err != nil {
		t.Fatal(err)
	}
	newRaw, err := store.CreateRaw(context.Background(), "new.md", []byte("new\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = append(state.Sources, domain.SourceRecord{SourceKey: "https://example.test/article", Snapshots: []domain.RawRecord{
		{Filename: oldRaw.Name, WrittenAt: time.Unix(1, 0).UTC()},
		{Filename: newRaw.Name, WrittenAt: time.Unix(2, 0).UTC()},
	}})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	oldBefore, err := store.ReadDocument(context.Background(), domain.RawRef(oldRaw.Name))
	if err != nil {
		t.Fatal(err)
	}
	newBefore, err := store.ReadDocument(context.Background(), domain.RawRef(newRaw.Name))
	if err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-1", "read_document", "{\"filename\":\"new.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"new\\n\"}"),
	}}
	result, err := application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions(), []string{"read_document", "write_summary"}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("SynthesizeWithTools = %#v, %v", result, err)
	}
	oldAfter, err := store.ReadDocument(context.Background(), domain.RawRef(oldRaw.Name))
	if err != nil {
		t.Fatal(err)
	}
	newAfter, err := store.ReadDocument(context.Background(), domain.RawRef(newRaw.Name))
	if err != nil {
		t.Fatal(err)
	}
	if string(oldBefore) != string(oldAfter) || string(newBefore) != string(newAfter) {
		t.Fatal("raw document changed")
	}
	state, _, err = store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 1 || state.Sources[0].Summary == nil || state.Sources[0].Summary.DerivedFrom != "new.md" {
		t.Fatalf("state = %#v", state)
	}
}

func TestSynthesizeWithToolsValidatesNames(t *testing.T) {
	for _, names := range [][]string{{"read_document", "read_document"}, {"unknown"}} {
		if _, err := application.SynthesizeWithTools(context.Background(), nil, "notes", nil, application.DefaultSynthesisOptions(), names, application.OperationOptions{Log: local.NewOperationLog(t.TempDir())}); err == nil {
			t.Fatalf("tool names accepted: %v", names)
		}
	}
}
