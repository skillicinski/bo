package agent_test

import (
	"context"
	"errors"
	"testing"

	"github.com/skillicinski/bo/internal/agent"
)

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

func TestRuntimeUsesConfiguredToolSet(t *testing.T) {
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "call-1", Type: "function", Function: agent.ToolFunction{Name: "echo"},
		}}}},
		{Message: agent.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	called := false
	runtime := agent.Runtime{
		Provider: provider,
		Tools: []agent.Tool{{
			Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "echo"}},
			Execute: func(context.Context, agent.ToolCall) (string, error) {
				called = true
				return "ok", nil
			},
		}},
	}
	result, err := runtime.Run(context.Background(), []agent.ChatMessage{{Role: "user", Content: "run"}}, agent.Options{
		MaxTurns: 2, MaxToolCalls: 1, MaxToolOutputBytes: 16, MaxResponseTokens: 8,
	})
	if err != nil {
		t.Fatal(err)
	}
	if !called || len(provider.requests) != 2 {
		t.Fatalf("tool called = %t, requests = %d", called, len(provider.requests))
	}
	if len(provider.requests[0].Tools) != 1 || provider.requests[0].Tools[0].Function.Name != "echo" {
		t.Fatalf("tools = %#v", provider.requests[0].Tools)
	}
	if len(result.Messages) != 4 || result.Message.Content != "done" {
		t.Fatalf("result = %#v", result)
	}
}

func TestRuntimeStopsAfterCompletedToolBatch(t *testing.T) {
	provider := &fakeProvider{responses: []agent.CompletionResponse{{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
		ID: "write-1", Type: "function", Function: agent.ToolFunction{Name: "write"},
	}}}, Usage: &agent.TokenUsage{PromptTokens: 3, CompletionTokens: 2, TotalTokens: 5}}}}
	done := false
	runtime := agent.Runtime{
		Provider: provider,
		Done:     func() bool { return done },
		Tools: []agent.Tool{{
			Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "write"}},
			Execute: func(context.Context, agent.ToolCall) (string, error) {
				done = true
				return "ok", nil
			},
		}},
	}
	result, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 2, MaxToolCalls: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(provider.requests) != 1 || result.Turns != 1 || result.ToolCalls != 1 {
		t.Fatalf("result = %#v, requests = %d", result, len(provider.requests))
	}
	if result.Usage == nil || result.Usage.TotalTokens != 5 || result.Duration <= 0 {
		t.Fatalf("metrics = %#v", result.Metrics)
	}
}

type failingProvider struct {
	response agent.CompletionResponse
	err      error
	calls    int
}

func (p *failingProvider) Complete(_ context.Context, _ agent.CompletionRequest) (agent.CompletionResponse, error) {
	p.calls++
	if p.calls > 1 {
		return agent.CompletionResponse{}, p.err
	}
	return p.response, nil
}

func TestRuntimeReturnsPartialMetricsOnFailure(t *testing.T) {
	provider := &failingProvider{
		response: agent.CompletionResponse{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "read-1", Type: "function", Function: agent.ToolFunction{Name: "read"},
		}}}, Usage: &agent.TokenUsage{PromptTokens: 7, CompletionTokens: 4, TotalTokens: 11}},
		err: errors.New("provider stopped"),
	}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: "read"}},
		Execute:    func(context.Context, agent.ToolCall) (string, error) { return "ok", nil },
	}}}
	result, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 3, MaxToolCalls: 3})
	if err == nil || result.Turns != 2 || result.ToolCalls != 1 || result.Duration <= 0 {
		t.Fatalf("result = %#v, err = %v", result, err)
	}
	if result.Usage == nil || result.Usage.TotalTokens != 11 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}
