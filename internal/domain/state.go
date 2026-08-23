package domain

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/url"
	"path/filepath"
	"strings"
	"time"
	"unicode"
)

type State struct {
	Sources []SourceRecord `json:"sources"`
}

// SourceRecord is the aggregate for one exact source identity.
type SourceRecord struct {
	SourceKey string         `json:"source_key"`
	Snapshots []RawRecord    `json:"snapshots"`
	Summary   *SummaryRecord `json:"summary,omitempty"`
}

type RawRecord struct {
	Filename  string    `json:"filename"`
	WrittenAt time.Time `json:"written_at"`
}

type SummaryRecord struct {
	Filename    string    `json:"filename"`
	DerivedFrom string    `json:"derived_from"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
}

func (s State) MarshalJSON() ([]byte, error) {
	if err := s.Validate(); err != nil {
		return nil, err
	}
	sources := make([]SourceRecord, len(s.Sources))
	copy(sources, s.Sources)
	for index := range sources {
		if sources[index].Snapshots == nil {
			sources[index].Snapshots = []RawRecord{}
		}
	}
	type state State
	return json.Marshal(state{Sources: sources})
}

func MarshalState(state State) ([]byte, error) {
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(data, '\n'), nil
}

func UnmarshalState(data []byte) (State, error) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	var state State
	if err := decoder.Decode(&state); err != nil {
		return State{}, err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		if err == nil {
			return State{}, fmt.Errorf("state contains multiple JSON values")
		}
		return State{}, err
	}
	if state.Sources == nil {
		state.Sources = []SourceRecord{}
	}
	for index := range state.Sources {
		if state.Sources[index].Snapshots == nil {
			state.Sources[index].Snapshots = []RawRecord{}
		}
	}
	if err := state.Validate(); err != nil {
		return State{}, err
	}
	return state, nil
}

func (s State) Validate() error {
	sourceKeys := make(map[string]struct{}, len(s.Sources))
	rawFilenames := make(map[string]string)
	summaryFilenames := make(map[string]string)
	for sourceIndex, source := range s.Sources {
		if err := ValidateSourceKey(source.SourceKey); err != nil {
			return fmt.Errorf("sources[%d].source_key: %w", sourceIndex, err)
		}
		if _, exists := sourceKeys[source.SourceKey]; exists {
			return fmt.Errorf("sources[%d].source_key: duplicate source key %q", sourceIndex, source.SourceKey)
		}
		sourceKeys[source.SourceKey] = struct{}{}

		snapshots := make(map[string]struct{}, len(source.Snapshots))
		for snapshotIndex, snapshot := range source.Snapshots {
			if err := ValidateDocumentName(snapshot.Filename); err != nil {
				return fmt.Errorf("sources[%d].snapshots[%d].filename: %w", sourceIndex, snapshotIndex, err)
			}
			if _, exists := snapshots[snapshot.Filename]; exists {
				return fmt.Errorf("sources[%d].snapshots[%d].filename: duplicate snapshot filename %q", sourceIndex, snapshotIndex, snapshot.Filename)
			}
			if previous, exists := rawFilenames[snapshot.Filename]; exists {
				return fmt.Errorf("sources[%d].snapshots[%d].filename: already used by %s", sourceIndex, snapshotIndex, previous)
			}
			snapshots[snapshot.Filename] = struct{}{}
			rawFilenames[snapshot.Filename] = fmt.Sprintf("sources[%d].snapshots[%d]", sourceIndex, snapshotIndex)
			if err := ValidateTimestamp(snapshot.WrittenAt); err != nil {
				return fmt.Errorf("sources[%d].snapshots[%d].written_at: %w", sourceIndex, snapshotIndex, err)
			}
		}

		if source.Summary == nil {
			continue
		}
		summary := source.Summary
		if err := ValidateDocumentName(summary.Filename); err != nil {
			return fmt.Errorf("sources[%d].summary.filename: %w", sourceIndex, err)
		}
		if previous, exists := summaryFilenames[summary.Filename]; exists {
			return fmt.Errorf("sources[%d].summary.filename: already used by %s", sourceIndex, previous)
		}
		summaryFilenames[summary.Filename] = fmt.Sprintf("sources[%d].summary", sourceIndex)
		if err := ValidateDocumentName(summary.DerivedFrom); err != nil {
			return fmt.Errorf("sources[%d].summary.derived_from: %w", sourceIndex, err)
		}
		if _, exists := snapshots[summary.DerivedFrom]; !exists {
			return fmt.Errorf("sources[%d].summary.derived_from: snapshot %q does not exist for source %q", sourceIndex, summary.DerivedFrom, source.SourceKey)
		}
		if err := ValidateTimestamp(summary.CreatedAt); err != nil {
			return fmt.Errorf("sources[%d].summary.created_at: %w", sourceIndex, err)
		}
		if err := ValidateTimestamp(summary.UpdatedAt); err != nil {
			return fmt.Errorf("sources[%d].summary.updated_at: %w", sourceIndex, err)
		}
		if summary.UpdatedAt.Before(summary.CreatedAt) {
			return fmt.Errorf("sources[%d].summary.updated_at: before created_at", sourceIndex)
		}
	}
	return nil
}

func ValidateSourceKey(sourceKey string) error {
	if sourceKey == "" {
		return fmt.Errorf("source key must not be empty")
	}
	if strings.HasPrefix(sourceKey, "raw:") {
		if err := ValidateDocumentName(strings.TrimPrefix(sourceKey, "raw:")); err != nil {
			return fmt.Errorf("raw source key must contain a document name: %w", err)
		}
		return nil
	}
	parsed, err := url.Parse(sourceKey)
	if err != nil || parsed.Host == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") {
		return fmt.Errorf("source key must be an http or https URL or raw:<filename>")
	}
	for _, r := range sourceKey {
		if unicode.IsControl(r) || unicode.IsSpace(r) {
			return fmt.Errorf("source key must not contain whitespace or control characters")
		}
	}
	return nil
}

func ValidateDocumentName(name string) error {
	if name == "" || name == "." || name == ".." || strings.ContainsAny(name, `/\`) || strings.ContainsRune(name, 0) ||
		!strings.EqualFold(filepath.Ext(name), ".md") {
		return fmt.Errorf("document name must be a Markdown file name")
	}
	for _, r := range name {
		if unicode.IsControl(r) {
			return fmt.Errorf("document name must not contain control characters")
		}
	}
	return nil
}

func ValidateTimestamp(timestamp time.Time) error {
	if timestamp.IsZero() {
		return fmt.Errorf("timestamp must not be zero")
	}
	if _, offset := timestamp.Zone(); offset != 0 {
		return fmt.Errorf("timestamp must use UTC")
	}
	return nil
}
