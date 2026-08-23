package local_test

import (
	"context"
	"os"
	"path/filepath"
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
	if err := os.WriteFile(filepath.Join(target, "state.json"), changed, 0o600); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:new.md", Filename: "new.md", WrittenAt: time.Unix(2, 0).UTC(), Contents: []byte("new\n"),
	}, revision)
	if !bo.IsKind(err, bo.ErrorKindConflict) {
		t.Fatalf("expected conflict, got %v", err)
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
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	state, revision, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("old\n"),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "summaries", "note.md"), []byte("external\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: state.Sources[0].Summary.CreatedAt, UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("bo overwrite\n"),
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

func TestLocalSummaryCommitDoesNotUseFixedTemporary(t *testing.T) {
	store, target := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, revision, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	state, revision, err := store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("old\n"),
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(target, ".state.json.tmp"), 0o755); err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: state.Sources[0].Summary.CreatedAt, UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("new\n"),
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
