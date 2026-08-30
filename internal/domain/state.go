package domain

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/url"
	"path/filepath"
	"strings"
	"time"
	"unicode"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type State struct {
	Sources               []SourceRecord       `json:"sources"`
	DistillationDocuments []DistillationRecord `json:"distillation_documents,omitempty"`
}

// SourceRecord is the aggregate for one exact source identity.
type SourceRecord struct {
	SourceKey string         `json:"source_key"`
	Snapshots []RawRecord    `json:"snapshots"`
	Summary   *SummaryRecord `json:"summary,omitempty"`
}

type RawRecord struct {
	Filename          string    `json:"filename"`
	WrittenAt         time.Time `json:"written_at"`
	ContentDigest     string    `json:"content_digest,omitempty"`
	ContentSize       *int64    `json:"content_size,omitempty"`
	ContentModifiedAt string    `json:"content_modified_at,omitempty"`
}

type SummaryRecord struct {
	Filename          string    `json:"filename"`
	DerivedFrom       string    `json:"derived_from"`
	CreatedAt         time.Time `json:"created_at"`
	UpdatedAt         time.Time `json:"updated_at"`
	ContentDigest     string    `json:"content_digest,omitempty"`
	ContentSize       *int64    `json:"content_size,omitempty"`
	ContentModifiedAt string    `json:"content_modified_at,omitempty"`
}

type DistillationInput struct {
	SourceKey     string       `json:"source_key"`
	Kind          DocumentKind `json:"kind"`
	Filename      string       `json:"filename"`
	ContentDigest string       `json:"content_digest"`
}

