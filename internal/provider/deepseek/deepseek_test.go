package deepseek

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"testing"

	"github.com/skillicinski/bo/internal/agent"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }

func TestCompleteUsesProviderDefaultModel(t *testing.T) {
	client := New("key", "http://provider.test/completions")
	client.HTTPClient = &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		var payload struct {
			Model string `json:"model"`
		}
		if err := json.NewDecoder(request.Body).Decode(&payload); err != nil {
			t.Errorf("decode request: %v", err)
		}
		if payload.Model != DefaultModel {
			t.Errorf("model = %q, want %q", payload.Model, DefaultModel)
		}
		return &http.Response{
			StatusCode: http.StatusOK,
			Body:       io.NopCloser(bytes.NewReader([]byte(`{"choices":[{"message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}`))),
			Header:     http.Header{"Content-Type": []string{"application/json"}},
			Request:    request,
		}, nil
	})}
	result, err := client.Complete(context.Background(), agent.CompletionRequest{})
	if err != nil {
		t.Fatal(err)
	}
	if result.Usage == nil || result.Usage.TotalTokens != 5 {
		t.Fatalf("usage = %#v", result.Usage)
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
