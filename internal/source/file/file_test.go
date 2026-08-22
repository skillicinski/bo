package file_test

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source/file"
)

func TestMarkdownPluginPreservesContentAndUsesH1Title(t *testing.T) {
	directory := t.TempDir()
	name := "note.md"
	contents := []byte("intro\n\n# Chosen title\n\nbody\n")
	if err := os.WriteFile(filepath.Join(directory, name), contents, 0o600); err != nil {
		t.Fatal(err)
	}
	origin, err := file.NewTransport().Route(context.Background(), filepath.Join(directory, name))
	if err != nil {
		t.Fatal(err)
	}
	snapshot, err := file.NewMarkdownPlugin().Handle(context.Background(), origin)
	if err != nil || snapshot.SourceKey != "raw:"+name || snapshot.Title != "Chosen title" || string(snapshot.Markdown) != string(contents) {
		t.Fatalf("snapshot = %#v, err = %v", snapshot, err)
	}
}

func TestMarkdownPluginUsesFilenameAndRejectsEmptyOrUnsupportedFiles(t *testing.T) {
	directory := t.TempDir()
	empty := filepath.Join(directory, "empty.md")
	if err := os.WriteFile(empty, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	origin, err := file.NewTransport().Route(context.Background(), empty)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.NewMarkdownPlugin().Handle(context.Background(), origin); !internalerrors.IsCategory(err, internalerrors.CategoryContent) {
		t.Fatalf("empty file error = %v", err)
	}

	filename := filepath.Join(directory, "fallback.md")
	if err := os.WriteFile(filename, []byte("body\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	origin, err = file.NewTransport().Route(context.Background(), filename)
	if err != nil {
		t.Fatal(err)
	}
	snapshot, err := file.NewMarkdownPlugin().Handle(context.Background(), origin)
	if err != nil || snapshot.Title != "fallback" {
		t.Fatalf("fallback snapshot = %#v, err = %v", snapshot, err)
	}
	if _, err := file.NewTransport().Route(context.Background(), filepath.Join(directory, "note.txt")); !internalerrors.IsCategory(err, internalerrors.CategoryUnsupported) {
		t.Fatalf("unsupported extension error = %v", err)
	}
}
