package bo_test

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/skillicinski/bo"
)

type fakeProvider struct {
	responses []bo.CompletionResponse
	requests  []bo.CompletionRequest
}

func (p *fakeProvider) Complete(_ context.Context, request bo.CompletionRequest) (bo.CompletionResponse, error) {
	p.requests = append(p.requests, request)
	response := p.responses[0]
	p.responses = p.responses[1:]
	return response, nil
}

func toolResponse(id, name, arguments string) bo.CompletionResponse {
	return bo.CompletionResponse{Message: bo.ChatMessage{
		Role:      "assistant",
		Content:   nil,
		ToolCalls: []bo.ToolCall{{ID: id, Type: "function", Function: bo.ToolFunction{Name: name, Arguments: arguments}}},
	}}
}

func TestAgentReplaysToolMessagesAndUpsertsSummary(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("# Article\n\nfact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Raw = append(state.Raw, bo.RawRecord{Filename: raw.Name, URL: "https://example.test/article", WrittenAt: 1})
	generation, err = store.PublishState(context.Background(), state, generation)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.ReplaceSummary(context.Background(), bo.SummaryRef("article.md"), []byte("old summary")); err != nil {
		t.Fatal(err)
	}
	state.Summaries = append(state.Summaries, bo.SummaryRecord{Filename: "article.md", SourceKey: "https://example.test/article", DerivedFrom: "article.md", CreatedAt: 2, UpdatedAt: 3})
	if generation, err = store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []bo.CompletionResponse{
		toolResponse("state-1", "read_state", "{}"),
		toolResponse("read-1", "read_summary", "{\"source_key\":\"https://example.test/article\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"https://example.test/article\",\"markdown\":\"# Summary\\n\\nfact\\n\"}"),
		{Message: bo.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	written, err := bo.RunAgent(context.Background(), filepath.Dir(target), target, store, provider, bo.AgentConfig{MaxTurns: 4, MaxToolCalls: 3, MaxToolOutputBytes: 256, MaxResponseTokens: 16, TimeoutSeconds: 5})
	if err != nil || written != 1 {
		t.Fatalf("RunAgent = %d, %v", written, err)
	}
	if len(provider.requests) != 4 {
		t.Fatalf("requests = %d", len(provider.requests))
	}
	if len(provider.requests[1].Messages) != 4 || provider.requests[1].Messages[2].ToolCalls[0].ID != "state-1" || provider.requests[1].Messages[3].ToolCallID != "state-1" {
		t.Fatalf("tool replay = %#v", provider.requests[1].Messages)
	}
	data, err := store.ReadDocument(context.Background(), bo.SummaryRef("article.md"))
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "# Summary\n\nfact\n" {
		t.Fatalf("summary = %q", data)
	}
}

func TestAgentToolsRejectRawEscape(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("fact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Raw = append(state.Raw, bo.RawRecord{Filename: raw.Name, URL: "raw:article.md"})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []bo.CompletionResponse{
		toolResponse("state-1", "read_state", "{}"),
		toolResponse("bad-1", "bash", "{\"command\":\"cat ../article.md\"}"),
		toolResponse("write-1", "write_summary", "{\"source_key\":\"raw:article.md\",\"markdown\":\"summary\\n\"}"),
		{Message: bo.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	if _, err := bo.RunAgent(context.Background(), filepath.Dir(target), target, store, provider, bo.DefaultAgentConfig()); err != nil {
		t.Fatal(err)
	}
}

func TestAgentOptions(t *testing.T) {
	config, err := bo.ParseAgentOptions([]string{"--max-turns", "2", "--max-tool-calls", "3", "--max-tool-output-bytes", "4", "--max-response-tokens", "5", "--timeout-seconds", "6"})
	if err != nil {
		t.Fatal(err)
	}
	if config.MaxTurns != 2 || config.MaxToolCalls != 3 || config.MaxToolOutputBytes != 4 || config.MaxResponseTokens != 5 || config.TimeoutSeconds != 6 {
		t.Fatalf("config = %#v", config)
	}
	if _, err := bo.ParseAgentOptions([]string{"--unknown", "1"}); err == nil {
		t.Fatal("unknown option succeeded")
	}
	if _, err := bo.ParseAgentOptions([]string{"--max-turns"}); err == nil {
		t.Fatal("missing value succeeded")
	}
	if _, err := bo.ParseAgentOptions([]string{"--max-turns", "zero"}); err == nil {
		t.Fatal("zero succeeded")
	}
}
