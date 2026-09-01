package agent_test

import (
	"context"
	"errors"
	"testing"

	"github.com/skillicinski/bo/internal/agent"
	internalerrors "github.com/skillicinski/bo/internal/errors"
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
			ID: "call-1", Function: agent.ToolFunction{Name: "echo"},
		}}}},
		{Message: agent.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	called := false
	runtime := agent.Runtime{
		Provider: provider,
		Tools: []agent.Tool{{
			Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "echo"}},
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

func TestRuntimeRecordsToolTelemetry(t *testing.T) {
	arguments := `{"filename":"one.md"}`
	output := "document contents"
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "read-1", Function: agent.ToolFunction{Name: "read_document", Arguments: arguments},
		}}}},
		{Message: agent.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "read_document"}},
		Execute:    func(context.Context, agent.ToolCall) (string, error) { return output, nil },
	}}}
	result, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 2, MaxToolCalls: 1, MaxToolOutputBytes: 8})
	if err != nil {
		t.Fatal(err)
	}
	if result.Telemetry.TerminalReason != "assistant_message" || len(result.Telemetry.ToolCalls) != 1 {
		t.Fatalf("telemetry = %#v", result.Telemetry)
	}
	call := result.Telemetry.ToolCalls[0]
	if call.Turn != 1 || call.Index != 1 || call.Name != "read_document" || call.ArgumentsPreview != arguments {
		t.Fatalf("tool telemetry = %#v", call)
	}
	if call.ArgumentsBytes != len(arguments) || call.ArgumentsSHA256 == "" || call.OutputSHA256 == "" {
		t.Fatalf("tool hashes = %#v", call)
	}
	if call.OutputBytes != len(output) || call.OutputReturnedBytes != 8 || !call.OutputTruncated {
		t.Fatalf("tool output telemetry = %#v", call)
	}
}

func TestRuntimeStopsAfterCompletedToolBatch(t *testing.T) {
	provider := &fakeProvider{responses: []agent.CompletionResponse{{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
		ID: "write-1", Function: agent.ToolFunction{Name: "write"},
	}}}, Usage: &agent.TokenUsage{PromptTokens: 3, CompletionTokens: 2, TotalTokens: 5}}}}
	done := false
	runtime := agent.Runtime{
		Provider: provider,
		Done:     func() bool { return done },
		Tools: []agent.Tool{{
			Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}},
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

type responseErrorProvider struct {
	response agent.CompletionResponse
	err      error
}

func (p responseErrorProvider) Complete(context.Context, agent.CompletionRequest) (agent.CompletionResponse, error) {
	return p.response, p.err
}

func TestRuntimeAggregatesUsageFromProviderError(t *testing.T) {
	providerErr := internalerrors.ProviderRejected("completion was incomplete", false)
	runtime := agent.Runtime{Provider: responseErrorProvider{
		response: agent.CompletionResponse{
			Usage:                &agent.TokenUsage{PromptTokens: 5, CompletionTokens: 7, TotalTokens: 12, ThoughtsTokens: 2},
			ProviderRetries:      1,
			ProviderRetryReasons: []string{"malformed_function_call"},
		},
		err: providerErr,
	}}
	result, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 1})
	if err != providerErr {
		t.Fatalf("error = %v", err)
	}
	if result.Usage == nil || result.Usage.PromptTokens != 5 || result.Usage.CompletionTokens != 7 || result.Usage.TotalTokens != 12 || result.Usage.ThoughtsTokens != 2 {
		t.Fatalf("usage = %#v", result.Usage)
	}
	if result.Telemetry.ProviderRetries != 1 || len(result.Telemetry.ProviderRetryReasons) != 1 || result.Telemetry.ProviderRetryReasons[0] != "malformed_function_call" {
		t.Fatalf("telemetry = %#v", result.Telemetry)
	}
}

func TestRuntimeLeavesUsageUnknownWithoutProviderUsage(t *testing.T) {
	providerErr := internalerrors.ProviderRejected("completion failed", false)
	runtime := agent.Runtime{Provider: responseErrorProvider{err: providerErr}}
	result, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 1})
	if err != providerErr {
		t.Fatalf("error = %v", err)
	}
	if result.Usage != nil {
		t.Fatalf("usage = %#v, want nil", result.Usage)
	}
}

func TestRuntimeReturnsPartialMetricsOnFailure(t *testing.T) {
	provider := &failingProvider{
		response: agent.CompletionResponse{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "read-1", Function: agent.ToolFunction{Name: "read"},
		}}}, Usage: &agent.TokenUsage{PromptTokens: 7, CompletionTokens: 4, TotalTokens: 11}},
		err: errors.New("provider stopped"),
	}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "read"}},
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

