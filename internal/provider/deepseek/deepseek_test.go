package deepseek

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"

	"github.com/skillicinski/bo/internal/agent"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }

type failingReadCloser struct{ err error }

func (f failingReadCloser) Read([]byte) (int, error) { return 0, f.err }
func (f failingReadCloser) Close() error             { return nil }

func TestCompleteUsesProviderDefaultModel(t *testing.T) {
	client := New("key", "http://provider.test/completions")
	client.HTTPClient = &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		var payload completionRequest
		if err := json.NewDecoder(request.Body).Decode(&payload); err != nil {
			t.Errorf("decode request: %v", err)
		}
		if payload.Model != DefaultModel {
			t.Errorf("model = %q, want %q", payload.Model, DefaultModel)
		}
		if payload.Stream || payload.Thinking.Type != "disabled" || payload.ToolChoice != "auto" {
			t.Errorf("provider settings = %#v", payload)
		}
		if len(payload.Messages) != 1 || payload.Messages[0].Role != "user" || len(payload.Tools) != 1 || payload.Tools[0].Type != "function" {
			t.Errorf("wire tools and messages = %#v", payload)
		}
		return &http.Response{
			StatusCode: http.StatusOK,
			Body:       io.NopCloser(bytes.NewReader([]byte(`{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}`))),
			Header:     http.Header{"Content-Type": []string{"application/json"}},
			Request:    request,
		}, nil
	})}
	result, err := client.Complete(context.Background(), agent.CompletionRequest{
		Messages: []agent.ChatMessage{{Role: "user", Content: "read"}},
		Tools:    []agent.ToolDefinition{{Function: agent.ToolDeclaration{Name: "read"}}}, MaxTokens: 12,
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Usage == nil || result.Usage.TotalTokens != 5 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}

func TestCompletePreservesToolCallsAndUsage(t *testing.T) {
	client := New("key", "http://provider.test/completions")
	client.HTTPClient = &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		return &http.Response{
			StatusCode: http.StatusOK,
			Body:       io.NopCloser(bytes.NewReader([]byte(`{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}`))),
			Header:     http.Header{"Content-Type": []string{"application/json"}}, Request: request,
		}, nil
	})}
	result, err := client.Complete(context.Background(), agent.CompletionRequest{})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Message.ToolCalls) != 1 || result.Message.ToolCalls[0].ID != "call-1" || result.Message.ToolCalls[0].Function.Name != "read" || result.Message.ToolCalls[0].Function.Arguments != "{}" {
		t.Fatalf("tool calls = %#v", result.Message.ToolCalls)
	}
	if result.Usage == nil || result.Usage.PromptTokens != 3 || result.Usage.CompletionTokens != 2 || result.Usage.TotalTokens != 5 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}

func TestCompleteRejectsOversizedAndInvalidResponses(t *testing.T) {
	oversized := []byte(`{"choices":[{"message":{"role":"assistant","content":"` + strings.Repeat("x", maxResponseBodyBytes) + `"},"finish_reason":"stop"}]}`)
	cases := []struct {
		name string
		body []byte
	}{
		{name: "oversized", body: oversized},
		{name: "negative usage", body: []byte(`{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":-1}}`)},
		{name: "missing message", body: []byte(`{"choices":[{"finish_reason":"stop"}]}`)},
		{name: "invalid finish reason", body: []byte(`{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"unknown"}]}`)},
		{name: "invalid content", body: []byte(`{"choices":[{"message":{"role":"assistant","content":{"text":"not a string"}},"finish_reason":"stop"}]}`)},
		{name: "tool calls missing", body: []byte(`{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"tool_calls"}]}`)},
		{name: "tool calls mismatch", body: []byte(`{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read","arguments":"{}"}}]},"finish_reason":"stop"}]}`)},
		{name: "tool call missing id", body: []byte(`{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"type":"function","function":{"name":"read","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}`)},
		{name: "tool call missing function", body: []byte(`{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"arguments":"{}"}}]},"finish_reason":"tool_calls"}]}`)},
		{name: "tool call invalid arguments", body: []byte(`{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read","arguments":"not-json"}}]},"finish_reason":"tool_calls"}]}`)},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			client := New("key", "http://provider.test/completions")
			client.HTTPClient = &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(bytes.NewReader(test.body)), Header: http.Header{}, Request: request}, nil
			})}
			_, err := client.Complete(context.Background(), agent.CompletionRequest{})
			if !internalerrors.IsKind(err, internalerrors.KindProviderMalformed) {
				t.Fatalf("error = %v", err)
			}
		})
	}
}

