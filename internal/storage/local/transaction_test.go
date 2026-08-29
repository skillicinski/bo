package local

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
)

func timePtr(value time.Time) *time.Time { return &value }

func TestPreparedSummaryTransactionRollsBackAfterSummaryRename(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	target, err := Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, revision, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: domain.Operation{
			OperationID: "transaction-snapshot", Attempt: 1, Timestamp: "1970-01-01T00:00:01Z", Actor: "test", Command: domain.CommandSnap, Outcome: domain.OutcomeCommitted,
			Source: &domain.SourceIdentity{SourceKey: "raw:note.md"}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "note.md"},
		},
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	_, revision, err = store.CommitSummary(context.Background(), application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("old\n"), Event: domain.Operation{
			OperationID: "transaction-summary", Attempt: 1, Timestamp: "1970-01-01T00:00:02Z", Actor: "test", Command: domain.CommandWriteSummary, Outcome: domain.OutcomeCommitted,
			Source: &domain.SourceIdentity{SourceKey: "raw:note.md"}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindSummary, Filename: "note.md"},
			Provenance: &domain.OperationProvenance{DerivedFrom: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "note.md"}, RawWrittenAt: timePtr(time.Unix(1, 0).UTC())},
		},
	}, revision)
	if err != nil {
		t.Fatal(err)
	}
	current, oldState, _, err := store.readState()
	if err != nil {
		t.Fatal(err)
	}
	commit := application.SummaryCommit{
		SourceKey: "raw:note.md", Filename: "note.md", DerivedFrom: "note.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(3, 0).UTC(), Contents: []byte("new\n"),
	}
	next, err := applySummary(current, commit)
	if err != nil {
		t.Fatal(err)
	}
	newState, err := domain.MarshalState(next)
	if err != nil {
		t.Fatal(err)
	}
	oldSummary, hadOldSummary, err := store.optionalDocument(domain.SummaryRef(commit.Filename))
	if err != nil {
		t.Fatal(err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSummary, commit.Filename, oldSummary, hadOldSummary, commit.Contents, oldState, newState)
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.writeAtomic(filepath.Join("summaries", commit.Filename), transaction.DocumentTemporary, commit.Contents); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	recovered, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	defer recovered.Close()
	state, _, err := recovered.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if state.Sources[0].Summary == nil || state.Sources[0].Summary.UpdatedAt != time.Unix(2, 0).UTC() {
		t.Fatalf("recovered state = %#v", state)
	}
	contents, err := recovered.ReadDocument(context.Background(), domain.SummaryRef(commit.Filename))
	if err != nil || string(contents) != "old\n" {
		t.Fatalf("recovered summary = %q, %v", contents, err)
	}
}

func TestSnapshotTransactionRollsBackAfterRawRename(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	target, err := Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	current, oldState, _, err := store.readState()
	if err != nil {
		t.Fatal(err)
	}
	commit := application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"),
	}
	next, err := appendSnapshot(current, commit)
	if err != nil {
		t.Fatal(err)
	}
	newState, err := domain.MarshalState(next)
	if err != nil {
		t.Fatal(err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSnapshot, commit.Filename, nil, false, commit.Contents, oldState, newState)
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.writeNewRaw(commit.Filename, transaction.DocumentTemporary, commit.Contents); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	recovered, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	defer recovered.Close()
	state, _, err := recovered.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 0 {
		t.Fatalf("recovered state = %#v", state)
	}
	if _, err := recovered.ReadDocument(context.Background(), domain.RawRef(commit.Filename)); err == nil {
		t.Fatalf("recovered raw document error = %v", err)
	}
}

