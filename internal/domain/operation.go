package domain

import (
	"crypto/rand"
	"fmt"
	"strings"
	"time"
	"unicode"
)

type OperationCommand string

const (
	CommandSeed         OperationCommand = "seed"
	CommandSnap         OperationCommand = "snap"
	CommandState        OperationCommand = "state"
	CommandSynth        OperationCommand = "synth"
	CommandWriteSummary OperationCommand = "write_summary"
)

type OperationOutcome string

const (
	OutcomeCommitted OperationOutcome = "committed"
	OutcomeFailed    OperationOutcome = "failed"
)

type SourceIdentity struct {
	SourceKey string `json:"source_key"`
}

type DocumentIdentity struct {
	Kind     DocumentKind `json:"kind"`
	Filename string       `json:"filename"`
}

type OperationProvenance struct {
	DerivedFrom  *DocumentIdentity `json:"derived_from,omitempty"`
	RawWrittenAt *time.Time        `json:"raw_written_at,omitempty"`
}

type OperationError struct {
	Kind      string `json:"kind"`
	Retryable bool   `json:"retryable"`
}

type TokenUsage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

type OperationMetrics struct {
	Turns            int           `json:"turns"`
	ToolCalls        int           `json:"tool_calls"`
	Duration         time.Duration `json:"duration"`
	SummariesWritten int           `json:"summaries_written"`
	SummariesSkipped int           `json:"summaries_skipped"`
	Usage            *TokenUsage   `json:"usage,omitempty"`
}

// Operation is a durable, typed application event.
type Operation struct {
	OperationID string               `json:"operation_id"`
	Attempt     int                  `json:"attempt"`
	Timestamp   string               `json:"timestamp"`
	Actor       string               `json:"actor"`
	Command     OperationCommand     `json:"command"`
	Outcome     OperationOutcome     `json:"outcome"`
	Source      *SourceIdentity      `json:"source,omitempty"`
	Document    *DocumentIdentity    `json:"document,omitempty"`
	Provenance  *OperationProvenance `json:"provenance,omitempty"`
	Error       *OperationError      `json:"error,omitempty"`
	Metrics     *OperationMetrics    `json:"metrics,omitempty"`
}

func NewOperationID() string {
	return "op-" + rand.Text()
}

func (o *Operation) Normalize() {
	if o.OperationID == "" {
		o.OperationID = NewOperationID()
	}
	if o.Attempt < 1 {
		o.Attempt = 1
	}
	if o.Timestamp == "" {
		o.Timestamp = time.Now().UTC().Format(time.RFC3339Nano)
	}
	if o.Actor == "" {
		o.Actor = "system"
	}
	if o.Outcome == "" {
		o.Outcome = OutcomeFailed
	}
}

func (o Operation) Validate() error {
	if o.OperationID == "" {
		return fmt.Errorf("operation_id must not be empty")
	}
	if strings.ContainsAny(o.OperationID, `/\\`) {
		return fmt.Errorf("operation_id must be one path component")
	}
	for _, r := range o.OperationID {
		if unicode.IsControl(r) {
			return fmt.Errorf("operation_id must not contain control characters")
		}
	}
	if o.Attempt < 1 {
		return fmt.Errorf("attempt must be positive")
	}
	parsed, err := time.Parse(time.RFC3339Nano, o.Timestamp)
	if err != nil {
		return fmt.Errorf("timestamp must be RFC3339: %w", err)
	}
	if _, offset := parsed.Zone(); offset != 0 {
		return fmt.Errorf("timestamp must use UTC")
	}
	if o.Actor == "" {
		return fmt.Errorf("actor must not be empty")
	}
	switch o.Command {
	case CommandSeed, CommandSnap, CommandState, CommandSynth, CommandWriteSummary:
	default:
		return fmt.Errorf("invalid command %q", o.Command)
	}
	if o.Outcome != OutcomeCommitted && o.Outcome != OutcomeFailed {
		return fmt.Errorf("invalid outcome %q", o.Outcome)
	}
	if o.Outcome == OutcomeCommitted && o.Error != nil {
		return fmt.Errorf("committed operation must not contain an error")
	}
	if o.Outcome == OutcomeFailed && o.Error == nil {
		return fmt.Errorf("failed operation must contain an error")
	}
	if o.Error != nil {
		if !validOperationErrorKind(o.Error.Kind) {
			return fmt.Errorf("invalid error kind %q", o.Error.Kind)
		}
	}
	if o.Source != nil {
		if err := ValidateSourceKey(o.Source.SourceKey); err != nil {
			return fmt.Errorf("source: %w", err)
		}
	}
	if o.Document != nil {
		if o.Document.Kind != DocumentKindRaw && o.Document.Kind != DocumentKindSummary {
			return fmt.Errorf("document kind is invalid")
		}
		if err := ValidateDocumentName(o.Document.Filename); err != nil {
			return fmt.Errorf("document: %w", err)
		}
	}
	if o.Provenance != nil {
		if o.Provenance.DerivedFrom != nil {
			if o.Provenance.DerivedFrom.Kind != DocumentKindRaw && o.Provenance.DerivedFrom.Kind != DocumentKindSummary {
				return fmt.Errorf("provenance document kind is invalid")
			}
			if err := ValidateDocumentName(o.Provenance.DerivedFrom.Filename); err != nil {
				return fmt.Errorf("provenance: %w", err)
			}
		}
		if o.Provenance.RawWrittenAt != nil {
			if err := ValidateTimestamp(*o.Provenance.RawWrittenAt); err != nil {
				return fmt.Errorf("provenance raw_written_at: %w", err)
			}
		}
	}
	if o.Metrics != nil {
		if o.Metrics.Duration < 0 {
			return fmt.Errorf("metrics duration must not be negative")
		}
		if o.Metrics.Turns < 0 || o.Metrics.ToolCalls < 0 || o.Metrics.SummariesWritten < 0 || o.Metrics.SummariesSkipped < 0 {
			return fmt.Errorf("metrics counts must not be negative")
		}
		if o.Metrics.Usage != nil && (o.Metrics.Usage.PromptTokens < 0 || o.Metrics.Usage.CompletionTokens < 0 || o.Metrics.Usage.TotalTokens < 0) {
			return fmt.Errorf("metrics usage counts must not be negative")
		}
	}
	return nil
}

func validOperationErrorKind(kind string) bool {
	switch kind {
	case "unknown", "request", "validation", "source", "filesystem", "missing_resource", "conflict", "already_exists", "provider_transport", "provider_rejected", "provider_malformed", "canceled", "deadline":
		return true
	default:
		return false
	}
}