func TestCompleteClassifiesIncompleteFinishReasons(t *testing.T) {
	for _, test := range []struct {
		reason    string
		retryable bool
	}{
		{reason: "length"},
		{reason: "content_filter"},
		{reason: "insufficient_system_resource", retryable: true},
	} {
		t.Run(test.reason, func(t *testing.T) {
			client := New("key", "http://provider.test/completions")
			client.HTTPClient = &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				body := []byte(`{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"` + test.reason + `"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}`)
				return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(bytes.NewReader(body)), Header: http.Header{}, Request: request}, nil
			})}
			result, err := client.Complete(context.Background(), agent.CompletionRequest{})
			if !internalerrors.IsKind(err, internalerrors.KindProviderRejected) {
				t.Fatalf("error = %v", err)
			}
			if result.Usage == nil || result.Usage.TotalTokens != 5 {
				t.Fatalf("usage = %#v", result.Usage)
			}
			var typed *internalerrors.Error
			if !errors.As(err, &typed) || typed.Retryable != test.retryable {
				t.Fatalf("typed error = %#v", typed)
			}
		})
	}
}

func TestCompleteClassifiesResponseReadFailure(t *testing.T) {
	cause := errors.New("connection reset")
	client := New("key", "http://provider.test/completions")
	client.HTTPClient = &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: http.StatusOK, Body: failingReadCloser{err: cause}, Header: http.Header{}, Request: request}, nil
	})}
	_, err := client.Complete(context.Background(), agent.CompletionRequest{})
	if !internalerrors.IsKind(err, internalerrors.KindProviderTransport) || !errors.Is(err, cause) {
		t.Fatalf("error = %v", err)
	}
	var typed *internalerrors.Error
	if !errors.As(err, &typed) || !typed.Retryable {
		t.Fatalf("typed error = %#v", typed)
	}
}

func TestCompleteClassifiesProviderFailures(t *testing.T) {
	tests := []struct {
		name      string
		client    *http.Client
		kind      internalerrors.Kind
		retryable bool
	}{
		{
			name: "rejected and retryable",
			client: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: http.StatusTooManyRequests, Body: io.NopCloser(bytes.NewReader([]byte("busy"))), Header: http.Header{}, Request: request}, nil
			})},
			kind: internalerrors.KindProviderRejected, retryable: true,
		},
		{
			name: "bad request is not retryable",
			client: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: http.StatusBadRequest, Body: io.NopCloser(bytes.NewReader([]byte("invalid"))), Header: http.Header{}, Request: request}, nil
			})},
			kind: internalerrors.KindProviderRejected,
		},
		{
			name: "unauthorized is not retryable",
			client: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: http.StatusUnauthorized, Body: io.NopCloser(bytes.NewReader([]byte("unauthorized"))), Header: http.Header{}, Request: request}, nil
			})},
			kind: internalerrors.KindProviderRejected,
		},
		{
			name: "service unavailable is retryable",
			client: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: http.StatusServiceUnavailable, Body: io.NopCloser(bytes.NewReader([]byte("busy"))), Header: http.Header{}, Request: request}, nil
			})},
			kind: internalerrors.KindProviderRejected, retryable: true,
		},
		{
			name: "malformed",
			client: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(bytes.NewReader([]byte("not-json"))), Header: http.Header{}, Request: request}, nil
			})},
			kind: internalerrors.KindProviderMalformed,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			client := New("key", "http://provider.test/completions")
			client.HTTPClient = test.client
			_, err := client.Complete(context.Background(), agent.CompletionRequest{})
			if !internalerrors.IsKind(err, test.kind) {
				t.Fatalf("error = %v", err)
			}
			var typed *internalerrors.Error
			if !errors.As(err, &typed) {
				t.Fatalf("typed error = %v", err)
			}
			if typed.Retryable != test.retryable {
				t.Fatalf("retryable = %v, want %v", typed.Retryable, test.retryable)
			}
		})
	}

	cause := errors.New("connection refused")
	client := New("key", "http://provider.test/completions")
	client.HTTPClient = &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		return nil, cause
	})}
	_, err := client.Complete(context.Background(), agent.CompletionRequest{})
	if !internalerrors.IsKind(err, internalerrors.KindProviderTransport) || !errors.Is(err, cause) {
		t.Fatalf("transport error = %v", err)
	}
	var transportError *internalerrors.Error
	if !errors.As(err, &transportError) || !transportError.Retryable {
		t.Fatalf("transport retryable = %v", transportError)
	}

	client = New("key", "://invalid")
	_, err = client.Complete(context.Background(), agent.CompletionRequest{})
	if !internalerrors.IsKind(err, internalerrors.KindProviderTransport) {
		t.Fatalf("endpoint error = %v", err)
	}
	var endpointError *internalerrors.Error
	if !errors.As(err, &endpointError) {
		t.Fatalf("endpoint typed error = %v", err)
	}
	if endpointError.Retryable {
		t.Fatalf("endpoint retryable = %v", endpointError.Retryable)
	}
}
