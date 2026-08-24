package application_test

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/storage/local"
)

type failingCompletionProvider struct{}

func (failingCompletionProvider) Complete(context.Context, agent.CompletionRequest) (agent.CompletionResponse, error) {
	return agent.CompletionResponse{}, errors.New("provider failed: credentials must not be persisted")
}

func TestSeedStoresEventInWorkspaceLedger(t *testing.T) {
	home := t.TempDir()
	manager := local.NewManager(home)
	if _, err := application.Seed(context.Background(), manager, "notes", application.OperationOptions{Actor: "test"}); err != nil {
		t.Fatal(err)
	}
	workspace, err := manager.Open(context.Background(), "notes")
	if err != nil {
		t.Fatal(err)
	}
	defer workspace.Close()
	page, err := workspace.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].Command != domain.CommandSeed || page.Entries[0].Outcome != domain.OutcomeCommitted {
		t.Fatalf("seed events = %#v, err = %v", page, err)
	}
	if _, err := os.Stat(filepath.Join(home, ".bo", "log.jsonl")); !os.IsNotExist(err) {
		t.Fatalf("legacy process log exists: %v", err)
	}
}

func TestSnapStoresEventsInWorkspaceLedger(t *testing.T) {
	store, _ := seededStore(t)
	source := rawSource{
		"https://example.test/one": {Title: "One", Markdown: []byte("one\n")},
		"https://example.test/two": {Title: "Two", Markdown: []byte("two\n")},
	}
	if _, err := application.SnapWithWorkflow(context.Background(), store, source, []string{
		"https://example.test/one", "https://example.test/missing", "https://example.test/two",
	}, application.OperationOptions{Actor: "test"}); err != nil {
		t.Fatal(err)
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 3 {
		t.Fatalf("events = %#v, err = %v", page, err)
	}
	if page.Entries[0].Outcome != domain.OutcomeCommitted || page.Entries[1].Outcome != domain.OutcomeFailed || page.Entries[2].Outcome != domain.OutcomeCommitted {
		t.Fatalf("events = %#v", page.Entries)
	}
}

func TestReadEventDoesNotChangeContentRevision(t *testing.T) {
	store, _ := seededStore(t)
	_, before, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := application.ReadState(context.Background(), store, application.OperationOptions{Actor: "test"}); err != nil {
		t.Fatal(err)
	}
	_, after, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if !before.Equal(after) {
		t.Fatalf("content revision changed after read event: before=%s after=%s", before, after)
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].Command != domain.CommandState {
		t.Fatalf("events = %#v, err = %v", page, err)
	}
}

func TestReadStateFailureStoresTypedError(t *testing.T) {
	store, target := seededStore(t)
	if err := os.Remove(filepath.Join(target, "state.json")); err != nil {
		t.Fatal(err)
	}
	if _, _, err := application.ReadState(context.Background(), store, application.OperationOptions{Actor: "test"}); err == nil {
		t.Fatal("ReadState succeeded after state removal")
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].Error == nil {
		t.Fatalf("events = %#v, err = %v", page, err)
	}
}

func TestCanceledReadStillStoresFailureEvent(t *testing.T) {
	store, _ := seededStore(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, _, readErr := application.ReadState(ctx, store, application.OperationOptions{Actor: "test"})
	if readErr == nil {
		t.Fatal("canceled ReadState succeeded")
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].Outcome != domain.OutcomeFailed || page.Entries[0].Error == nil {
		t.Fatalf("events = %#v, read err = %v, event err = %v", page, readErr, err)
	}
}

func TestSynthesisStoresFailureWithoutProviderText(t *testing.T) {
	store, _ := seededStore(t)
	commitRaw(t, store, "https://example.test/article", "article.md", time.Unix(1, 0).UTC(), []byte("fact\n"))
	_, err := application.SynthesizeWithTools(context.Background(), store, failingCompletionProvider{}, application.DefaultSynthesisOptions(), []string{"read_logs"}, application.OperationOptions{Actor: "test"})
	if err == nil {
		t.Fatal("SynthesizeWithTools succeeded")
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil {
		t.Fatal(err)
	}
	var found bool
	for _, event := range page.Entries {
		if event.Command == domain.CommandSynth {
			found = true
			data, marshalErr := json.Marshal(event)
			if event.Outcome != domain.OutcomeFailed || event.Error == nil || marshalErr != nil || strings.Contains(string(data), "provider failed: credentials must not be persisted") {
				t.Fatalf("event = %#v", event)
			}
		}
	}
	if !found {
		t.Fatalf("synthesis event missing: %#v", page)
	}
}