type DistillationRecord struct {
	Filename          string              `json:"filename"`
	Topic             string              `json:"topic,omitempty"`
	Kind              DocumentKind        `json:"kind"`
	CreatedAt         time.Time           `json:"created_at"`
	UpdatedAt         time.Time           `json:"updated_at"`
	ContentDigest     string              `json:"content_digest,omitempty"`
	ContentSize       *int64              `json:"content_size,omitempty"`
	ContentModifiedAt string              `json:"content_modified_at,omitempty"`
	DerivedFrom       []DistillationInput `json:"derived_from"`
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
	var distillation *[]DistillationRecord
	if s.DistillationDocuments != nil {
		value := append([]DistillationRecord{}, s.DistillationDocuments...)
		distillation = &value
	}
	return json.Marshal(struct {
		Sources               []SourceRecord        `json:"sources"`
		DistillationDocuments *[]DistillationRecord `json:"distillation_documents,omitempty"`
	}{Sources: sources, DistillationDocuments: distillation})
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
		return State{}, internalerrors.Wrap(internalerrors.KindValidation, "invalid state", err)
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		if err == nil {
			return State{}, internalerrors.Validation("state contains multiple JSON values")
		}
		return State{}, internalerrors.Wrap(internalerrors.KindValidation, "invalid state", err)
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
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].source_key", sourceIndex), err)
		}
		if _, exists := sourceKeys[source.SourceKey]; exists {
			return internalerrors.Validation(fmt.Sprintf("sources[%d].source_key: duplicate source key %q", sourceIndex, source.SourceKey))
		}
		sourceKeys[source.SourceKey] = struct{}{}

		snapshots := make(map[string]struct{}, len(source.Snapshots))
		for snapshotIndex, snapshot := range source.Snapshots {
			if err := ValidateDocumentName(snapshot.Filename); err != nil {
				return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].snapshots[%d].filename", sourceIndex, snapshotIndex), err)
			}
			if _, exists := snapshots[snapshot.Filename]; exists {
				return internalerrors.Validation(fmt.Sprintf("sources[%d].snapshots[%d].filename: duplicate snapshot filename %q", sourceIndex, snapshotIndex, snapshot.Filename))
			}
			if previous, exists := rawFilenames[snapshot.Filename]; exists {
				return internalerrors.Validation(fmt.Sprintf("sources[%d].snapshots[%d].filename: already used by %s", sourceIndex, snapshotIndex, previous))
			}
			snapshots[snapshot.Filename] = struct{}{}
			rawFilenames[snapshot.Filename] = fmt.Sprintf("sources[%d].snapshots[%d]", sourceIndex, snapshotIndex)
			if err := ValidateTimestamp(snapshot.WrittenAt); err != nil {
				return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].snapshots[%d].written_at", sourceIndex, snapshotIndex), err)
			}
			if err := validateDocumentBaseline(snapshot.ContentDigest, snapshot.ContentSize, snapshot.ContentModifiedAt); err != nil {
				return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].snapshots[%d].content_baseline", sourceIndex, snapshotIndex), err)
			}
		}

		if source.Summary == nil {
			continue
		}
		summary := source.Summary
		if err := ValidateDocumentName(summary.Filename); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].summary.filename", sourceIndex), err)
		}
		if previous, exists := summaryFilenames[summary.Filename]; exists {
			return internalerrors.Validation(fmt.Sprintf("sources[%d].summary.filename: already used by %s", sourceIndex, previous))
		}
		summaryFilenames[summary.Filename] = fmt.Sprintf("sources[%d].summary", sourceIndex)
		if err := ValidateDocumentName(summary.DerivedFrom); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].summary.derived_from", sourceIndex), err)
		}
		if _, exists := snapshots[summary.DerivedFrom]; !exists {
			return internalerrors.Validation(fmt.Sprintf("sources[%d].summary.derived_from: snapshot %q does not exist for source %q", sourceIndex, summary.DerivedFrom, source.SourceKey))
		}
		if err := ValidateTimestamp(summary.CreatedAt); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].summary.created_at", sourceIndex), err)
		}
		if err := ValidateTimestamp(summary.UpdatedAt); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].summary.updated_at", sourceIndex), err)
		}
		if summary.UpdatedAt.Before(summary.CreatedAt) {
			return internalerrors.Validation(fmt.Sprintf("sources[%d].summary.updated_at: before created_at", sourceIndex))
		}
		if err := validateDocumentBaseline(summary.ContentDigest, summary.ContentSize, summary.ContentModifiedAt); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("sources[%d].summary.content_baseline", sourceIndex), err)
		}
	}
	distillationFilenames := make(map[string]string, len(s.DistillationDocuments))
	distillationTopics := make(map[string]string, len(s.DistillationDocuments))
	for index, record := range s.DistillationDocuments {
		if err := ValidateDocumentName(record.Filename); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].filename", index), err)
		}
		if previous, exists := distillationFilenames[record.Filename]; exists {
			return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].filename: already used by %s", index, previous))
		}
		distillationFilenames[record.Filename] = fmt.Sprintf("distillation_documents[%d]", index)
		if err := ValidateTopic(record.Topic); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].topic", index), err)
		}
		if previous, exists := distillationTopics[record.Topic]; exists {
			return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].topic: already used by %s", index, previous))
		}
		distillationTopics[record.Topic] = fmt.Sprintf("distillation_documents[%d]", index)
		if record.Kind != DocumentKindDistillation {
			return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].kind: invalid distillation kind %q", index, record.Kind))
		}
		if err := ValidateTimestamp(record.CreatedAt); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].created_at", index), err)
		}
		if err := ValidateTimestamp(record.UpdatedAt); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].updated_at", index), err)
		}
		if record.UpdatedAt.Before(record.CreatedAt) {
			return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].updated_at: before created_at", index))
		}
		if err := validateDocumentBaseline(record.ContentDigest, record.ContentSize, record.ContentModifiedAt); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].content_baseline", index), err)
		}
		if len(record.DerivedFrom) < 1 {
			return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].derived_from: must not be empty", index))
		}
		inputKeys := make(map[string]struct{}, len(record.DerivedFrom))
		distinctSources := make(map[string]struct{}, len(record.DerivedFrom))
		for inputIndex, input := range record.DerivedFrom {
			if err := ValidateSourceKey(input.SourceKey); err != nil {
				return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].derived_from[%d].source_key", index, inputIndex), err)
			}
			if input.Kind != DocumentKindRaw && input.Kind != DocumentKindSummary {
				return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].derived_from[%d].kind: invalid document kind %q", index, inputIndex, input.Kind))
			}
			if err := ValidateDocumentName(input.Filename); err != nil {
				return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].derived_from[%d].filename", index, inputIndex), err)
			}
			if err := validateDigest(input.ContentDigest); err != nil {
				return internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("distillation_documents[%d].derived_from[%d].content_digest", index, inputIndex), err)
			}
			key := fmt.Sprintf("%s\x00%s\x00%s", input.SourceKey, input.Kind, input.Filename)
			if _, exists := inputKeys[key]; exists {
				return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].derived_from[%d]: duplicate input", index, inputIndex))
			}
			inputKeys[key] = struct{}{}
			distinctSources[input.SourceKey] = struct{}{}
			if !s.ContainsDocument(input) {
				return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].derived_from[%d]: document %q does not belong to source %q", index, inputIndex, input.Filename, input.SourceKey))
			}
		}
		if len(distinctSources) < 2 {
			return internalerrors.Validation(fmt.Sprintf("distillation_documents[%d].derived_from: must contain at least two source identities", index))
		}
	}
	return nil
}

func validateDigest(digest string) error {
	if len(digest) != sha256.Size*2 {
		return internalerrors.Validation("document content digest must be a SHA-256 hex digest")
	}
	if _, err := hex.DecodeString(digest); err != nil {
		return internalerrors.Validation("document content digest must be a SHA-256 hex digest")
	}
	return nil
}

// ContainsDocument reports whether input names a document in its source aggregate.
func (s State) ContainsDocument(input DistillationInput) bool {
	for _, source := range s.Sources {
		if source.SourceKey != input.SourceKey {
			continue
		}
		switch input.Kind {
		case DocumentKindRaw:
			for _, snapshot := range source.Snapshots {
				if snapshot.Filename == input.Filename {
					return true
				}
			}
		case DocumentKindSummary:
			return source.Summary != nil && source.Summary.Filename == input.Filename
		}
	}
	return false
}