func TestRuntimeReturnsLastTypedToolFailureWhenModelStops(t *testing.T) {
	cause := errors.New("generation changed")
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "write-1", Function: agent.ToolFunction{Name: "write"},
		}}}},
		{Message: agent.ChatMessage{Role: "assistant", Content: "stopped"}},
	}}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}},
		Execute: func(context.Context, agent.ToolCall) (string, error) {
			return "", internalerrors.Wrap(internalerrors.KindConflict, "publishing state failed", cause)
		},
	}}}
	_, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 2, MaxToolCalls: 2})
	if !internalerrors.IsKind(err, internalerrors.KindConflict) || !errors.Is(err, cause) {
		t.Fatalf("error = %v", err)
	}
}

func TestRuntimeClearsToolFailureAfterSuccessfulBatch(t *testing.T) {
	cause := errors.New("generation changed")
	provider := &fakeProvider{responses: []agent.CompletionResponse{
		{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "fail-1", Function: agent.ToolFunction{Name: "write"},
		}}}},
		{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "ok-1", Function: agent.ToolFunction{Name: "write"},
		}}}},
		{Message: agent.ChatMessage{Role: "assistant", Content: "done"}},
	}}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}},
		Execute: func(_ context.Context, call agent.ToolCall) (string, error) {
			if call.ID == "fail-1" {
				return "", internalerrors.Wrap(internalerrors.KindConflict, "publishing state failed", cause)
			}
			return "ok", nil
		},
	}}}
	_, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 3, MaxToolCalls: 3})
	if err != nil {
		t.Fatalf("error after successful batch = %v", err)
	}
}

func TestRuntimeUsesToolFailureAtTerminalLimits(t *testing.T) {
	tests := []struct {
		name    string
		message agent.ChatMessage
		options agent.Options
	}{
		{
			name: "turn limit",
			message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
				ID: "write-1", Function: agent.ToolFunction{Name: "write"},
			}}},
			options: agent.Options{MaxTurns: 1, MaxToolCalls: 2},
		},
		{
			name: "tool call limit",
			message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{
				{ID: "write-1", Function: agent.ToolFunction{Name: "write"}},
				{ID: "write-2", Function: agent.ToolFunction{Name: "write"}},
			}},
			options: agent.Options{MaxTurns: 2, MaxToolCalls: 1},
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			cause := errors.New("generation changed")
			provider := &fakeProvider{responses: []agent.CompletionResponse{{Message: test.message}}}
			runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
				Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}},
				Execute: func(context.Context, agent.ToolCall) (string, error) {
					return "", internalerrors.Wrap(internalerrors.KindConflict, "publishing state failed", cause)
				},
			}}}
			_, err := runtime.Run(context.Background(), nil, test.options)
			if !internalerrors.IsKind(err, internalerrors.KindConflict) || !errors.Is(err, cause) {
				t.Fatalf("error = %v", err)
			}
		})
	}
}

func TestRuntimePrefersCurrentContextError(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	provider := &fakeProvider{responses: []agent.CompletionResponse{{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
		ID: "write-1", Function: agent.ToolFunction{Name: "write"},
	}}}}}}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}},
		Execute: func(context.Context, agent.ToolCall) (string, error) {
			cancel()
			return "", internalerrors.Conflict("publishing state failed")
		},
	}}}
	_, err := runtime.Run(ctx, nil, agent.Options{MaxTurns: 2, MaxToolCalls: 2})
	if !errors.Is(err, context.Canceled) || !internalerrors.IsKind(err, internalerrors.KindCanceled) {
		t.Fatalf("error = %v", err)
	}
}

func TestRuntimePrefersCurrentProviderError(t *testing.T) {
	providerErr := internalerrors.ProviderRejected("provider is busy", true)
	provider := &failingProvider{
		response: agent.CompletionResponse{Message: agent.ChatMessage{Role: "assistant", ToolCalls: []agent.ToolCall{{
			ID: "write-1", Function: agent.ToolFunction{Name: "write"},
		}}}},
		err: providerErr,
	}
	runtime := agent.Runtime{Provider: provider, Tools: []agent.Tool{{
		Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}},
		Execute: func(context.Context, agent.ToolCall) (string, error) {
			return "", internalerrors.Conflict("publishing state failed")
		},
	}}}
	_, err := runtime.Run(context.Background(), nil, agent.Options{MaxTurns: 3, MaxToolCalls: 3})
	if !internalerrors.IsKind(err, internalerrors.KindProviderRejected) || err != providerErr {
		t.Fatalf("error = %v", err)
	}
}
