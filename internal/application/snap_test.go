package application_test

import (
	"context"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
	urlsource "github.com/skillicinski/bo/internal/source/url"
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

func TestSnapRejectsCredentialURLsBeforeRequestOrStateCommit(t *testing.T) {
	for _, input := range []string{
		"https://user:secret@example.test/article",
		"https://example.test/article?token=secret",
	} {
		t.Run(input, func(t *testing.T) {
			store, target := seededStore(t)
			before, beforeRevision, err := store.ReadState(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			requests := 0
			client := &http.Client{Transport: snapRoundTripFunc(func(*http.Request) (*http.Response, error) {
				requests++
				return nil, errors.New("unexpected HTTP request")
			})}
			workflow := source.NewWorkflow(
				[]source.Transport{urlsource.NewTransport()},
				map[source.OriginType]source.Plugin{source.OriginHTML: urlsource.NewHTML(client)},
			)
			outcomes, err := application.Snap(context.Background(), store, workflow, []string{input}, operationOptionsFor(target))
			if err != nil || len(outcomes) != 1 || !internalerrors.IsKind(outcomes[0].Err, internalerrors.KindValidation) {
				t.Fatalf("outcomes = %#v, error = %v", outcomes, err)
			}
			if requests != 0 {
				t.Fatalf("HTTP requests = %d", requests)
			}
			after, afterRevision, err := store.ReadState(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(after, before) || !afterRevision.Equal(beforeRevision) {
				t.Fatalf("workspace state changed: before=%#v/%s after=%#v/%s", before, beforeRevision, after, afterRevision)
			}
		})
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

func TestSnapStopsOnContextFailureWithoutUnstartedEvents(t *testing.T) {
	for _, test := range []struct {
		name  string
		cause error
		kind  internalerrors.Kind
	}{
		{name: "canceled", cause: context.Canceled, kind: internalerrors.KindCanceled},
		{name: "deadline", cause: context.DeadlineExceeded, kind: internalerrors.KindDeadline},
	} {
		t.Run(test.name, func(t *testing.T) {
			store, target := seededStore(t)
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			source := &terminalRawSource{failure: test.cause, cancel: cancel}
			inputs := []string{
				"https://example.test/first",
				"https://example.test/second",
				"https://example.test/third",
			}
			outcomes, err := application.Snap(ctx, store, source, inputs, operationOptionsFor(target))
			if err == nil || !errors.Is(err, test.cause) || len(outcomes) != 1 {
				t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
			}
			var commandErr *application.SnapCommandError
			if !errors.As(err, &commandErr) || commandErr.SourceKey != inputs[1] || len(commandErr.Completed) != 1 {
				t.Fatalf("command error = %#v", commandErr)
			}
			if !internalerrors.IsKind(err, test.kind) || len(source.calls) != 2 || source.calls[1] != inputs[1] {
				t.Fatalf("calls = %#v, err = %v", source.calls, err)
			}
			page, pageErr := store.ReadEvents(context.Background(), 0, 20)
			if pageErr != nil || len(page.Entries) != 2 || page.Entries[1].Error == nil || page.Entries[1].Error.Kind != string(test.kind) {
				t.Fatalf("events = %#v, err = %v", page, pageErr)
			}
		})
	}
}

func TestSnapBoundsFilenameCollisions(t *testing.T) {
	store, target := seededStore(t)
	workspace := &alwaysExistingWorkspace{Workspace: store}
	outcomes, err := application.Snap(context.Background(), workspace, rawSource{
		"https://example.test/article": {Title: "Article", Markdown: []byte("content\n")},
	}, []string{"https://example.test/article"}, operationOptionsFor(target))
	if err == nil || !internalerrors.IsKind(err, internalerrors.KindAlreadyExists) || !errors.Is(err, internalerrors.ErrAlreadyExists) || len(outcomes) != 0 {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
	var categorized *internalerrors.Error
	if !errors.As(err, &categorized) || categorized.Kind != internalerrors.KindAlreadyExists {
		t.Fatalf("collision error = %v", err)
	}
	if workspace.attempts != 8 {
		t.Fatalf("collision attempts = %d", workspace.attempts)
	}
	page, pageErr := store.ReadEvents(context.Background(), 0, 20)
	if pageErr != nil || len(page.Entries) != workspace.attempts {
		t.Fatalf("events = %#v, err = %v", page, pageErr)
	}
	filenames := make(map[string]bool, len(page.Entries))
	for _, event := range page.Entries {
		if event.Outcome != domain.OutcomeFailed || event.Document == nil || filenames[event.Document.Filename] {
			t.Fatalf("collision event = %#v", event)
		}
		filenames[event.Document.Filename] = true
	}
}

func TestSnapChecksContextBetweenFilenameCollisions(t *testing.T) {
	store, target := seededStore(t)
	ctx, cancel := context.WithCancel(context.Background())
	workspace := &cancelOnCollisionWorkspace{Workspace: store, cancel: cancel}
	outcomes, err := application.Snap(ctx, workspace, rawSource{
		"https://example.test/article": {Title: "Article", Markdown: []byte("content\n")},
	}, []string{"https://example.test/article"}, operationOptionsFor(target))
	if err == nil || !errors.Is(err, context.Canceled) || len(outcomes) != 0 || workspace.attempts != 1 {
		t.Fatalf("outcomes = %#v, attempts = %d, err = %v", outcomes, workspace.attempts, err)
	}
	page, pageErr := store.ReadEvents(context.Background(), 0, 20)
	if pageErr != nil || len(page.Entries) != 2 || page.Entries[1].Attempt != 2 || page.Entries[1].Error == nil || page.Entries[1].Error.Kind != string(internalerrors.KindCanceled) {
		t.Fatalf("events = %#v, err = %v", page, pageErr)
	}
}

func TestSnapRecordsContextBeforeFirstCommit(t *testing.T) {
	store, target := seededStore(t)
	ctx, cancel := context.WithCancel(context.Background())
	source := cancelBeforeCommitSource{cancel: cancel}
	outcomes, err := application.Snap(ctx, store, source, []string{"https://example.test/article"}, operationOptionsFor(target))
	if err == nil || !errors.Is(err, context.Canceled) || len(outcomes) != 0 {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
	page, pageErr := store.ReadEvents(context.Background(), 0, 20)
	if pageErr != nil || len(page.Entries) != 1 || page.Entries[0].Attempt != 1 || page.Entries[0].Error == nil || page.Entries[0].Error.Kind != string(internalerrors.KindCanceled) {
		t.Fatalf("events = %#v, err = %v", page, pageErr)
	}
}

type terminalRawSource struct {
	failure error
	cancel  context.CancelFunc
	calls   []string
}

func (s *terminalRawSource) Fetch(_ context.Context, input string) (domain.RawSnapshot, error) {
	s.calls = append(s.calls, input)
	if len(s.calls) == 2 {
		s.cancel()
		return domain.RawSnapshot{}, s.failure
	}
	return domain.RawSnapshot{Title: "First", Markdown: []byte("content\n")}, nil
}

type cancelBeforeCommitSource struct {
	cancel context.CancelFunc
}

type snapRoundTripFunc func(*http.Request) (*http.Response, error)

func (f snapRoundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return f(request)
}

func (s cancelBeforeCommitSource) Fetch(context.Context, string) (domain.RawSnapshot, error) {
	s.cancel()
	return domain.RawSnapshot{Title: "Article", Markdown: []byte("content\n")}, nil
}

type alwaysExistingWorkspace struct {
	application.Workspace
	attempts int
}

func (w *alwaysExistingWorkspace) CommitSnapshot(context.Context, application.SnapshotCommit, application.Revision) (domain.State, application.Revision, error) {
	w.attempts++
	return domain.State{}, application.Revision{}, internalerrors.ErrAlreadyExists
}

type cancelOnCollisionWorkspace struct {
	application.Workspace
	cancel   context.CancelFunc
	attempts int
}

func (w *cancelOnCollisionWorkspace) CommitSnapshot(context.Context, application.SnapshotCommit, application.Revision) (domain.State, application.Revision, error) {
	w.attempts++
	w.cancel()
	return domain.State{}, application.Revision{}, internalerrors.AlreadyExists("document already exists")
}
