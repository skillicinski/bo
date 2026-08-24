package application_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
)

type rawSource map[string]domain.RawSnapshot

func (s rawSource) Fetch(_ context.Context, input string) (domain.RawSnapshot, error) {
	return s[input], nil
}

func TestSnapPublishesStateSequentially(t *testing.T) {
	store, target := seededStore(t)
	source := rawSource{
		"https://example.test/one": {Title: "First Page", Markdown: []byte("# First Page\n\ncontent\n")},
		"https://example.test/two": {Title: "Second Page", Markdown: []byte("# Second Page\n\ncontent\n")},
	}
	outcomes, err := application.Snap(context.Background(), store, source, []string{"https://example.test/one", "https://example.test/two"}, operationOptionsFor(target))
	if err != nil {
		t.Fatal(err)
	}
	if len(outcomes) != 2 || outcomes[0].Filename != "first-page.md" || outcomes[1].Filename != "second-page.md" {
		t.Fatalf("unexpected outcomes: %#v", outcomes)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 2 || state.Sources[0].SourceKey != "https://example.test/one" {
		t.Fatalf("unexpected state: %#v", state)
	}
	if _, err := os.Stat(filepath.Join(target, "first-page.md")); err != nil {
		t.Fatal(err)
	}
}

func TestSnapStoresRepeatedSourceSnapshotsInOneAggregate(t *testing.T) {
	store, target := seededStore(t)
	key := "https://example.test/article"
	outcomes, err := application.Snap(context.Background(), store, rawSource{
		key: {Title: "Article", Markdown: []byte("latest\n")},
	}, []string{key, key}, operationOptionsFor(target))
	if err != nil || len(outcomes) != 2 {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 1 || state.Sources[0].SourceKey != key || len(state.Sources[0].Snapshots) != 2 {
		t.Fatalf("state = %#v", state)
	}
}

func TestSnapDoesNotPublishWhenTransactionMarkerCannotBeWritten(t *testing.T) {
	store, target := seededStore(t)
	transactionMarker := filepath.Join(target, ".bo-transaction.json")
	if err := os.Mkdir(transactionMarker, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(transactionMarker, "blocked"), []byte("blocked\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := application.Snap(context.Background(), store, rawSource{"https://example.test/url": {Title: "Page", Markdown: []byte("content\n")}}, []string{"https://example.test/url"}, operationOptionsFor(target))
	if err == nil {
		t.Fatal("Snap succeeded")
	}
	if _, statErr := os.Stat(filepath.Join(target, "page.md")); !os.IsNotExist(statErr) {
		t.Fatalf("raw document remains: %v", statErr)
	}
	if err := os.Remove(filepath.Join(transactionMarker, "blocked")); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(transactionMarker); err != nil {
		t.Fatal(err)
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil {
		t.Fatal(err)
	}
	for _, event := range page.Entries {
		if event.Command == domain.CommandSnap && event.Outcome == domain.OutcomeCommitted {
			t.Fatalf("successful event recorded for failed mutation: %#v", event)
		}
	}
}

func TestSnapRecordsCorrelatedFailedAndCommittedAttempts(t *testing.T) {
	store, target := seededStore(t)
	if err := os.WriteFile(filepath.Join(target, "article.md"), []byte("external\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	key := "https://example.test/article"
	outcomes, err := application.Snap(context.Background(), store, rawSource{key: {Title: "Article", Markdown: []byte("latest\n")}}, []string{key}, operationOptionsFor(target))
	if err != nil || len(outcomes) != 1 || outcomes[0].Err != nil || outcomes[0].Filename == "article.md" {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 2 {
		t.Fatalf("events = %#v, err = %v", page, err)
	}
	first, second := page.Entries[0], page.Entries[1]
	if first.OperationID == "" || first.OperationID != second.OperationID || first.Attempt != 1 || second.Attempt != 2 || first.Outcome != domain.OutcomeFailed || second.Outcome != domain.OutcomeCommitted || first.Error == nil || first.Error.Retryable {
		t.Fatalf("attempt events = %#v", page.Entries)
	}
}

type failingRawSource struct{}

func (failingRawSource) Fetch(context.Context, string) (domain.RawSnapshot, error) {
	return domain.RawSnapshot{}, errors.New("caption unavailable")
}

func TestSnapDoesNotStoreFailedFetch(t *testing.T) {
	store, target := seededStore(t)
	outcomes, err := application.Snap(context.Background(), store, failingRawSource{}, []string{"https://www.youtube.com/watch?v=a1mhk7mAetk"}, operationOptionsFor(target))
	if err != nil {
		t.Fatal(err)
	}
	if len(outcomes) != 1 || outcomes[0].Err == nil {
		t.Fatalf("outcomes = %#v", outcomes)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 0 {
		t.Fatalf("state = %#v", state)
	}
	entries, err := os.ReadDir(target)
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if filepath.Ext(entry.Name()) == ".md" {
			t.Fatalf("failed snapshot remains: %s", entry.Name())
		}
	}
}

func TestSnapAcceptsMixedURLAndMarkdownInputs(t *testing.T) {
	store, target := seededStore(t)
	path := filepath.Join(t.TempDir(), "local.md")
	remote := "https://example.test/remote"
	outcomes, err := application.Snap(context.Background(), store, rawSource{
		remote: {SourceKey: remote, Title: "Remote", Markdown: []byte("remote content\n")},
		path:   {SourceKey: "raw:local.md", Title: "Local", Markdown: []byte("local content\n")},
	}, []string{remote, path}, operationOptionsFor(target))
	if err != nil || len(outcomes) != 2 {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
	if outcomes[0].SourceKey != remote || outcomes[1].SourceKey != "raw:local.md" {
		t.Fatalf("outcomes = %#v", outcomes)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.Sources) != 2 || state.Sources[1].SourceKey != "raw:local.md" {
		t.Fatalf("state = %#v, err = %v", state, err)
	}
}