func TestCommittedSnapshotTransactionFinishesAfterMarkerPublication(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	target, err := Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	current, oldState, _, err := store.readState()
	if err != nil {
		t.Fatal(err)
	}
	commit := application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"),
	}
	next, err := appendSnapshot(current, commit)
	if err != nil {
		t.Fatal(err)
	}
	newState, err := domain.MarshalState(next)
	if err != nil {
		t.Fatal(err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSnapshot, commit.Filename, nil, false, commit.Contents, oldState, newState)
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.writeNewRaw(commit.Filename, transaction.DocumentTemporary, commit.Contents); err != nil {
		t.Fatal(err)
	}
	if err := store.writeAtomic("state.json", transaction.StateTemporary, newState); err != nil {
		t.Fatal(err)
	}
	transaction.Phase = workspaceTransactionPhaseCommit
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	recovered, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	defer recovered.Close()
	state, _, err := recovered.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 1 || state.Sources[0].SourceKey != commit.SourceKey || len(state.Sources[0].Snapshots) != 1 {
		t.Fatalf("recovered state = %#v", state)
	}
	contents, err := recovered.ReadDocument(context.Background(), domain.RawRef(commit.Filename))
	if err != nil || string(contents) != string(commit.Contents) {
		t.Fatalf("recovered raw document = %q, %v", contents, err)
	}
}

