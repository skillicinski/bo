package domain_test

import (
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/domain"
)

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

func TestUnmarshalStateRejectsOldSchema(t *testing.T) {
	if _, err := domain.UnmarshalState([]byte(`{"raw":[],"summaries":[]}`)); err == nil {
		t.Fatal("old state schema was accepted")
	}
}
