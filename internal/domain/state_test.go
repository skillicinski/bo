package domain_test

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/domain"
)

func TestOperationJSONIsStableAndOmitsSensitiveContent(t *testing.T) {
	rawWrittenAt := time.Date(2026, time.August, 23, 12, 34, 56, 0, time.UTC)
	event := domain.Operation{
		OperationID: "op-42",
		Attempt:     2,
		Timestamp:   "2026-08-23T12:34:57Z",
		Actor:       "agent",
		Command:     domain.CommandWriteSummary,
		Outcome:     domain.OutcomeCommitted,
		Source:      &domain.SourceIdentity{SourceKey: "https://example.test/article"},
		Document:    &domain.DocumentIdentity{Kind: domain.DocumentKindSummary, Filename: "article.md"},
		Provenance: &domain.OperationProvenance{
			DerivedFrom:  &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: "article.md"},
			RawWrittenAt: &rawWrittenAt,
		},
		Metrics: &domain.OperationMetrics{
			Turns: 3, ToolCalls: 4, Duration: 1500 * time.Nanosecond, SummariesWritten: 1, SummariesSkipped: 2,
			Usage: &domain.TokenUsage{PromptTokens: 10, CompletionTokens: 20, TotalTokens: 30},
		},
	}
	data, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"operation_id":"op-42","attempt":2,"timestamp":"2026-08-23T12:34:57Z","actor":"agent","command":"write_summary","outcome":"committed","source":{"source_key":"https://example.test/article"},"document":{"kind":"summary","filename":"article.md"},"provenance":{"derived_from":{"kind":"raw","filename":"article.md"},"raw_written_at":"2026-08-23T12:34:56Z"},"metrics":{"turns":3,"tool_calls":4,"duration":1500,"summaries_written":1,"summaries_skipped":2,"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}}`
	if string(data) != want {
		t.Fatalf("event JSON = %s", data)
	}
	if containsAny(string(data), "markdown", "api_key", `"success"`, `"details"`) {
		t.Fatalf("event contains legacy or sensitive data: %s", data)
	}
}

func containsAny(value string, parts ...string) bool {
	for _, part := range parts {
		if strings.Contains(value, part) {
			return true
		}
	}
	return false
}

func TestStateJSONIsStable(t *testing.T) {
	writtenAt := time.Date(2026, time.August, 23, 12, 34, 56, 123456789, time.UTC)
	data, err := domain.MarshalState(domain.State{Sources: []domain.SourceRecord{{
		SourceKey: "https://example.test/article",
		Snapshots: []domain.RawRecord{{Filename: "article.md", WrittenAt: writtenAt}},
		Summary: &domain.SummaryRecord{
			Filename:    "article.md",
			DerivedFrom: "article.md",
			CreatedAt:   writtenAt,
			UpdatedAt:   writtenAt,
		},
	}}})
	if err != nil {
		t.Fatal(err)
	}
	want := "{\n  \"sources\": [\n    {\n      \"source_key\": \"https://example.test/article\",\n      \"snapshots\": [\n        {\n          \"filename\": \"article.md\",\n          \"written_at\": \"2026-08-23T12:34:56.123456789Z\"\n        }\n      ],\n      \"summary\": {\n        \"filename\": \"article.md\",\n        \"derived_from\": \"article.md\",\n        \"created_at\": \"2026-08-23T12:34:56.123456789Z\",\n        \"updated_at\": \"2026-08-23T12:34:56.123456789Z\"\n      }\n    }\n  ]\n}\n"
	if string(data) != want {
		t.Fatalf("unexpected state: %q", data)
	}
}

func TestStateValidationRejectsBrokenReferences(t *testing.T) {
	state := domain.State{Sources: []domain.SourceRecord{{
		SourceKey: "https://example.test/article",
		Snapshots: []domain.RawRecord{{Filename: "article.md", WrittenAt: time.Now().UTC()}},
		Summary: &domain.SummaryRecord{
			Filename:    "article.md",
			DerivedFrom: "missing.md",
			CreatedAt:   time.Now().UTC(),
			UpdatedAt:   time.Now().UTC(),
		},
	}}}
	if _, err := domain.MarshalState(state); err == nil {
		t.Fatal("invalid provenance was accepted")
	}
}

func TestStateValidationRejectsInvalidTrustBoundaryValues(t *testing.T) {
	validTime := time.Date(2026, time.August, 23, 12, 34, 56, 0, time.UTC)
	cases := []struct {
		name  string
		state domain.State
	}{
		{
			name:  "source key",
			state: domain.State{Sources: []domain.SourceRecord{{SourceKey: "not-a-source"}}},
		},
		{
			name:  "document name",
			state: domain.State{Sources: []domain.SourceRecord{{SourceKey: "raw:note.md", Snapshots: []domain.RawRecord{{Filename: "../note.md", WrittenAt: validTime}}}}},
		},
		{
			name:  "timestamp",
			state: domain.State{Sources: []domain.SourceRecord{{SourceKey: "raw:note.md", Snapshots: []domain.RawRecord{{Filename: "note.md"}}}}},
		},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			if _, err := domain.MarshalState(test.state); err == nil {
				t.Fatal("invalid state was accepted")
			}
		})
	}
}

func TestStateValidationAcceptsZeroOffsetTimestamps(t *testing.T) {
	fixedZone := time.FixedZone("zero", 0)
	state := domain.State{Sources: []domain.SourceRecord{{
		SourceKey: "raw:note.md",
		Snapshots: []domain.RawRecord{{Filename: "note.md", WrittenAt: time.Date(2026, time.August, 23, 12, 34, 56, 0, fixedZone)}},
	}}}
	if _, err := domain.MarshalState(state); err != nil {
		t.Fatalf("fixed zero-offset timestamp rejected: %v", err)
	}
	data := []byte(`{"sources":[{"source_key":"raw:note.md","snapshots":[{"filename":"note.md","written_at":"2026-08-23T12:34:56+00:00"}]}]}`)
	if _, err := domain.UnmarshalState(data); err != nil {
		t.Fatalf("parsed zero-offset timestamp rejected: %v", err)
	}
}

func TestValidateSourceKeyRejectsCredentialBearingURLs(t *testing.T) {
	if err := domain.ValidateSourceKey("https://user:secret@example.test/article"); err == nil {
		t.Fatal("URL user information was accepted")
	}
	for _, sourceKey := range []string{
		"https://example.test/article?X-Amz-Signature=abc123",
		"https://example.test/article?X-Goog-Credential=abc123",
		"https://example.test/article?token=abc123",
		"https://example.test/article#credential",
		"https://example.test/article#",
	} {
		if err := domain.ValidateSourceKey(sourceKey); err == nil {
			t.Fatalf("credential-bearing URL was accepted: %s", sourceKey)
		}
	}
	if err := domain.ValidateSourceKey("https://example.test/article?section=1"); err != nil {
		t.Fatalf("ordinary query URL rejected: %v", err)
	}
}

func TestUnmarshalStateRejectsOldSchema(t *testing.T) {
	if _, err := domain.UnmarshalState([]byte(`{"raw":[],"summaries":[]}`)); err == nil {
		t.Fatal("old state schema was accepted")
	}
}