func TestPreparedTransactionSurvivesFailedRollbackPublication(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	target, err := Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	current, oldState, _, err := store.readState()
	if err != nil {
		t.Fatal(err)
	}
	commit := application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"),
	}
	next, err := appendSnapshot(current, commit)
	if err != nil {
		t.Fatal(err)
	}
	newState, err := domain.MarshalState(next)
	if err != nil {
		t.Fatal(err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSnapshot, commit.Filename, nil, false, commit.Contents, oldState, newState)
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.writeNewRaw(commit.Filename, transaction.DocumentTemporary, commit.Contents); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(target, transaction.TransactionTemporary), 0o755); err != nil {
		t.Fatal(err)
	}
	stateTemporary := filepath.Join(target, transaction.StateTemporary)
	if err := os.Mkdir(stateTemporary, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(stateTemporary, "blocked"), []byte("blocked\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	if err := store.writeAtomic("state.json", transaction.StateTemporary, newState); err == nil {
		t.Fatal("state publication succeeded")
	}
	if err := store.abortWorkspaceTransaction(transaction, errors.New("write failed")); err == nil {
		t.Fatal("abort succeeded")
	}
	stored, err := store.readWorkspaceTransaction()
	if err != nil {
		t.Fatal(err)
	}
	if stored == nil || stored.Phase != workspaceTransactionPhaseReady {
		t.Fatalf("stored transaction = %#v", stored)
	}
	if err := os.Remove(filepath.Join(target, transaction.TransactionTemporary)); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(stateTemporary, "blocked")); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(stateTemporary); err != nil {
		t.Fatal(err)
	}
	if err := store.recoverWorkspaceTransaction(); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ReadDocument(context.Background(), domain.RawRef(commit.Filename)); err == nil {
		t.Fatalf("raw document after recovery error = %v", err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestPreparedTransactionRecoversPartialWorkspaceEvent(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	seedEvent := domain.Operation{
		OperationID: "partial-seed", Attempt: 1, Timestamp: "1970-01-01T00:00:00Z", Actor: "test", Command: domain.CommandSeed, Outcome: domain.OutcomeCommitted,
	}
	target, err := SeedWithEvent(home, &name, seedEvent)
	if err != nil {
		t.Fatal(err)
	}
	store, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	current, oldState, _, err := store.readState()
	if err != nil {
		t.Fatal(err)
	}
	commit := application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: domain.Operation{
			OperationID: "partial-event", Attempt: 1, Timestamp: "1970-01-01T00:00:01Z", Actor: "test", Command: domain.CommandSnap, Outcome: domain.OutcomeCommitted,
			Source: &domain.SourceIdentity{SourceKey: "raw:note.md"}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "note.md"},
		},
	}
	next, err := appendSnapshot(current, commit)
	if err != nil {
		t.Fatal(err)
	}
	newState, err := domain.MarshalState(next)
	if err != nil {
		t.Fatal(err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSnapshot, commit.Filename, nil, false, commit.Contents, oldState, newState)
	if err := store.trackTransactionEvent(&transaction, commit.Event); err != nil {
		t.Fatal(err)
	}
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	oldLedger, err := os.Stat(filepath.Join(target, workspaceEventFile))
	if err != nil {
		t.Fatal(err)
	}
	ledger, err := os.OpenFile(filepath.Join(target, workspaceEventFile), os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	partialLength := len(transaction.EventLine) / 2
	if _, err := ledger.Write(transaction.EventLine[:partialLength]); err != nil {
		_ = ledger.Close()
		t.Fatal(err)
	}
	if err := ledger.Close(); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	recovered, err := Open(target)
	if err != nil {
		t.Fatalf("recovery failed: %v", err)
	}
	defer recovered.Close()
	page, err := recovered.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].OperationID != seedEvent.OperationID {
		t.Fatalf("recovered events = %#v, err = %v", page, err)
	}
	info, err := os.Stat(filepath.Join(target, workspaceEventFile))
	if err != nil {
		t.Fatal(err)
	}
	if info.Size() != oldLedger.Size() {
		t.Fatalf("recovered ledger size = %d, want %d", info.Size(), oldLedger.Size())
	}
}

func TestCommittedTransactionRecoversWorkspaceEvent(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	target, err := Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	current, oldState, _, err := store.readState()
	if err != nil {
		t.Fatal(err)
	}
	commit := application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: domain.Operation{
			OperationID: "transaction-event", Attempt: 1, Timestamp: "1970-01-01T00:00:01Z", Actor: "test", Command: domain.CommandSnap, Outcome: domain.OutcomeCommitted,
			Source: &domain.SourceIdentity{SourceKey: "raw:note.md"}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "note.md"},
		},
	}
	next, err := appendSnapshot(current, commit)
	if err != nil {
		t.Fatal(err)
	}
	newState, err := domain.MarshalState(next)
	if err != nil {
		t.Fatal(err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSnapshot, commit.Filename, nil, false, commit.Contents, oldState, newState)
	if err := store.trackTransactionEvent(&transaction, commit.Event); err != nil {
		t.Fatal(err)
	}
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.writeNewRaw(commit.Filename, transaction.DocumentTemporary, commit.Contents); err != nil {
		t.Fatal(err)
	}
	if err := store.writeAtomic("state.json", transaction.StateTemporary, newState); err != nil {
		t.Fatal(err)
	}
	transaction.Phase = workspaceTransactionPhaseCommit
	if err := store.writeWorkspaceTransaction(transaction); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}

	recovered, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	defer recovered.Close()
	page, err := recovered.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].OperationID != commit.Event.OperationID {
		t.Fatalf("recovered events = %#v, err = %v", page, err)
	}
}

func TestOpenSharesWorkspaceLock(t *testing.T) {
	home := t.TempDir()
	name := "notes"
	target, err := Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	first, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	second, err := Open(filepath.Join(target, "."))
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()
	if first.mu != second.mu {
		t.Fatal("workspace handles do not share a lock")
	}

	_, revision, err := first.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	first.mu.Lock()
	result := make(chan error, 1)
	go func() {
		_, _, err := second.CommitSnapshot(context.Background(), application.SnapshotCommit{
			SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: domain.Operation{
				OperationID: "transaction-lock", Attempt: 1, Timestamp: "1970-01-01T00:00:01Z", Actor: "test", Command: domain.CommandSnap, Outcome: domain.OutcomeCommitted,
				Source: &domain.SourceIdentity{SourceKey: "raw:note.md"}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "note.md"},
			},
		}, revision)
		result <- err
	}()
	select {
	case err := <-result:
		first.mu.Unlock()
		t.Fatalf("commit completed while lock was held: %v", err)
	case <-time.After(50 * time.Millisecond):
	}
	first.mu.Unlock()
	if err := <-result; err != nil {
		t.Fatal(err)
	}
}
