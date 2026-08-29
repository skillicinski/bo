package local_test

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
)

func operation(timestamp, id string, command domain.OperationCommand) domain.Operation {
	return domain.Operation{OperationID: id, Attempt: 1, Timestamp: timestamp, Actor: "test", Command: command, Outcome: domain.OutcomeCommitted}
}

func snapshotEvent(sourceKey, filename string, writtenAt time.Time) domain.Operation {
	return domain.Operation{
		OperationID: "snapshot-" + filename, Attempt: 1, Timestamp: writtenAt.Format(time.RFC3339Nano), Actor: "test",
		Command: domain.CommandSnap, Outcome: domain.OutcomeCommitted,
		Source: &domain.SourceIdentity{SourceKey: sourceKey}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: filename},
	}
}

func summaryEvent(sourceKey, filename, derivedFrom string, rawWrittenAt, updatedAt time.Time) domain.Operation {
	return domain.Operation{
		OperationID: "summary-" + filename + "-" + strconv.FormatInt(updatedAt.UnixNano(), 10), Attempt: 1, Timestamp: updatedAt.Format(time.RFC3339Nano), Actor: "test",
		Command: domain.CommandWriteSummary, Outcome: domain.OutcomeCommitted,
		Source: &domain.SourceIdentity{SourceKey: sourceKey}, Document: &domain.DocumentIdentity{Kind: domain.DocumentKindSummary, Filename: filename},
		Provenance: &domain.OperationProvenance{DerivedFrom: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: derivedFrom}, RawWrittenAt: &rawWrittenAt},
	}
}

func TestWorkspaceMutationStoresCommittedEventWithContent(t *testing.T) {
	store, _ := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	writtenAt := time.Unix(1, 0).UTC()
	event := snapshotEvent("raw:note.md", "note.md", writtenAt)
	if _, _, err := store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: writtenAt, Contents: []byte("note\n"), Event: event,
	}, revision); err != nil {
		t.Fatal(err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.Sources) != 1 || len(state.Sources[0].Snapshots) != 1 {
		t.Fatalf("state = %#v, err = %v", state, err)
	}
	page, err := store.ReadEvents(context.Background(), 0, 20)
	if err != nil || len(page.Entries) != 1 || page.Entries[0].Outcome != domain.OutcomeCommitted {
		t.Fatalf("events = %#v, err = %v", page, err)
	}
	contents, err := store.ReadDocument(context.Background(), domain.RawRef("note.md"))
	if err != nil || string(contents) != "note\n" {
		t.Fatalf("raw document = %q, err = %v", contents, err)
	}
}

func TestWorkspaceRejectsMissingOrUnrelatedMutationEvent(t *testing.T) {
	store, _ := seededStore(t)
	_, revision, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"),
	}, revision)
	if err == nil {
		t.Fatal("snapshot without event succeeded")
	}
	event := snapshotEvent("raw:other.md", "note.md", time.Unix(1, 0).UTC())
	_, _, err = store.CommitSnapshot(context.Background(), application.SnapshotCommit{
		SourceKey: "raw:note.md", Filename: "note.md", WrittenAt: time.Unix(1, 0).UTC(), Contents: []byte("note\n"), Event: event,
	}, revision)
	if err == nil {
		t.Fatal("snapshot with unrelated event succeeded")
	}
}

func TestWorkspaceEventsPaginationIsBounded(t *testing.T) {
	store, _ := seededStore(t)
	for index := 0; index < 105; index++ {
		event := operation("2026-08-23T12:00:00Z", "op-page-"+strconv.Itoa(index), domain.CommandState)
		if err := store.CommitEvent(context.Background(), event); err != nil {
			t.Fatal(err)
		}
	}
	first, err := store.ReadEvents(context.Background(), 0, 1000)
	if err != nil || len(first.Entries) != 100 || first.Limit != 100 || !first.HasMore || first.NextOffset != 100 {
		t.Fatalf("first event page = %#v, err = %v", first, err)
	}
	second, err := store.ReadEvents(context.Background(), first.NextOffset, 1000)
	if err != nil || len(second.Entries) != 5 || second.HasMore || second.NextOffset != 105 {
		t.Fatalf("second event page = %#v, err = %v", second, err)
	}
}

func TestEventAppendDoesNotChangeContentRevision(t *testing.T) {
	store, _ := seededStore(t)
	_, before, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if err := store.CommitEvent(context.Background(), operation("2026-08-23T12:00:00Z", "read", domain.CommandState)); err != nil {
		t.Fatal(err)
	}
	_, after, err := store.ReadState(context.Background())
	if err != nil || !before.Equal(after) {
		t.Fatalf("revision after event = %s, %v", after, err)
	}
}

func TestWorkspaceEventsRejectMalformedLines(t *testing.T) {
	store, target := seededStore(t)
	file, err := os.OpenFile(filepath.Join(target, "log.jsonl"), os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteString("not-json\n"); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ReadEvents(context.Background(), 0, 20); err == nil {
		t.Fatal("malformed event line was accepted")
	}
}

func TestWorkspaceEventsRejectOversizedLines(t *testing.T) {
	store, target := seededStore(t)
	file, err := os.OpenFile(filepath.Join(target, "log.jsonl"), os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteString(strings.Repeat("x", 1<<20) + "\n"); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ReadEvents(context.Background(), 0, 20); err == nil {
		t.Fatal("oversized event line was accepted")
	}
}

func TestWorkspaceEventsConcurrentAppends(t *testing.T) {
	store, _ := seededStore(t)
	const count = 32
	var group sync.WaitGroup
	group.Add(count)
	for index := 0; index < count; index++ {
		go func(index int) {
			defer group.Done()
			if err := store.CommitEvent(context.Background(), operation("2026-08-23T12:00:00Z", "op-"+strconv.Itoa(index), domain.CommandState)); err != nil {
				t.Errorf("append: %v", err)
			}
		}(index)
	}
	group.Wait()
	page, err := store.ReadEvents(context.Background(), 0, count)
	if err != nil || len(page.Entries) != count || page.HasMore {
		t.Fatalf("page = %#v, err = %v", page, err)
	}
}
