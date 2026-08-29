package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"testing"
	"time"

	"github.com/skillicinski/bo"
)

func TestPublicWorkflows(t *testing.T) {
	ctx := context.Background()
	path := writeSource(t)
	store := &storage{revision: bo.NewRevision(nil)}
	ws := workspace{name: "consumer", store: store}
	options := bo.OperationOptions{Actor: "consumer"}

	if _, err := bo.Seed(ctx, bo.SeedRequest{Creator: creator{}, Name: "consumer", Operations: options}); err != nil {
		t.Fatal(err)
	}
	snap, err := bo.Snap(ctx, bo.SnapRequest{Workspace: ws, Sources: []string{path}, Operations: options})
	if err != nil || len(snap.Outcomes) != 1 || snap.Outcomes[0].Err != nil {
		t.Fatalf("snap = %#v, error = %v", snap, err)
	}
	state, err := bo.ReadState(ctx, bo.StateRequest{Workspace: ws, Operations: options})
	if err != nil || state.State.SnapshotCount() != 1 {
		t.Fatalf("state = %#v, error = %v", state, err)
	}

	synthStore := &storage{
		state: bo.State{Sources: []bo.SourceRecord{{
			SourceKey: "https://example.test/article",
			Snapshots: []bo.RawRecord{{Filename: "article.md", WrittenAt: time.Unix(1, 0).UTC()}},
		}}},
		revision:  bo.NewRevision(nil),
		documents: map[bo.DocumentRef][]byte{bo.RawRef("article.md"): []byte("# Article\n\ncontent\n")},
	}
	synthWorkspace := workspace{name: "consumer", store: synthStore}
	response, err := json.Marshal(map[string]any{
		"choices": []any{map[string]any{
			"message": map[string]any{
				"role": "assistant",
				"tool_calls": []any{map[string]any{
					"id": "write-1", "type": "function",
					"function": map[string]string{
						"name":      "write_summary",
						"arguments": `{"source_key":"https://example.test/article","markdown":"# Summary\n\ncontent\n"}`,
					},
				}},
			},
			"finish_reason": "tool_calls",
		}},
		"usage": map[string]int{"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
	})
	if err != nil {
		t.Fatal(err)
	}
	result, err := bo.Synth(ctx, bo.SynthRequest{
		Workspace: synthWorkspace,
		Provider: bo.NewDeepSeekProvider(bo.DeepSeekConfig{
			APIKey: "test", Endpoint: "https://provider.test/completions",
			HTTPClient: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				return &http.Response{
					StatusCode: http.StatusOK,
					Body:       io.NopCloser(bytes.NewReader(response)),
					Header:     http.Header{"Content-Type": []string{"application/json"}},
					Request:    request,
				}, nil
			})},
		}),
		Options: bo.SynthesisOptions{
			MaxTurns: 1, MaxToolCalls: 1, MaxToolOutputBytes: 1024, MaxResponseTokens: 64, TimeoutSeconds: 5,
		},
		Operations: options,
	})
	if err != nil || result.SummariesWritten != 1 || synthStore.state.Sources[0].Summary == nil || string(synthStore.documents[bo.RawRef("article.md")]) != "# Article\n\ncontent\n" || string(synthStore.documents[bo.SummaryRef("article.md")]) != "# Summary\n\ncontent\n" {
		t.Fatalf("synth = %#v, error = %v, state = %#v", result, err, synthStore.state)
	}
}

func TestPublicDistill(t *testing.T) {
	ctx := context.Background()
	store := &storage{
		state: bo.State{Sources: []bo.SourceRecord{
			{SourceKey: "https://example.test/one", Snapshots: []bo.RawRecord{{Filename: "one.md", WrittenAt: time.Unix(1, 0).UTC()}}},
			{SourceKey: "https://example.test/two", Snapshots: []bo.RawRecord{{Filename: "two.md", WrittenAt: time.Unix(2, 0).UTC()}}},
		}},
		revision: bo.NewRevision(nil),
		documents: map[bo.DocumentRef][]byte{
			bo.RawRef("one.md"): []byte("one\n"),
			bo.RawRef("two.md"): []byte("two\n"),
		},
	}
	response, err := json.Marshal(map[string]any{
		"choices": []any{map[string]any{
			"message": map[string]any{
				"role": "assistant",
				"tool_calls": []any{map[string]any{
					"id": "read-one", "type": "function",
					"function": map[string]string{"name": "read_document", "arguments": `{"filename":"one.md"}`},
				}},
			},
			"finish_reason": "tool_calls",
		}},
		"usage": map[string]int{"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
	})
	if err != nil {
		t.Fatal(err)
	}
	responses := [][]byte{response}
	writeResponse, err := json.Marshal(map[string]any{
		"choices": []any{map[string]any{
			"message": map[string]any{
				"role": "assistant",
				"tool_calls": []any{map[string]any{
					"id": "read-two", "type": "function",
					"function": map[string]string{"name": "read_document", "arguments": `{"filename":"two.md"}`},
				}},
			},
			"finish_reason": "tool_calls",
		}},
		"usage": map[string]int{"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
	})
	if err != nil {
		t.Fatal(err)
	}
	responses = append(responses, writeResponse)
	finalResponse, err := json.Marshal(map[string]any{
		"choices": []any{map[string]any{
			"message": map[string]any{
				"role": "assistant",
				"tool_calls": []any{map[string]any{
					"id": "write", "type": "function",
					"function": map[string]string{"name": "write_distill", "arguments": `{"title":"Shared facts","introduction":"intro","sections":[{"heading":"Facts","paragraph":"paragraph","bullets":["one","two"],"sources":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md"}]}]}`},
				}},
			},
			"finish_reason": "tool_calls",
		}},
		"usage": map[string]int{"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
	})
	if err != nil {
		t.Fatal(err)
	}
	responses = append(responses, finalResponse)
	index := 0
	result, err := bo.Distill(ctx, bo.DistillRequest{
		Workspace: workspace{name: "consumer", store: store},
		Provider: bo.NewDeepSeekProvider(bo.DeepSeekConfig{
			APIKey: "test", Endpoint: "https://provider.test/completions",
			HTTPClient: &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
				payload := responses[index]
				index++
				return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(bytes.NewReader(payload)), Header: http.Header{"Content-Type": []string{"application/json"}}, Request: request}, nil
			})},
		}),
		Options: bo.SynthesisOptions{MaxTurns: 3, MaxToolCalls: 3, MaxToolOutputBytes: 4096, MaxResponseTokens: 64, TimeoutSeconds: 5},
	})
	if err != nil || result.Skipped || result.Filename != "shared-facts.md" || len(store.state.SynthesizedDocuments) != 1 {
		t.Fatalf("distill = %#v, error = %v, state = %#v", result, err, store.state)
	}
}

