package application

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

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
	contextState := &agentContext{directory: "notes", maxOutputBytes: 1 << 20, logEvents: page.Entries, logWindowLoaded: true}
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
	if len(got.Entries) != 2 || got.Entries[0].Outcome != domain.OutcomeCommitted || got.Entries[1].Outcome != domain.OutcomeFailed {
		t.Fatalf("read_logs entries = %#v", got.Entries)
	}
	if got.Entries[1].Error == nil || got.Entries[1].Error.Kind != "provider_transport" || !got.Entries[1].Error.Retryable {
		t.Fatalf("failed event error = %#v", got.Entries[1].Error)
	}
}

func TestReadLogsDefaultsToNewestEvents(t *testing.T) {
	events := make([]Operation, 25)
	for index := range events {
		events[index] = operationEvent(fmt.Sprintf("event-%03d", index), domain.OutcomeCommitted)
	}
	contextState := &agentContext{directory: "notes", maxOutputBytes: 1 << 20, logEvents: events, logWindowLoaded: true}
	data, err := executeToolCall(contextState, agent.ToolCall{Function: agent.ToolFunction{Name: toolReadLogs, Arguments: "{}"}})
	if err != nil {
		t.Fatal(err)
	}
	var got OperationPage
	if err := json.Unmarshal([]byte(data), &got); err != nil {
		t.Fatal(err)
	}
	if len(got.Entries) != 20 || got.Entries[0].OperationID != "event-024" || got.Entries[19].OperationID != "event-005" || got.NextOffset != 20 || !got.HasMore {
		t.Fatalf("default read_logs page = %#v", got)
	}
}

type mutationWorkspace struct {
	Workspace
	commits int
}

func (w *mutationWorkspace) CommitSummary(context.Context, SummaryCommit, Revision) (domain.State, Revision, error) {
	w.commits++
	return domain.State{}, NewRevision(nil), nil
}

func TestReadLogsCursorRemainsStableAfterInterleavedMutation(t *testing.T) {
	events := make([]Operation, 25)
	for index := range events {
		events[index] = operationEvent(fmt.Sprintf("event-%03d", index), domain.OutcomeCommitted)
	}
	workspace := &mutationWorkspace{}
	contextState := &agentContext{
		workspace: workspace, directory: "notes", maxOutputBytes: 1 << 20,
		logEvents: events, logWindowLoaded: true,
		sources:   map[string]agentSource{"raw:article.md": {LatestFilename: "article.md"}},
		completed: map[string]bool{}, written: map[string]bool{}, mutationOps: map[string]Operation{},
	}
	data, err := readLogs(contextState, 0, 20)
	if err != nil {
		t.Fatal(err)
	}
	var first OperationPage
	if err := json.Unmarshal([]byte(data), &first); err != nil {
		t.Fatal(err)
	}
	if len(first.Entries) != 20 || first.Entries[0].OperationID != "event-024" || first.Entries[19].OperationID != "event-005" || first.NextOffset != 20 || !first.HasMore {
		t.Fatalf("first page = %#v", first)
	}
	if _, err := executeToolCall(contextState, agent.ToolCall{Function: agent.ToolFunction{
		Name: toolWriteSummary, Arguments: `{"source_key":"raw:article.md","markdown":"summary"}`,
	}}); err != nil {
		t.Fatal(err)
	}
	if workspace.commits != 1 || len(contextState.logEvents) != len(events) {
		t.Fatalf("mutation persistence/window = %d/%d", workspace.commits, len(contextState.logEvents))
	}
	data, err = readLogs(contextState, first.NextOffset, 20)
	if err != nil {
		t.Fatal(err)
	}
	var second OperationPage
	if err := json.Unmarshal([]byte(data), &second); err != nil {
		t.Fatal(err)
	}
	if len(second.Entries) != 5 || second.Entries[0].OperationID != "event-004" || second.Entries[4].OperationID != "event-000" || second.NextOffset != 25 || second.HasMore {
		t.Fatalf("second page = %#v", second)
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

func TestReadLogsTruncatesFilteredWindowCursor(t *testing.T) {
	other := operationEvent("other", domain.OutcomeCommitted)
	other.Source = &domain.SourceIdentity{SourceKey: "https://example.test/other"}
	first := operationEvent("first", domain.OutcomeCommitted)
	first.Source = &domain.SourceIdentity{SourceKey: "https://example.test/current"}
	second := operationEvent("second", domain.OutcomeCommitted)
	second.Source = first.Source
	second.Actor = strings.Repeat("x", 256)
	window := scopedSynthesisEvents([]Operation{other, first, other, second}, map[string]agentSource{
		"https://example.test/current": {LatestFilename: "current.md"},
	})
	fullPage := OperationPage{Directory: "notes", Entries: window, Offset: 0, Limit: 2, NextOffset: 2}
	newestPage := fullPage
	newestPage.Entries = window[1:2]
	newestPage.NextOffset = 1
	newestPage.HasMore = true
	maxData, err := json.Marshal(newestPage)
	if err != nil {
		t.Fatal(err)
	}
	contextState := &agentContext{directory: "notes", maxOutputBytes: len(maxData), logEvents: window, logWindowLoaded: true}
	data, err := readLogs(contextState, 0, 2)
	if err != nil {
		t.Fatal(err)
	}
	var got OperationPage
	if err := json.Unmarshal([]byte(data), &got); err != nil {
		t.Fatal(err)
	}
	if len(got.Entries) != 1 || got.Entries[0].OperationID != "second" || got.NextOffset != 1 || !got.HasMore {
		t.Fatalf("truncated filtered page = %#v", got)
	}
	data, err = readLogs(contextState, got.NextOffset, 2)
	if err != nil {
		t.Fatal(err)
	}
	var next OperationPage
	if err := json.Unmarshal([]byte(data), &next); err != nil {
		t.Fatal(err)
	}
	if len(next.Entries) != 1 || next.Entries[0].OperationID != "first" || next.NextOffset != 2 || next.HasMore || next.Entries[0].OperationID == got.Entries[0].OperationID {
		t.Fatalf("next truncated filtered page = %#v", next)
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
