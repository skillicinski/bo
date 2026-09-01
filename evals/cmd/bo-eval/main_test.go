package main

import (
	"os"
	"testing"
)

func TestParseCaptureArgs(t *testing.T) {
	options, err := parseCaptureArgs([]string{"--name", "capture", "--corpus", "corpus.txt"})
	if err != nil || options.name != "capture" || options.corpus != "corpus.txt" {
		t.Fatalf("parseCaptureArgs = %#v, %v", options, err)
	}
	if _, err := parseCaptureArgs([]string{"--name", "capture"}); err == nil {
		t.Fatal("missing corpus succeeded")
	}
}

func TestParseRunArgs(t *testing.T) {
	options, err := parseRunArgs([]string{"--name", "trial", "--workflow", "end-to-end", "--provider", "gemini"})
	if err != nil || options.name != "trial" || options.workflow != "end-to-end" || options.provider != "gemini" {
		t.Fatalf("parseRunArgs = %#v, %v", options, err)
	}
	if _, err := parseRunArgs([]string{"--name", "trial", "--workflow", "unknown"}); err == nil {
		t.Fatal("unknown workflow succeeded")
	}
}

func TestReadCorpusIgnoresCommentsAndBlankLines(t *testing.T) {
	path := t.TempDir() + "/corpus.txt"
	if err := writeTestFile(path, "# comment\n\nhttps://example.com\nnotes.md\n"); err != nil {
		t.Fatal(err)
	}
	sources, err := readCorpus(path)
	if err != nil || len(sources) != 2 || sources[0] != "https://example.com" || sources[1] != "notes.md" {
		t.Fatalf("readCorpus = %#v, %v", sources, err)
	}
}

func writeTestFile(path, contents string) error {
	return os.WriteFile(path, []byte(contents), 0o600)
}