func TestConfiguredSnapSources(t *testing.T) {
	ctx := context.Background()
	path := writeSource(t)
	store := &storage{revision: bo.NewRevision(nil)}
	var requests int
	client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		requests++
		if request.URL.String() != "https://example.test/remote" {
			t.Fatalf("request URL = %s", request.URL)
		}
		return &http.Response{
			StatusCode: http.StatusOK,
			Header:     http.Header{"Content-Type": []string{"text/html"}},
			Body:       io.NopCloser(bytes.NewReader([]byte("<html><head><title>Remote</title></head><body><article>remote</article></body></html>"))),
		}, nil
	})}
	result, err := bo.Snap(ctx, bo.SnapRequest{
		Workspace: workspace{name: "consumer", store: store},
		Sources:   []string{"https://example.test/remote", path},
		SourceConfig: &bo.SnapSourceConfig{
			AllowLocalFiles: false,
			HTTPClient:      client,
		},
	})
	if err != nil || len(result.Outcomes) != 2 || result.Outcomes[0].Err != nil || result.Outcomes[1].Err == nil {
		t.Fatalf("configured snap = %#v, error = %v", result, err)
	}
	if requests != 1 || store.state.SnapshotCount() != 1 || store.state.Sources[0].SourceKey != "https://example.test/remote" {
		t.Fatalf("configured snap state = %#v, requests = %d", store.state, requests)
	}
}

func TestSnapRejectsFragmentsBeforePersistence(t *testing.T) {
	ctx := context.Background()
	store := &storage{revision: bo.NewRevision(nil)}
	requests := 0
	client := &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		requests++
		return &http.Response{StatusCode: http.StatusOK, Header: http.Header{"Content-Type": []string{"text/html"}}, Body: io.NopCloser(bytes.NewReader(nil))}, nil
	})}
	result, err := bo.Snap(ctx, bo.SnapRequest{
		Workspace: workspace{name: "consumer", store: store},
		Sources:   []string{"https://example.test/remote#credential"},
		SourceConfig: &bo.SnapSourceConfig{
			HTTPClient: client,
		},
	})
	if err != nil || len(result.Outcomes) != 1 || result.Outcomes[0].Err == nil {
		t.Fatalf("fragment snap = %#v, error = %v", result, err)
	}
	if requests != 0 || store.state.SnapshotCount() != 0 || len(store.events) != 1 || store.events[0].Source != nil {
		t.Fatalf("fragment persisted: state = %#v, events = %#v, requests = %d", store.state, store.events, requests)
	}
	data, err := json.Marshal(store.events)
	if err != nil || bytes.Contains(data, []byte("credential")) {
		t.Fatalf("fragment event = %s, error = %v", data, err)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return f(request)
}

func TestScopedWorkspacesDoNotShareState(t *testing.T) {
	ctx := context.Background()
	path := writeSource(t)
	first := &storage{revision: bo.NewRevision(nil)}
	second := &storage{revision: bo.NewRevision(nil)}
	options := bo.OperationOptions{Actor: "consumer"}
	for _, store := range []*storage{first, second} {
		result, err := bo.Snap(ctx, bo.SnapRequest{
			Workspace: workspace{name: "scoped", store: store}, Sources: []string{path}, Operations: options,
		})
		if err != nil || len(result.Outcomes) != 1 || result.Outcomes[0].Err != nil {
			t.Fatalf("snap result = %#v, error = %v", result, err)
		}
	}
	if first.state.SnapshotCount() != 1 || second.state.SnapshotCount() != 1 {
		t.Fatalf("workspace state leaked: first=%#v second=%#v", first.state, second.state)
	}
}

func writeSource(t *testing.T) string {
	t.Helper()
	file, err := os.CreateTemp(t.TempDir(), "source-*.md")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteString("# Article\n\ncontent\n"); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	return file.Name()
}
