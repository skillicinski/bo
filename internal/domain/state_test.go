package domain_test

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"reflect"
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

func TestSynthesizedStateJSONAndValidation(t *testing.T) {
	validTime := time.Date(2026, time.August, 23, 12, 34, 56, 0, time.UTC)
	state := validSynthesizedState(validTime)
	data, err := domain.MarshalState(state)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), `"synthesized_documents"`) {
		t.Fatalf("synthesized state field is missing: %s", data)
	}
	roundTrip, err := domain.UnmarshalState(data)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(state, roundTrip) {
		t.Fatalf("state changed after round trip: %#v != %#v", state, roundTrip)
	}
	oldState, err := domain.UnmarshalState([]byte(`{"sources":[]}`))
	if err != nil || len(oldState.SynthesizedDocuments) != 0 {
		t.Fatalf("state without synthesized_documents = %#v, %v", oldState, err)
	}

	cases := []struct {
		name   string
		mutate func(*domain.State)
	}{
		{name: "invalid synthesized kind", mutate: func(state *domain.State) { state.SynthesizedDocuments[0].Kind = "unknown" }},
		{name: "invalid input kind", mutate: func(state *domain.State) {
			state.SynthesizedDocuments[0].DerivedFrom[0].Kind = domain.DocumentKindSynthesized
		}},
		{name: "invalid input digest", mutate: func(state *domain.State) { state.SynthesizedDocuments[0].DerivedFrom[0].ContentDigest = "not-a-digest" }},
		{name: "missing input", mutate: func(state *domain.State) { state.SynthesizedDocuments[0].DerivedFrom[0].Filename = "missing.md" }},
		{name: "wrong input owner", mutate: func(state *domain.State) {
			state.SynthesizedDocuments[0].DerivedFrom[0].SourceKey = "https://example.test/two"
		}},
		{name: "duplicate input", mutate: func(state *domain.State) {
			state.SynthesizedDocuments[0].DerivedFrom = append(state.SynthesizedDocuments[0].DerivedFrom, state.SynthesizedDocuments[0].DerivedFrom[0])
		}},
		{name: "one source identity", mutate: func(state *domain.State) {
			state.SynthesizedDocuments[0].DerivedFrom = state.SynthesizedDocuments[0].DerivedFrom[:2]
		}},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			candidate := validSynthesizedState(validTime)
			test.mutate(&candidate)
			if _, err := domain.MarshalState(candidate); err == nil {
				t.Fatal("invalid synthesized state was accepted")
			}
		})
	}
}

func validSynthesizedState(timestamp time.Time) domain.State {
	return domain.State{Sources: []domain.SourceRecord{
		{
			SourceKey: "https://example.test/one",
			Snapshots: []domain.RawRecord{{Filename: "one.md", WrittenAt: timestamp}},
			Summary:   &domain.SummaryRecord{Filename: "one-summary.md", DerivedFrom: "one.md", CreatedAt: timestamp, UpdatedAt: timestamp},
		},
		{SourceKey: "https://example.test/two", Snapshots: []domain.RawRecord{{Filename: "two.md", WrittenAt: timestamp.Add(time.Second)}}},
	}, SynthesizedDocuments: []domain.SynthesizedRecord{{
		Filename: "distill.md", Kind: domain.SynthesizedKindDistill, CreatedAt: timestamp, UpdatedAt: timestamp,
		DerivedFrom: []domain.SynthesizedInput{
			{SourceKey: "https://example.test/one", Kind: domain.DocumentKindRaw, Filename: "one.md", ContentDigest: testDigest("one\n")},
			{SourceKey: "https://example.test/one", Kind: domain.DocumentKindSummary, Filename: "one-summary.md", ContentDigest: testDigest("one summary\n")},
			{SourceKey: "https://example.test/two", Kind: domain.DocumentKindRaw, Filename: "two.md", ContentDigest: testDigest("two\n")},
		},
	}}}
}

func testDigest(value string) string {
	digest := sha256.Sum256([]byte(value))
	return hex.EncodeToString(digest[:])
}

