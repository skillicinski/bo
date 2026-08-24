package application

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type eventReader struct{ page OperationPage }

func (r eventReader) ReadEvents(context.Context, int, int) (OperationPage, error) {
	return r.page, nil
}

func (r eventReader) CommitEvent(context.Context, Operation) error { return nil }

func operationEvent(id string, outcome domain.OperationOutcome) Operation {
	event := Operation{
		OperationID: id, Attempt: 1, Timestamp: "2026-08-24T00:00:00Z", Actor: "agent",
		Command: domain.CommandState, Outcome: outcome,
	}
	if outcome == domain.OutcomeFailed {
		event.Error = &domain.OperationError{Kind: "provider_transport", Retryable: true}
	}
	return event
}

func TestReadLogsReturnsCommittedAndFailedEvents(t *testing.T) {
	page := OperationPage{
		Directory:  "notes",
		Entries:    []Operation{operationEvent("failed", domain.OutcomeFailed), operationEvent("committed", domain.OutcomeCommitted)},
		Offset:     0,
		Limit:      2,
		NextOffset: 2,
	}
	contextState := &agentContext{events: eventReader{page: page}, directory: "notes", maxOutputBytes: 1 << 20}
	data, err := readLogs(contextState, 0, 2)
	if err != nil {
		t.Fatal(err)
	}
	if !json.Valid([]byte(data)) || len(data) > contextState.maxOutputBytes {
		t.Fatalf("read_logs output is not bounded JSON: %q", data)
	}
	var got OperationPage
	if err := json.Unmarshal([]byte(data), &got); err != nil {
		t.Fatal(err)
	}
	if len(got.Entries) != 2 || got.Entries[0].Outcome != domain.OutcomeFailed || got.Entries[1].Outcome != domain.OutcomeCommitted {
		t.Fatalf("read_logs entries = %#v", got.Entries)
	}
	if got.Entries[0].Error == nil || got.Entries[0].Error.Kind != "provider_transport" || !got.Entries[0].Error.Retryable {
		t.Fatalf("failed event error = %#v", got.Entries[0].Error)
	}
}

func TestBoundedOperationPageAdvancesCursorWhenReducing(t *testing.T) {
	page := OperationPage{
		Directory: "notes",
		Entries: []Operation{
			operationEvent("one", domain.OutcomeCommitted),
			operationEvent("two", domain.OutcomeCommitted),
			{OperationID: "three", Attempt: 1, Timestamp: "2026-08-24T00:00:00Z", Actor: strings.Repeat("x", 128), Command: domain.CommandState, Outcome: domain.OutcomeCommitted},
		},
		Offset: 0, Limit: 3, NextOffset: 3,
	}
	candidate := page
	candidate.Entries = page.Entries[:2]
	candidate.NextOffset = 2
	candidate.HasMore = true
	encodedCandidate, err := json.Marshal(candidate)
	if err != nil {
		t.Fatal(err)
	}
	data, err := boundedOperationPage(page, len(encodedCandidate))
	if err != nil {
		t.Fatal(err)
	}
	if !json.Valid([]byte(data)) || len(data) > len(encodedCandidate) {
		t.Fatalf("bounded page = %q", data)
	}
	var got OperationPage
	if err := json.Unmarshal([]byte(data), &got); err != nil {
		t.Fatal(err)
	}
	if len(got.Entries) != 2 || got.NextOffset != 2 || !got.HasMore {
		t.Fatalf("reduced page = %#v", got)
	}
}

func TestBoundedOperationPageFailsWhenNoEntryFits(t *testing.T) {
	page := OperationPage{
		Directory: "notes",
		Entries: []Operation{{
			OperationID: "large", Attempt: 1, Timestamp: "2026-08-24T00:00:00Z", Actor: strings.Repeat("x", 256),
			Command: domain.CommandState, Outcome: domain.OutcomeCommitted,
		}},
		Limit: 1, NextOffset: 1,
	}
	if _, err := boundedOperationPage(page, 64); !internalerrors.IsKind(err, internalerrors.KindValidation) {
		t.Fatalf("bounded page error = %v", err)
	}
}

func TestOnlyCommittedWriteSummaryEventsCompleteSynthesis(t *testing.T) {
	writtenAt := time.Date(2026, time.August, 24, 0, 0, 0, 0, time.UTC)
	contextState := &agentContext{
		state: domain.State{Sources: []domain.SourceRecord{{
			SourceKey: "https://example.test/article",
			Snapshots: []domain.RawRecord{{Filename: "article.md", WrittenAt: writtenAt}},
			Summary:   &domain.SummaryRecord{Filename: "article-summary.md", DerivedFrom: "article.md", CreatedAt: writtenAt, UpdatedAt: writtenAt},
		}}},
		sources:   map[string]agentSource{"https://example.test/article": {LatestFilename: "article.md", LatestWrittenAt: writtenAt}},
		completed: map[string]bool{},
	}
	failed := operationEvent("failed-write", domain.OutcomeFailed)
	failed.Command = domain.CommandWriteSummary
	failed.Source = &domain.SourceIdentity{SourceKey: "https://example.test/article"}
	failed.Provenance = &domain.OperationProvenance{DerivedFrom: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "article.md"}}
	markCompletedFromOperation(contextState, failed)
	if contextState.completed["https://example.test/article"] {
		t.Fatal("failed write_summary event marked synthesis complete")
	}
	nonSummary := operationEvent("committed-state", domain.OutcomeCommitted)
	nonSummary.Source = failed.Source
	markCompletedFromOperation(contextState, nonSummary)
	if contextState.completed["https://example.test/article"] {
		t.Fatal("non-summary event marked synthesis complete")
	}
	committed := failed
	committed.OperationID = "committed-write"
	committed.Outcome = domain.OutcomeCommitted
	committed.Error = nil
	markCompletedFromOperation(contextState, committed)
	if !contextState.completed["https://example.test/article"] {
		t.Fatal("committed write_summary event did not mark synthesis complete")
	}
}

func TestScopedSynthesisEventsKeepsWorkspaceRecords(t *testing.T) {
	workspaceFailure := operationEvent("workspace-failure", domain.OutcomeFailed)
	workspaceFailure.Command = domain.CommandSynth
	otherSource := operationEvent("other-source", domain.OutcomeCommitted)
	otherSource.Command = domain.CommandWriteSummary
	otherSource.Source = &domain.SourceIdentity{SourceKey: "https://example.test/other"}
	currentSource := operationEvent("current-source", domain.OutcomeCommitted)
	currentSource.Command = domain.CommandWriteSummary
	currentSource.Source = &domain.SourceIdentity{SourceKey: "https://example.test/current"}

	got := scopedSynthesisEvents([]Operation{workspaceFailure, otherSource, currentSource}, map[string]agentSource{
		"https://example.test/current": {LatestFilename: "current.md"},
	})
	if len(got) != 2 || got[0].OperationID != workspaceFailure.OperationID || got[1].OperationID != currentSource.OperationID {
		t.Fatalf("scoped events = %#v", got)
	}
}
