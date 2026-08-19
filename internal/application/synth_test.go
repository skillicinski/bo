package application_test

import (
	"context"
	"path/filepath"
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
		toolResponse("state-1", "read_state", "{}"),
		toolResponse("read-1", "read_summary", "{\"source_key\":\"https://example.test/article\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"# Summary\\n\\nfact\\n\"}"),
		{Message: agent.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	written, err := application.Synthesize(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.SynthesisOptions{MaxTurns: 4, MaxToolCalls: 3, MaxToolOutputBytes: 256, MaxResponseTokens: 16, TimeoutSeconds: 5})
	if err != nil || written != 1 {
		t.Fatalf("Synthesize = %d, %v", written, err)
	}
	if len(provider.requests) != 4 {
		t.Fatalf("requests = %d", len(provider.requests))
	}
	if len(provider.requests[1].Messages) != 4 || provider.requests[1].Messages[2].ToolCalls[0].ID != "state-1" || provider.requests[1].Messages[3].ToolCallID != "state-1" {
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
		toolResponse("state-1", "read_state", "{}"),
		toolResponse("bad-1", "bash", "{\"command\":\"cat ../article.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"raw:article.md\",\"markdown\":\"summary\\n\"}"),
		{Message: agent.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	if _, err := application.Synthesize(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions()); err != nil {
		t.Fatal(err)
	}
}
