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
		documents: map[string][]byte{"article.md": []byte("# Article\n\ncontent\n")},
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
	if err != nil || result.SummariesWritten != 1 || synthStore.state.Sources[0].Summary == nil || string(synthStore.documents["article.md"]) != "# Summary\n\ncontent\n" {
		t.Fatalf("synth = %#v, error = %v, state = %#v", result, err, synthStore.state)
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
