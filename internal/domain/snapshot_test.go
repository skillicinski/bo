package domain_test

import (
	"testing"

	"github.com/skillicinski/bo/internal/domain"
)

func TestNewRawSnapshotOwnsMarkdownAndSourceKey(t *testing.T) {
	markdown := []byte("# Note\n")
	snapshot := domain.NewRawSnapshot("raw:note.md", "Note", markdown)
	markdown[0] = 'x'
	if snapshot.SourceKey != "raw:note.md" || snapshot.Title != "Note" || string(snapshot.Markdown) != "# Note\n" {
		t.Fatalf("snapshot = %#v", snapshot)
	}
}
