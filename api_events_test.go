package bo

import (
	"context"
	"testing"
	"time"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type eventPageWorkspace struct{ page OperationPage }

func (w eventPageWorkspace) Name() string { return "notes" }

func (w eventPageWorkspace) ListDocuments(context.Context, DocumentKind) ([]DocumentRef, error) {
	return nil, nil
}

func (w eventPageWorkspace) ReadDocument(context.Context, DocumentRef) ([]byte, error) {
	return nil, nil
}

func (w eventPageWorkspace) ReadState(context.Context) (State, Revision, error) {
	return State{}, NewRevision(nil), nil
}

func (w eventPageWorkspace) ReadEvents(context.Context, int, int) (OperationPage, error) {
	return w.page, nil
}

func (w eventPageWorkspace) ReadRecentEvents(context.Context, int) ([]Operation, error) {
	return w.page.Entries, nil
}

func (w eventPageWorkspace) CommitEvent(context.Context, Operation) error { return nil }

func (w eventPageWorkspace) CommitSnapshot(context.Context, SnapshotCommit, Revision) (State, Revision, error) {
	return State{}, Revision{}, nil
}

func (w eventPageWorkspace) CommitSummary(context.Context, SummaryCommit, Revision) (State, Revision, error) {
	return State{}, Revision{}, nil
}

func (w eventPageWorkspace) Close() error { return nil }

func validPublicEvent(id string, outcome OperationOutcome) Operation {
	event := Operation{OperationID: id, Attempt: 1, Timestamp: "2026-08-24T00:00:00Z", Actor: "agent", Command: CommandState, Outcome: outcome}
	if outcome == OutcomeFailed {
		event.Error = &OperationError{Kind: "provider_transport", Retryable: true}
	}
	return event
}

func TestPublicWorkspaceRejectsInvalidEventPages(t *testing.T) {
	valid := validPublicEvent("valid", OutcomeCommitted)
	negativeDuration := valid
	negativeDuration.Metrics = &OperationMetrics{Duration: -time.Nanosecond}
	cases := []struct {
		name string
		page OperationPage
	}{
		{
			name: "invalid event",
			page: OperationPage{Entries: []Operation{{}}, Offset: 0, Limit: 1, NextOffset: 1},
		},
		{
			name: "negative metrics duration",
			page: OperationPage{Entries: []Operation{negativeDuration}, Offset: 0, Limit: 1, NextOffset: 1},
		},
		{
			name: "offset mismatch",
			page: OperationPage{Entries: []Operation{valid}, Offset: 1, Limit: 1, NextOffset: 2},
		},
		{
			name: "limit mismatch",
			page: OperationPage{Entries: []Operation{valid}, Offset: 0, Limit: 2, NextOffset: 1},
		},
		{
			name: "page exceeds limit",
			page: OperationPage{Entries: []Operation{valid, validPublicEvent("second", OutcomeFailed)}, Offset: 0, Limit: 1, NextOffset: 2},
		},
		{
			name: "cursor does not progress",
			page: OperationPage{Offset: 0, Limit: 1, HasMore: true},
		},
		{
			name: "cursor skips entries",
			page: OperationPage{Entries: []Operation{valid}, Offset: 0, Limit: 1, NextOffset: 2},
		},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			workspace := &publicWorkspace{workspace: eventPageWorkspace{page: test.page}}
			if _, err := workspace.ReadEvents(context.Background(), 0, 1); !internalerrors.IsKind(err, internalerrors.KindValidation) {
				t.Fatalf("ReadEvents error = %v", err)
			}
		})
	}
}

func TestPublicWorkspaceAcceptsTypedFailedEvent(t *testing.T) {
	page := OperationPage{
		Entries:    []Operation{validPublicEvent("failed", OutcomeFailed)},
		Offset:     0,
		Limit:      1,
		NextOffset: 1,
	}
	workspace := &publicWorkspace{workspace: eventPageWorkspace{page: page}}
	got, err := workspace.ReadEvents(context.Background(), 0, 1)
	if err != nil || len(got.Entries) != 1 || got.Entries[0].Error == nil || got.Entries[0].Error.Kind != "provider_transport" || !got.Entries[0].Error.Retryable {
		t.Fatalf("ReadEvents = %#v, error = %v", got, err)
	}
}

func TestPublicWorkspaceRejectsInvalidEventPageRequest(t *testing.T) {
	workspace := &publicWorkspace{workspace: eventPageWorkspace{}}
	for _, test := range []struct {
		name          string
		offset, limit int
	}{
		{name: "negative offset", offset: -1, limit: 1},
		{name: "zero limit", offset: 0, limit: 0},
		{name: "limit too large", offset: 0, limit: 101},
	} {
		t.Run(test.name, func(t *testing.T) {
			if _, err := workspace.ReadEvents(context.Background(), test.offset, test.limit); !internalerrors.IsKind(err, internalerrors.KindValidation) {
				t.Fatalf("ReadEvents error = %v", err)
			}
		})
	}
}
