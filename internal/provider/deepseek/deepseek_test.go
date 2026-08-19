package deepseek

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"testing"

	"github.com/skillicinski/bo/internal/agent"
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
			Body:       io.NopCloser(bytes.NewReader([]byte(`{"choices":[{"message":{"role":"assistant","content":"ok"}}]}`))),
			Header:     http.Header{"Content-Type": []string{"application/json"}},
			Request:    request,
		}, nil
	})}
	if _, err := client.Complete(context.Background(), agent.CompletionRequest{}); err != nil {
		t.Fatal(err)
	}
}
