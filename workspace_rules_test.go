package bo_test

import (
	"testing"
	"time"

	"github.com/skillicinski/bo"
)

func TestPublicWorkspaceRules(t *testing.T) {
	timeOne := time.Date(2026, 8, 30, 12, 0, 0, 0, time.UTC)
	one := bo.SnapshotCommit{
		SourceKey: "https://example.test/one", Filename: "one.md", WrittenAt: timeOne, Contents: []byte("one"),
		Event: workspaceRuleEvent("snap-one", timeOne, bo.CommandSnap, bo.RawRef("one.md"), "https://example.test/one", nil),
	}
	state, err := (bo.State{}).ApplySnapshot(one)
	if err != nil {
		t.Fatal(err)
	}
	two := one
	two.SourceKey, two.Filename, two.Contents = "https://example.test/two", "two.md", []byte("two")
	two.Event = workspaceRuleEvent("snap-two", timeOne.Add(time.Minute), bo.CommandSnap, bo.RawRef("two.md"), two.SourceKey, nil)
	state, err = state.ApplySnapshot(two)
	if err != nil {
		t.Fatal(err)
	}
	summaryTime := timeOne.Add(time.Hour)
	summary := bo.SummaryCommit{
		SourceKey: one.SourceKey, Filename: "one-summary.md", DerivedFrom: "one.md", RawWrittenAt: timeOne,
		CreatedAt: summaryTime, UpdatedAt: summaryTime, Contents: []byte("summary"),
		Event: workspaceRuleEvent("summary-one", summaryTime, bo.CommandWriteSummary, bo.SummaryRef("one-summary.md"), one.SourceKey, &bo.OperationProvenance{
			DerivedFrom: &bo.DocumentIdentity{Kind: bo.DocumentKindRaw, Filename: "one.md"}, RawWrittenAt: &timeOne,
		}),
	}
	state, err = state.ApplySummary(summary)
	if err != nil {
		t.Fatal(err)
	}
	distillTime := summaryTime.Add(time.Hour)
	distillation := bo.DistillationCommit{
		Kind: bo.DocumentKindDistillation, Filename: "facts.md", Topic: "shared-facts", CreatedAt: distillTime, UpdatedAt: distillTime,
		DerivedFrom: []bo.DistillationInput{
			{SourceKey: one.SourceKey, Kind: bo.DocumentKindRaw, Filename: "one.md", ContentDigest: bo.NewRevision(one.Contents).String()},
			{SourceKey: two.SourceKey, Kind: bo.DocumentKindRaw, Filename: "two.md", ContentDigest: bo.NewRevision(two.Contents).String()},
		}, Contents: []byte("facts"),
		Event: workspaceRuleEvent("distill", distillTime, bo.CommandWriteDistillation, bo.DistillationRef("facts.md"), "", nil),
	}
	state, err = state.ApplyDistillation(distillation)
	if err != nil {
		t.Fatal(err)
	}
	if err := state.Validate(); err != nil {
		t.Fatalf("state validation = %v", err)
	}
	invalidName := string([]byte{0xff, '.', 'm', 'd'})
	if err := bo.ValidateDocumentName(invalidName); !bo.IsKind(err, bo.ErrorKindValidation) {
		t.Fatalf("document validation error = %v", err)
	}
	invalidState := bo.State{Sources: []bo.SourceRecord{{SourceKey: one.SourceKey, Snapshots: []bo.RawRecord{{Filename: invalidName, WrittenAt: timeOne}}}}}
	if err := invalidState.Validate(); !bo.IsKind(err, bo.ErrorKindValidation) {
		t.Fatalf("state validation error = %v", err)
	}
	invalidOperation := workspaceRuleEvent("valid", timeOne, bo.CommandSnap, bo.RawRef("one.md"), one.SourceKey, nil)
	invalidOperation.OperationID = string([]byte{0xff})
	if err := invalidOperation.Validate(); !bo.IsKind(err, bo.ErrorKindValidation) {
		t.Fatalf("operation validation error = %v", err)
	}
	updated := distillation
	updated.Update = true
	updated.UpdatedAt = distillTime.Add(time.Minute)
	updated.Event = workspaceRuleEvent("distill-update", updated.UpdatedAt, bo.CommandWriteDistillation, bo.DistillationRef("facts.md"), "", nil)
	state, err = state.ApplyDistillation(updated)
	if err != nil || !state.DistillationDocuments[0].CreatedAt.Equal(distillTime) {
		t.Fatalf("distillation update = %#v, error = %v", state.DistillationDocuments, err)
	}
}

func workspaceRuleEvent(id string, timestamp time.Time, command bo.OperationCommand, document bo.DocumentRef, source string, provenance *bo.OperationProvenance) bo.Operation {
	event := bo.Operation{
		OperationID: id, Attempt: 1, Timestamp: timestamp.Format(time.RFC3339Nano), Actor: "test",
		Command: command, Outcome: bo.OutcomeCommitted,
		Document: &bo.DocumentIdentity{Kind: document.Kind, Filename: document.Name}, Provenance: provenance,
	}
	if source != "" {
		event.Source = &bo.SourceIdentity{SourceKey: source}
	}
	return event
}