func TestStateBaselineMetadataRoundTripsZeroLength(t *testing.T) {
	writtenAt := time.Date(2026, time.August, 23, 12, 34, 56, 123456789, time.UTC)
	size := int64(0)
	state := domain.State{Sources: []domain.SourceRecord{{
		SourceKey: "https://example.test/article",
		Snapshots: []domain.RawRecord{{
			Filename:          "article.md",
			WrittenAt:         writtenAt,
			ContentDigest:     "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
			ContentSize:       &size,
			ContentModifiedAt: "2026-08-23T12:34:56.123456789Z",
		}},
		Summary: &domain.SummaryRecord{
			Filename:          "article.md",
			DerivedFrom:       "article.md",
			CreatedAt:         writtenAt,
			UpdatedAt:         writtenAt,
			ContentDigest:     "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
			ContentSize:       &size,
			ContentModifiedAt: "2026-08-23T12:34:56.123456789Z",
		},
	}}}
	data, err := domain.MarshalState(state)
	if err != nil {
		t.Fatal(err)
	}
	roundTrip, err := domain.UnmarshalState(data)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(state, roundTrip) {
		t.Fatalf("state changed after round trip: %#v != %#v", state, roundTrip)
	}
	dataAgain, err := domain.MarshalState(roundTrip)
	if err != nil {
		t.Fatal(err)
	}
	if string(dataAgain) != string(data) {
		t.Fatalf("state JSON changed after round trip: %s != %s", dataAgain, data)
	}
}

func TestStateValidationRejectsInvalidBaselineMetadata(t *testing.T) {
	validDigest := "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
	validModifiedAt := "2026-08-23T12:34:56Z"
	validState := func() domain.State {
		return domain.State{Sources: []domain.SourceRecord{{
			SourceKey: "https://example.test/article",
			Snapshots: []domain.RawRecord{{
				Filename: "article.md", WrittenAt: time.Date(2026, time.August, 23, 12, 34, 56, 0, time.UTC),
				ContentDigest: validDigest, ContentSize: int64Pointer(0), ContentModifiedAt: validModifiedAt,
			}},
			Summary: &domain.SummaryRecord{
				Filename: "article.md", DerivedFrom: "article.md",
				CreatedAt:     time.Date(2026, time.August, 23, 12, 34, 56, 0, time.UTC),
				UpdatedAt:     time.Date(2026, time.August, 23, 12, 34, 56, 0, time.UTC),
				ContentDigest: validDigest, ContentSize: int64Pointer(0), ContentModifiedAt: validModifiedAt,
			},
		}}}
	}
	cases := []struct {
		name   string
		mutate func(*domain.State)
	}{
		{name: "raw invalid digest", mutate: func(state *domain.State) { state.Sources[0].Snapshots[0].ContentDigest = "not-a-digest" }},
		{name: "summary invalid digest", mutate: func(state *domain.State) { state.Sources[0].Summary.ContentDigest = "not-a-digest" }},
		{name: "raw invalid timestamp", mutate: func(state *domain.State) { state.Sources[0].Snapshots[0].ContentModifiedAt = "not-a-timestamp" }},
		{name: "summary non-UTC timestamp", mutate: func(state *domain.State) { state.Sources[0].Summary.ContentModifiedAt = "2026-08-23T12:34:56+01:00" }},
		{name: "raw negative size", mutate: func(state *domain.State) { *state.Sources[0].Snapshots[0].ContentSize = -1 }},
		{name: "summary negative size", mutate: func(state *domain.State) { *state.Sources[0].Summary.ContentSize = -1 }},
		{name: "raw partial digest", mutate: func(state *domain.State) { state.Sources[0].Snapshots[0].ContentDigest = "" }},
		{name: "summary partial timestamp", mutate: func(state *domain.State) { state.Sources[0].Summary.ContentModifiedAt = "" }},
		{name: "raw partial zero size", mutate: func(state *domain.State) {
			state.Sources[0].Snapshots[0].ContentDigest = ""
			state.Sources[0].Snapshots[0].ContentSize = int64Pointer(0)
			state.Sources[0].Snapshots[0].ContentModifiedAt = ""
		}},
		{name: "summary partial zero size", mutate: func(state *domain.State) {
			state.Sources[0].Summary.ContentDigest = ""
			state.Sources[0].Summary.ContentSize = int64Pointer(0)
			state.Sources[0].Summary.ContentModifiedAt = ""
		}},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			state := validState()
			test.mutate(&state)
			if _, err := domain.MarshalState(state); err == nil {
				t.Fatal("invalid baseline metadata was accepted")
			}
		})
	}
}

func int64Pointer(value int64) *int64 {
	return &value
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
