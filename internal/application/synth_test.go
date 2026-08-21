package application_test

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

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
	state.Raw = append(state.Raw, domain.RawRecord{Filename: raw.Name, URL: "https://example.test/article", WrittenAt: 1})
	generation, err = store.PublishState(context.Background(), state, generation)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.ReplaceSummary(context.Background(), domain.SummaryRef("article.md"), []byte("old summary")); err != nil {
		t.Fatal(err)
	}
	state.Summaries = append(state.Summaries, domain.SummaryRecord{Filename: "article.md", SourceKey: "https://example.test/article", DerivedFrom: "article.md", CreatedAt: 2, UpdatedAt: 3})
	if generation, err = store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("corpus-1", "read_corpus", "{}"),
		toolResponse("read-1", "read_summary", "{\"source_key\":\"https://example.test/article\"}"),
		toolResponse("edit-1", "edit_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"# Summary\\n\\nfact\\n\"}"),
	}}
	result, err := application.Synthesize(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.SynthesisOptions{MaxTurns: 4, MaxToolCalls: 3, MaxToolOutputBytes: 256, MaxResponseTokens: 16, TimeoutSeconds: 5})
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
	if want := "read_corpus,read_document,read_summary,write_summary,edit_summary"; strings.Join(toolNames, ",") != want {
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
	state.Raw = append(state.Raw, domain.RawRecord{Filename: raw.Name, URL: "raw:article.md"})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("bad-1", "read_document", "{\"filename\":\"raw/../article.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"raw:article.md\",\"markdown\":\"summary\\n\"}"),
	}}
	result, err := application.Synthesize(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions())
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
	state.Raw = append(state.Raw, domain.RawRecord{Filename: raw.Name, URL: "https://example.test/article", WrittenAt: 2})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		toolResponse("read-1", "read_document", "{\"filename\":\"article.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"latest fact\\n\"}"),
	}}
	result, err := application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.SynthesisOptions{MaxTurns: 2, MaxToolCalls: 2, MaxToolOutputBytes: 256, MaxResponseTokens: 16, TimeoutSeconds: 5}, []string{"read_document", "write_summary"})
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("SynthesizeWithTools = %#v, %v", result, err)
	}
	if len(provider.requests[0].Tools) != 2 || provider.requests[0].Tools[0].Function.Name != "read_document" || provider.requests[0].Tools[1].Function.Name != "write_summary" {
		t.Fatalf("tools = %#v", provider.requests[0].Tools)
	}
	if provider.requests[1].Messages[len(provider.requests[1].Messages)-1].Content != "# Article\n\nlatest fact\n" {
		t.Fatalf("document output = %#v", provider.requests[1].Messages[len(provider.requests[1].Messages)-1])
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
	state.Raw = append(state.Raw,
		domain.RawRecord{Filename: oldRaw.Name, URL: "https://example.test/article", WrittenAt: 1},
		domain.RawRecord{Filename: newRaw.Name, URL: "https://example.test/article", WrittenAt: 2},
	)
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
	result, err := application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions(), []string{"read_document", "write_summary"})
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
	if len(state.Summaries) != 1 || state.Summaries[0].DerivedFrom != "new.md" {
		t.Fatalf("state = %#v", state)
	}
}

func TestSynthesizeWithToolsValidatesNames(t *testing.T) {
	for _, names := range [][]string{{"read_document", "read_document"}, {"unknown"}} {
		if _, err := application.SynthesizeWithTools(context.Background(), nil, "notes", nil, application.DefaultSynthesisOptions(), names); err == nil {
			t.Fatalf("tool names accepted: %v", names)
		}
	}
}
