package domain

import (
	"strings"
	"time"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

// SnapshotCommit is the storage-neutral data for one raw document commit.
type SnapshotCommit struct {
	SourceKey string
	Filename  string
	WrittenAt time.Time
	Contents  []byte
	Event     Operation
}

// SummaryCommit is the storage-neutral data for one summary commit.
type SummaryCommit struct {
	SourceKey    string
	Filename     string
	DerivedFrom  string
	RawWrittenAt time.Time
	CreatedAt    time.Time
	UpdatedAt    time.Time
	Contents     []byte
	Event        Operation
}

// DistillationCommit is the storage-neutral data for one distillation commit.
type DistillationCommit struct {
	Kind        DocumentKind
	Filename    string
	Topic       string
	Update      bool
	CreatedAt   time.Time
	UpdatedAt   time.Time
	DerivedFrom []DistillationInput
	Contents    []byte
	Event       Operation
}

// ValidateDocumentKind validates a workspace document kind.
func ValidateDocumentKind(kind DocumentKind) error {
	if kind != DocumentKindRaw && kind != DocumentKindSummary && kind != DocumentKindDistillation {
		return internalerrors.Validation("unsupported document kind")
	}
	return nil
}

func (commit SnapshotCommit) Validate() error {
	if err := ValidateSourceKey(commit.SourceKey); err != nil {
		return err
	}
	if err := ValidateDocumentName(commit.Filename); err != nil {
		return err
	}
	if err := ValidateTimestamp(commit.WrittenAt); err != nil {
		return err
	}
	return validateMutationEvent(commit.Event, CommandSnap, commit.SourceKey, DocumentKindRaw, commit.Filename, nil)
}

func (commit SummaryCommit) Validate() error {
	if err := ValidateSourceKey(commit.SourceKey); err != nil {
		return err
	}
	if err := ValidateDocumentName(commit.Filename); err != nil {
		return err
	}
	if err := ValidateDocumentName(commit.DerivedFrom); err != nil {
		return err
	}
	if err := ValidateTimestamp(commit.RawWrittenAt); err != nil {
		return err
	}
	if err := ValidateTimestamp(commit.CreatedAt); err != nil {
		return err
	}
	if err := ValidateTimestamp(commit.UpdatedAt); err != nil {
		return err
	}
	return validateMutationEvent(commit.Event, CommandWriteSummary, commit.SourceKey, DocumentKindSummary, commit.Filename, &summaryEventProvenance{
		derivedFrom:  commit.DerivedFrom,
		rawWrittenAt: commit.RawWrittenAt,
	})
}

func (commit DistillationCommit) Validate() error {
	if commit.Kind != DocumentKindDistillation {
		return internalerrors.Validation("invalid distillation kind")
	}
	if err := ValidateDocumentName(commit.Filename); err != nil {
		return err
	}
	if err := ValidateTopic(commit.Topic); err != nil {
		return err
	}
	if err := ValidateTimestamp(commit.CreatedAt); err != nil {
		return err
	}
	if err := ValidateTimestamp(commit.UpdatedAt); err != nil {
		return err
	}
	if commit.UpdatedAt.Before(commit.CreatedAt) {
		return internalerrors.Validation("distillation updated_at is before created_at")
	}
	if len(commit.DerivedFrom) == 0 {
		return internalerrors.Validation("distillation document must have inputs")
	}
	if strings.TrimSpace(string(commit.Contents)) == "" {
		return internalerrors.Validation("distillation document must be non-empty")
	}
	for _, input := range commit.DerivedFrom {
		if input.Kind != DocumentKindRaw && input.Kind != DocumentKindSummary {
			return internalerrors.Validation("invalid distillation input kind")
		}
		if err := ValidateSourceKey(input.SourceKey); err != nil {
			return err
		}
		if err := ValidateDocumentName(input.Filename); err != nil {
			return err
		}
		if err := ValidateContentDigest(input.ContentDigest); err != nil {
			return err
		}
	}
	return validateMutationEvent(commit.Event, CommandWriteDistillation, "", DocumentKindDistillation, commit.Filename, nil)
}

type summaryEventProvenance struct {
	derivedFrom  string
	rawWrittenAt time.Time
}

func validateMutationEvent(event Operation, command OperationCommand, sourceKey string, kind DocumentKind, filename string, provenance *summaryEventProvenance) error {
	if err := event.Validate(); err != nil {
		return internalerrors.Wrap(internalerrors.KindValidation, "invalid mutation event", err)
	}
	if event.Command != command {
		return internalerrors.Validation("mutation event command does not match commit")
	}
	if event.Outcome != OutcomeCommitted || event.Error != nil {
		return internalerrors.Validation("mutation event must be committed without an error")
	}
	if command == CommandWriteDistillation {
		if event.Source != nil || event.Provenance != nil {
			return internalerrors.Validation("distillation mutation event must not contain source or provenance")
		}
	} else if event.Source == nil || event.Source.SourceKey != sourceKey {
		return internalerrors.Validation("mutation event source does not match commit")
	}
	if event.Document == nil || event.Document.Kind != kind || event.Document.Filename != filename {
		return internalerrors.Validation("mutation event document does not match commit")
	}
	if provenance == nil {
		if event.Provenance != nil {
			return internalerrors.Validation("mutation event provenance is not allowed")
		}
		return nil
	}
	if event.Provenance == nil || event.Provenance.DerivedFrom == nil ||
		event.Provenance.DerivedFrom.Kind != DocumentKindRaw ||
		event.Provenance.DerivedFrom.Filename != provenance.derivedFrom ||
		event.Provenance.RawWrittenAt == nil ||
		!event.Provenance.RawWrittenAt.Equal(provenance.rawWrittenAt) {
		return internalerrors.Validation("mutation event provenance does not match commit")
	}
	return nil
}

// ApplySnapshot returns the state after a validated raw document commit.
func (s State) ApplySnapshot(commit SnapshotCommit) (State, error) {
	next := cloneState(s)
	for index := range next.Sources {
		if next.Sources[index].SourceKey != commit.SourceKey {
			continue
		}
		for _, snapshot := range next.Sources[index].Snapshots {
			if snapshot.Filename == commit.Filename {
				return State{}, internalerrors.AlreadyExists("snapshot already belongs to workspace state")
			}
		}
		next.Sources[index].Snapshots = append(next.Sources[index].Snapshots, RawRecord{Filename: commit.Filename, WrittenAt: commit.WrittenAt})
		return next, next.Validate()
	}
	next.Sources = append(next.Sources, SourceRecord{
		SourceKey: commit.SourceKey,
		Snapshots: []RawRecord{{Filename: commit.Filename, WrittenAt: commit.WrittenAt}},
	})
	return next, next.Validate()
}

// ApplySummary returns the state after a validated summary commit.
func (s State) ApplySummary(commit SummaryCommit) (State, error) {
	next := cloneState(s)
	record := &SummaryRecord{Filename: commit.Filename, DerivedFrom: commit.DerivedFrom, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt}
	for index := range next.Sources {
		if next.Sources[index].SourceKey != commit.SourceKey {
			continue
		}
		if !hasSnapshot(next.Sources[index].Snapshots, commit.DerivedFrom) {
			if len(next.Sources[index].Snapshots) != 0 {
				return State{}, internalerrors.Validation("summary must derive from a workspace snapshot")
			}
			next.Sources[index].Snapshots = []RawRecord{{Filename: commit.DerivedFrom, WrittenAt: commit.RawWrittenAt}}
		}
		if next.Sources[index].Summary != nil && next.Sources[index].Summary.Filename != commit.Filename {
			return State{}, internalerrors.Validation("summary filename cannot change")
		}
		next.Sources[index].Summary = record
		return next, next.Validate()
	}
	next.Sources = append(next.Sources, SourceRecord{
		SourceKey: commit.SourceKey,
		Snapshots: []RawRecord{{Filename: commit.DerivedFrom, WrittenAt: commit.RawWrittenAt}},
		Summary:   record,
	})
	return next, next.Validate()
}

// ApplyDistillation returns the state after a validated distillation commit.
func (s State) ApplyDistillation(commit DistillationCommit) (State, error) {
	next := cloneState(s)
	record := DistillationRecord{
		Filename: commit.Filename, Topic: commit.Topic, Kind: commit.Kind,
		CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt,
		DerivedFrom: append([]DistillationInput(nil), commit.DerivedFrom...),
	}
	for index := range next.DistillationDocuments {
		if next.DistillationDocuments[index].Filename != commit.Filename {
			continue
		}
		if !commit.Update {
			return State{}, internalerrors.AlreadyExists("distillation document already belongs to workspace state")
		}
		if next.DistillationDocuments[index].Topic != record.Topic {
			return State{}, internalerrors.Validation("distillation topic cannot change on update")
		}
		record.CreatedAt = next.DistillationDocuments[index].CreatedAt
		next.DistillationDocuments[index] = record
		return next, next.Validate()
	}
	if commit.Update {
		return State{}, internalerrors.Conflict("distillation document exists outside workspace state")
	}
	next.DistillationDocuments = append(next.DistillationDocuments, record)
	return next, next.Validate()
}

func cloneState(state State) State {
	next := State{Sources: make([]SourceRecord, len(state.Sources))}
	if state.DistillationDocuments != nil {
		next.DistillationDocuments = make([]DistillationRecord, len(state.DistillationDocuments))
	}
	for index, source := range state.Sources {
		next.Sources[index] = SourceRecord{SourceKey: source.SourceKey, Snapshots: append([]RawRecord(nil), source.Snapshots...)}
		if source.Summary != nil {
			summary := *source.Summary
			next.Sources[index].Summary = &summary
		}
	}
	for index, record := range state.DistillationDocuments {
		next.DistillationDocuments[index] = record
		next.DistillationDocuments[index].DerivedFrom = append([]DistillationInput(nil), record.DerivedFrom...)
	}
	return next
}

func hasSnapshot(snapshots []RawRecord, filename string) bool {
	for _, snapshot := range snapshots {
		if snapshot.Filename == filename {
			return true
		}
	}
	return false
}
