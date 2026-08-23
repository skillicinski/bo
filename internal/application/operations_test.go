package application_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/storage/local"
)

type operationRecorder struct {
	mu      sync.Mutex
	entries []application.Operation
	err     error
}

func (r *operationRecorder) Append(_ context.Context, operation application.Operation) error {
	if r.err != nil {
		return r.err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	r.entries = append(r.entries, operation)
	return nil
}

func (r *operationRecorder) Read(_ context.Context, directory string, offset, limit int) (application.OperationPage, error) {
	if r.err != nil {
		return application.OperationPage{}, r.err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	entries := make([]application.Operation, 0)
	for _, operation := range r.entries {
		if operation.Directory == directory {
			entries = append(entries, operation)
		}
	}
	if offset > len(entries) {
		offset = len(entries)
	}
	end := offset + limit
	if end > len(entries) {
		end = len(entries)
	}
	return application.OperationPage{Directory: directory, Entries: entries[offset:end], Offset: offset, Limit: limit, NextOffset: end, HasMore: end < len(entries)}, nil
}

type failingCreator struct{}

func (failingCreator) Create(context.Context, string) (string, error) {
	return "", errors.New("create failed")
}

type failingCompletionProvider struct{}

func (failingCompletionProvider) Complete(context.Context, agent.CompletionRequest) (agent.CompletionResponse, error) {
	return agent.CompletionResponse{}, errors.New("provider failed")
}

func TestWorkflowsRejectMissingOperationLog(t *testing.T) {
	store, target := seededStore(t)
	if _, err := application.SnapWithWorkflow(context.Background(), store, rawSource{}, "notes", []string{"url"}, application.OperationOptions{}); !application.IsCategory(err, application.CategoryRequest) {
		t.Fatalf("Snap error = %v", err)
	}
	if _, err := application.StateOutput(context.Background(), store, "notes", false, application.OperationOptions{}); !application.IsCategory(err, application.CategoryRequest) {
		t.Fatalf("StateOutput error = %v", err)
	}
	if _, err := application.Seed(context.Background(), failingCreator{}, "notes", application.OperationOptions{}); !application.IsCategory(err, application.CategoryRequest) {
		t.Fatalf("Seed error = %v", err)
	}
	if _, err := application.SynthesizeWithTools(context.Background(), nil, "notes", nil, application.DefaultSynthesisOptions(), []string{"read_logs"}, application.OperationOptions{}); !application.IsCategory(err, application.CategoryRequest) {
		t.Fatalf("Synthesize error = %v", err)
	}
	_ = target
}

func TestSeedLogsSuccessAndFailure(t *testing.T) {
	home := t.TempDir()
	log := local.NewOperationLog(home)
	manager := local.NewManager(home)
	options := application.OperationOptions{Log: log}
	if _, err := application.Seed(context.Background(), manager, "notes", options); err != nil {
		t.Fatal(err)
	}
	if _, err := application.Seed(context.Background(), manager, "notes", options); err == nil {
		t.Fatal("duplicate seed succeeded")
	}
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil || len(page.Entries) != 2 {
		t.Fatalf("page = %#v, %v", page, err)
	}
	if !page.Entries[0].Success || page.Entries[1].Success || page.Entries[0].Actor != "system" {
		t.Fatalf("entries = %#v", page.Entries)
	}
}

func TestSnapLogsEachURLAndIgnoresLoggerFailure(t *testing.T) {
	store, target := seededStore(t)
	log := local.NewOperationLog(filepath.Dir(filepath.Dir(target)))
	source := rawSource{
		"https://example.test/one": {Title: "One", Markdown: []byte("one\n")},
		"https://example.test/two": {Title: "Two", Markdown: []byte("two\n")},
	}
	if _, err := application.SnapWithWorkflow(context.Background(), store, source, "notes", []string{"https://example.test/one", "https://example.test/missing", "https://example.test/two"}, application.OperationOptions{Log: log}); err != nil {
		t.Fatal(err)
	}
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil || len(page.Entries) != 3 {
		t.Fatalf("page = %#v, %v", page, err)
	}
	if !page.Entries[0].Success || page.Entries[1].Success || !page.Entries[2].Success {
		t.Fatalf("entries = %#v", page.Entries)
	}
	failingLog := &operationRecorder{err: errors.New("log unavailable")}
	outcomes, err := application.SnapWithWorkflow(context.Background(), store, source, "notes", []string{"https://example.test/one"}, application.OperationOptions{Log: failingLog})
	if err != nil || len(outcomes) != 1 || outcomes[0].Err != nil {
		t.Fatalf("best effort snap = %#v, %v", outcomes, err)
	}
}

func TestStateLogsSuccessAndFailure(t *testing.T) {
	store, target := seededStore(t)
	log := local.NewOperationLog(filepath.Dir(filepath.Dir(target)))
	if _, err := application.StateOutput(context.Background(), store, "notes", false, application.OperationOptions{Log: log}); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(target, "state.json")); err != nil {
		t.Fatal(err)
	}
	if _, err := application.StateOutput(context.Background(), store, "notes", false, application.OperationOptions{Log: log}); err == nil {
		t.Fatal("StateOutput succeeded after state removal")
	}
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil || len(page.Entries) != 2 || !page.Entries[0].Success || page.Entries[1].Success {
		t.Fatalf("page = %#v, %v", page, err)
	}
}

func TestSynthesisUsesSuccessfulCurrentWriteLogWithoutWriting(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("fact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = append(state.Sources, domain.SourceRecord{SourceKey: "https://example.test/article", Snapshots: []domain.RawRecord{{Filename: raw.Name, WrittenAt: time.Unix(1, 0).UTC()}}})
	if _, err = store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	if err := store.ReplaceSummary(context.Background(), domain.SummaryRef(raw.Name), []byte("summary\n")); err != nil {
		t.Fatal(err)
	}
	state, generation, err = store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources[0].Summary = &domain.SummaryRecord{Filename: raw.Name, DerivedFrom: raw.Name, CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC()}
	if _, err = store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	log := local.NewOperationLog(filepath.Dir(filepath.Dir(target)))
	if err := log.Append(context.Background(), domain.Operation{Timestamp: "2026-08-21T10:00:00Z", Actor: "test", Directory: "notes", Command: domain.CommandWriteSummary, Success: true, Details: map[string]any{"source_key": "https://example.test/article", "derived_from": raw.Name}}); err != nil {
		t.Fatal(err)
	}
	provider := &fakeProvider{responses: []agent.CompletionResponse{toolResponse("logs-1", "read_logs", "{}")}}
	result, err := application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", provider, application.DefaultSynthesisOptions(), []string{"read_logs"}, application.OperationOptions{Log: log, Actor: "test"})
	if err != nil || result.SummariesWritten != 0 || result.SummariesSkipped != 1 {
		t.Fatalf("result = %#v, %v", result, err)
	}
	if len(provider.requests) != 1 {
		t.Fatalf("requests = %d", len(provider.requests))
	}
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil {
		t.Fatal(err)
	}
	for _, operation := range page.Entries {
		if operation.Command == domain.CommandWriteSummary && operation.Timestamp != "2026-08-21T10:00:00Z" {
			t.Fatalf("unexpected new write_summary event: %#v", operation)
		}
	}
}

func TestSynthesisLogsFailureMetrics(t *testing.T) {
	store, target := seededStore(t)
	raw, err := store.CreateRaw(context.Background(), "article.md", []byte("fact\n"))
	if err != nil {
		t.Fatal(err)
	}
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state.Sources = append(state.Sources, domain.SourceRecord{SourceKey: "https://example.test/article", Snapshots: []domain.RawRecord{{Filename: raw.Name, WrittenAt: time.Unix(1, 0).UTC()}}})
	if _, err := store.PublishState(context.Background(), state, generation); err != nil {
		t.Fatal(err)
	}
	log := local.NewOperationLog(filepath.Dir(filepath.Dir(target)))
	_, err = application.SynthesizeWithTools(context.Background(), local.NewManager(filepath.Dir(filepath.Dir(target))), "notes", failingCompletionProvider{}, application.DefaultSynthesisOptions(), []string{"read_logs"}, application.OperationOptions{Log: log})
	if err == nil {
		t.Fatal("SynthesizeWithTools succeeded")
	}
	page, err := log.Read(context.Background(), "notes", 0, 20)
	if err != nil || len(page.Entries) != 1 {
		t.Fatalf("page = %#v, %v", page, err)
	}
	entry := page.Entries[0]
	if entry.Command != application.CommandSynth || entry.Success || entry.Details["error"] != "provider failed" {
		t.Fatalf("entry = %#v", entry)
	}
}
