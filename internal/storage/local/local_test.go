package local_test

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo"
	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/storage/local"
)

func seededStore(t *testing.T) (*local.Store, string) {
	t.Helper()
	home := t.TempDir()
	name := "notes"
	target, err := local.Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := local.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	return store, target
}

func commitRawNote(t *testing.T, store *local.Store) application.Revision {
	t.Helper()
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, revision, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: snapshotEvent("raw:note.md", "note.md", time.Unix(1, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	return revision
}

func commitSummaryNote(t *testing.T, store *local.Store, revision application.Revision) application.Revision {
	t.Helper()
	_, revision, err := store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md", RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("old\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(2, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	return revision
}

func TestValidateNameUsesPortableRules(t *testing.T) {
	for _, name := range []string{"", ".", "..", "../escape", "sub/name", "sub\\name", "CON", "CON.txt", "COM1", "LPT9", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "name.", "name ", "line\nbreak"} {
		if err := local.ValidateName(name); err == nil {
			t.Errorf("ValidateName(%q) succeeded", name)
		}
	}
	for _, name := range []string{"test", "hello-world", "has space", ".hidden", "café"} {
		if err := local.ValidateName(name); err != nil {
			t.Errorf("ValidateName(%q): %v", name, err)
		}
	}
}

func TestSeedFailureLeavesWorkspaceNameAvailable(t *testing.T) {
	home := t.TempDir()
	name := "retry"
	if _, err := local.SeedWithEvent(home, &name, domain.Operation{Command: domain.CommandSnap}); err == nil {
		t.Fatal("invalid seed event succeeded")
	}
	if _, err := local.Seed(home, &name); err != nil {
		t.Fatalf("retry seed: %v", err)
	}
	oversized := domain.Operation{
		OperationID: strings.Repeat("x", 1<<20), Attempt: 1, Timestamp: "1970-01-01T00:00:00Z", Actor: "test",
		Command: domain.CommandSeed, Outcome: domain.OutcomeCommitted,
	}
	oversizedName := "oversized"
	if _, err := local.SeedWithEvent(home, &oversizedName, oversized); err == nil {
		t.Fatal("oversized seed event succeeded")
	}
	if _, err := local.Seed(home, &oversizedName); err != nil {
		t.Fatalf("retry oversized seed: %v", err)
	}
}

func TestCommitEventRejectsOversizedLine(t *testing.T) {
	store, target := seededStore(t)
	event := domain.Operation{
		OperationID: strings.Repeat("x", 1<<20), Attempt: 1, Timestamp: "1970-01-01T00:00:00Z", Actor: "test",
		Command: domain.CommandState, Outcome: domain.OutcomeCommitted,
	}
	if err := store.CommitEvent(context.Background(), event); err == nil {
		t.Fatal("oversized event succeeded")
	}
	data, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if len(data) != 0 {
		t.Fatalf("oversized event changed ledger: %d bytes", len(data))
	}
}

func TestReadRecentEventsReturnsBoundedTail(t *testing.T) {
	store, _ := seededStore(t)
	for index := 0; index < 128; index++ {
		event := domain.Operation{
			OperationID: fmt.Sprintf("event-%03d", index), Attempt: 1,
			Timestamp: time.Unix(int64(index+1), 0).UTC().Format(time.RFC3339), Actor: "test",
			Command: domain.CommandState, Outcome: domain.OutcomeCommitted,
		}
		if err := store.CommitEvent(context.Background(), event); err != nil {
			t.Fatal(err)
		}
	}
	events, err := store.ReadRecentEvents(context.Background(), 3)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 3 {
		t.Fatalf("recent events = %d", len(events))
	}
	for index, event := range events {
		want := fmt.Sprintf("event-%03d", 125+index)
		if event.OperationID != want {
			t.Fatalf("recent event %d = %q, want %q", index, event.OperationID, want)
		}
	}
}

func TestLocalRevisionConflict(t *testing.T) {
	store, target := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	changed, err := domain.MarshalState(domain.State{Sources: []domain.SourceRecord{{
		SourceKey: "raw:changed.md", Snapshots: []domain.RawRecord{{Filename: "changed.md", WrittenAt: time.Unix(1, 0).UTC()}},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "changed.md"), []byte("changed\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "state.json"), changed, 0o600); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:new.md", Filename: "new.md", WrittenAt: time.Unix(2, 0).UTC(), Contents: []byte("new\n"), Event: snapshotEvent("raw:new.md", "new.md", time.Unix(2, 0).UTC()),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindConflict) {
		t.Fatalf("expected conflict, got %v", err)
	}
}

func TestLocalExternalRawEditChangesRevisionWhenSizeIsUnchanged(t *testing.T) {
	store, target := seededStore(t)
	revision := commitRawNote(t, store)
	path := filepath.Join(target, "note.md")
	if err := os.WriteFile(path, []byte("edit\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(path, time.Unix(1, 0), time.Unix(1, 0)); err != nil {
		t.Fatal(err)
	}
	_, current, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if current.Equal(revision) {
		t.Fatal("same-size external edit did not change revision")
	}
}

func TestLocalDeletedReferencedRawFailsBeforeMutation(t *testing.T) {
	store, target := seededStore(t)
	revision := commitRawNote(t, store)
	stateBefore, err := os.ReadFile(filepath.Join(target, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	eventsBefore, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(target, "note.md")); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:new.md", Filename: "new.md", WrittenAt: time.Unix(2, 0).UTC(), Contents: []byte("new\n"), Event: snapshotEvent("raw:new.md", "new.md", time.Unix(2, 0).UTC()),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindMissingResource) {
		t.Fatalf("expected missing raw resource, got %v", err)
	}
	stateAfter, err := os.ReadFile(filepath.Join(target, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	eventsAfter, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if string(stateAfter) != string(stateBefore) || string(eventsAfter) != string(eventsBefore) {
		t.Fatal("deleted raw changed workspace state")
	}
	if _, err := os.Stat(filepath.Join(target, "new.md")); !os.IsNotExist(err) {
		t.Fatalf("new raw document status = %v", err)
	}
}

func TestLocalDeletedReferencedSummaryFailsBeforeMutation(t *testing.T) {
	store, target := seededStore(t)
	revision := commitSummaryNote(t, store, commitRawNote(t, store))
	stateBefore, err := os.ReadFile(filepath.Join(target, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	eventsBefore, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(target, "summaries", "note.md")); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md", RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("new\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(3, 0).UTC()),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindMissingResource) {
		t.Fatalf("expected missing summary resource, got %v", err)
	}
	stateAfter, err := os.ReadFile(filepath.Join(target, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	eventsAfter, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if string(stateAfter) != string(stateBefore) || string(eventsAfter) != string(eventsBefore) {
		t.Fatal("deleted summary changed workspace state")
	}
	if _, err := os.Stat(filepath.Join(target, "summaries", "note.md")); !os.IsNotExist(err) {
		t.Fatalf("summary status after rejected commit = %v", err)
	}
}

func TestLocalEmptyReferencedSummaryFailsBeforeMutation(t *testing.T) {
	store, target := seededStore(t)
	revision := commitSummaryNote(t, store, commitRawNote(t, store))
	stateBefore, err := os.ReadFile(filepath.Join(target, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	eventsBefore, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "summaries", "note.md"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md", RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("new\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(3, 0).UTC()),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindMissingResource) {
		t.Fatalf("expected empty summary resource, got %v", err)
	}
	stateAfter, err := os.ReadFile(filepath.Join(target, "state.json"))
	if err != nil {
		t.Fatal(err)
	}
	eventsAfter, err := os.ReadFile(filepath.Join(target, "log.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if string(stateAfter) != string(stateBefore) || string(eventsAfter) != string(eventsBefore) {
		t.Fatal("empty summary changed workspace state")
	}
	contents, err := os.ReadFile(filepath.Join(target, "summaries", "note.md"))
	if err != nil {
		t.Fatal(err)
	}
	if len(contents) != 0 {
		t.Fatalf("summary after rejected commit = %q", contents)
	}
}

func TestLocalExistingSummaryEditUsesCurrentRevision(t *testing.T) {
	store, target := seededStore(t)
	revision := commitSummaryNote(t, store, commitRawNote(t, store))
	if err := os.WriteFile(filepath.Join(target, "summaries", "note.md"), []byte("manual\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	loaded, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatalf("manual summary edit invalid: %v", err)
	}
	if loaded.Sources[0].Summary == nil {
		t.Fatal("summary disappeared after manual edit")
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md", RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: loaded.Sources[0].Summary.CreatedAt, UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("updated\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(3, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	contents, err := os.ReadFile(filepath.Join(target, "summaries", "note.md"))
	if err != nil || string(contents) != "updated\n" {
		t.Fatalf("summary after edit = %q, %v", contents, err)
	}
}

func TestLocalCommitRejectsInvalidSummary(t *testing.T) {
	store, _ := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.txt",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(1, 0).UTC(), UpdatedAt: time.Unix(1, 0).UTC(), Contents: []byte("summary\n"),
	}, revision)
	if err == nil {
		t.Fatal("invalid summary was committed")
	}
	loaded, _, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Sources) != 0 {
		t.Fatalf("state changed after rejected publish: %#v", loaded)
	}
}

func TestLocalExternalSummaryEditConflictsWithoutOverwrite(t *testing.T) {
	store, target := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	state, revision, err := store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: snapshotEvent("raw:note.md", "note.md", time.Unix(1, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	state, revision, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("old\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(2, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "summaries", "note.md"), []byte("external\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: state.Sources[0].Summary.CreatedAt, UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("bo overwrite\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(3, 0).UTC()),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindConflict) {
		t.Fatalf("expected external edit conflict, got %v", err)
	}
	contents, err := os.ReadFile(filepath.Join(target, "summaries", "note.md"))
	if err != nil || string(contents) != "external\n" {
		t.Fatalf("summary after conflict = %q, %v", contents, err)
	}
	loaded, _, err := store.ReadState(context.Background())
	if err != nil || loaded.Sources[0].Summary == nil || loaded.Sources[0].Summary.UpdatedAt != state.Sources[0].Summary.UpdatedAt {
		t.Fatalf("state after conflict = %#v, %v", loaded, err)
	}
}

func TestLocalExternalSummaryEditConflictsWithRestoredMetadata(t *testing.T) {
	store, target := seededStore(t)
	revision := commitSummaryNote(t, store, commitRawNote(t, store))
	path := filepath.Join(target, "summaries", "note.md")
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	originalModTime := info.ModTime()
	external := []byte("new\n")
	if err := os.WriteFile(path, external, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(path, originalModTime, originalModTime); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("bo!\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(3, 0).UTC()),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindConflict) {
		t.Fatalf("expected typed conflict, got %v", err)
	}
	contents, err := os.ReadFile(path)
	if err != nil || string(contents) != string(external) {
		t.Fatalf("summary after conflict = %q, %v", contents, err)
	}
}

func TestLocalSummaryCommitDoesNotUseFixedTemporary(t *testing.T) {
	store, target := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, revision, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: snapshotEvent("raw:note.md", "note.md", time.Unix(1, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	state, revision, err := store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("old\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(2, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(target, ".state.json.tmp"), 0o755); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: state.Sources[0].Summary.CreatedAt, UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("new\n"), Event: summaryEvent("raw:note.md", "note.md", "note.md", time.Unix(1, 0).UTC(), time.Unix(3, 0).UTC()),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	contents, err := store.ReadDocument(context.Background(), domain.SummaryRef("note.md"))
	if err != nil || string(contents) != "new\n" {
		t.Fatalf("summary after commit = %q, %v", contents, err)
	}
	loaded, _, err := store.ReadState(context.Background())
	if err != nil || loaded.Sources[0].Summary == nil || loaded.Sources[0].Summary.UpdatedAt != time.Unix(3, 0).UTC() {
		t.Fatalf("state after commit = %#v, %v", loaded, err)
	}
}
