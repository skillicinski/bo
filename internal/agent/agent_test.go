package agent_test

import (
	"context"
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