// ValidateContentDigest validates a document content digest.
func ValidateContentDigest(digest string) error { return validateDigest(digest) }

func ValidateSourceKey(sourceKey string) error {
	if sourceKey == "" {
		return internalerrors.Validation("source key must not be empty")
	}
	if strings.HasPrefix(sourceKey, "raw:") {
		if err := ValidateDocumentName(strings.TrimPrefix(sourceKey, "raw:")); err != nil {
			return internalerrors.Wrap(internalerrors.KindValidation, "raw source key must contain a document name", err)
		}
		return nil
	}
	parsed, err := url.Parse(sourceKey)
	if err != nil || parsed.Host == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") {
		return internalerrors.Validation("source key must be an http or https URL or raw:<filename>")
	}
	if parsed.User != nil {
		return internalerrors.Validation("source key URL must not contain user information")
	}
	if parsed.Fragment != "" || strings.Contains(sourceKey, "#") {
		return internalerrors.Validation("source key URL must not contain a fragment")
	}
	if credentialQueryParameter(parsed.RawQuery) {
		return internalerrors.Validation("source key URL must not contain credential query parameters")
	}
	for _, r := range sourceKey {
		if unicode.IsControl(r) || unicode.IsSpace(r) {
			return internalerrors.Validation("source key must not contain whitespace or control characters")
		}
	}
	return nil
}

func credentialQueryParameter(rawQuery string) bool {
	if rawQuery == "" {
		return false
	}
	values, err := url.ParseQuery(rawQuery)
	if err != nil {
		return true
	}
	for key := range values {
		normalized := strings.ToLower(strings.ReplaceAll(key, "-", "_"))
		if strings.HasPrefix(normalized, "x_amz_") || strings.HasPrefix(normalized, "x_goog_") {
			return true
		}
		switch normalized {
		case "api_key", "apikey", "auth", "authorization", "key", "jwt", "oauth", "password", "passwd", "secret", "sig", "signature", "token":
			return true
		}
		for _, marker := range []string{"access_token", "credential", "password", "secret", "signature", "token"} {
			if strings.Contains(normalized, marker) {
				return true
			}
		}
	}
	return false
}

func ValidateDocumentName(name string) error {
	if name == "" || name == "." || name == ".." || strings.ContainsAny(name, `/\`) || strings.ContainsRune(name, 0) ||
		!strings.EqualFold(filepath.Ext(name), ".md") {
		return internalerrors.Validation("document name must be a Markdown file name")
	}
	for _, r := range name {
		if unicode.IsControl(r) {
			return internalerrors.Validation("document name must not contain control characters")
		}
	}
	return nil
}

func CanonicalTopic(topic string) (string, error) {
	topic = strings.TrimSpace(topic)
	var builder strings.Builder
	lastDash := false
	for _, r := range topic {
		if unicode.IsControl(r) {
			return "", internalerrors.Validation("topic must not contain control characters")
		}
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			builder.WriteRune(unicode.ToLower(r))
			lastDash = false
		} else if builder.Len() > 0 && !lastDash {
			builder.WriteByte('-')
			lastDash = true
		}
	}
	canonical := strings.TrimRight(builder.String(), "-")
	if canonical == "" {
		return "", internalerrors.Validation("topic must contain letters or digits")
	}
	return canonical, nil
}

func ValidateTopic(topic string) error {
	canonical, err := CanonicalTopic(topic)
	if err != nil {
		return err
	}
	if topic != canonical {
		return internalerrors.Validation("topic must use canonical kebab-case")
	}
	return nil
}

func ValidateTimestamp(timestamp time.Time) error {
	if timestamp.IsZero() {
		return internalerrors.Validation("timestamp must not be zero")
	}
	if _, offset := timestamp.Zone(); offset != 0 {
		return internalerrors.Validation("timestamp must use UTC")
	}
	return nil
}

func validateDocumentBaseline(digest string, size *int64, modifiedAt string) error {
	if size != nil && *size < 0 {
		return internalerrors.Validation("document content size must not be negative")
	}
	if digest == "" && size == nil && modifiedAt == "" {
		return nil
	}
	if digest == "" || size == nil || modifiedAt == "" {
		return internalerrors.Validation("document content baseline must include digest, size, and modification time")
	}
	if len(digest) != sha256.Size*2 {
		return internalerrors.Validation("document content digest must be a SHA-256 hex digest")
	}
	if _, err := hex.DecodeString(digest); err != nil {
		return internalerrors.Validation("document content digest must be a SHA-256 hex digest")
	}
	timestamp, err := time.Parse(time.RFC3339Nano, modifiedAt)
	if err != nil {
		return internalerrors.Validation("document content modification time must be RFC 3339")
	}
	if err := ValidateTimestamp(timestamp); err != nil {
		return internalerrors.Validation("document content modification time must use UTC")
	}
	return nil
}
