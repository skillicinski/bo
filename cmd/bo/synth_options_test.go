package main

import (
	"errors"
	"testing"

	"github.com/skillicinski/bo"
)

func TestParseSynthOptions(t *testing.T) {
	config, err := parseSynthOptions([]string{"--max-turns", "2", "--max-tool-calls", "3", "--max-tool-output-bytes", "4", "--max-response-tokens", "5", "--timeout-seconds", "6"})
	if err != nil {
		t.Fatal(err)
	}
	if config.MaxTurns != 2 || config.MaxToolCalls != 3 || config.MaxToolOutputBytes != 4 || config.MaxResponseTokens != 5 || config.TimeoutSeconds != 6 {
		t.Fatalf("config = %#v", config)
	}
	if _, err := parseSynthOptions([]string{"--unknown", "1"}); err == nil {
		t.Fatal("unknown option succeeded")
	}
	if _, err := parseSynthOptions([]string{"--max-turns"}); err == nil {
		t.Fatal("missing value succeeded")
	}
	if _, err := parseSynthOptions([]string{"--max-turns", "zero"}); err == nil {
		t.Fatal("zero succeeded")
	}
}

func TestAddSeedHintUsesErrorKind(t *testing.T) {
	err := bo.NewError(bo.ErrorKindMissingResource, "target is missing")
	hinted := addSeedHint(err, "notes")
	if !errors.Is(hinted, err) {
		t.Fatalf("hinted error = %v", hinted)
	}
	if got := hinted.Error(); got == err.Error() {
		t.Fatal("missing-resource hint was not added")
	}
	other := bo.NewError(bo.ErrorKindFilesystem, "permission denied")
	if addSeedHint(other, "notes") != other {
		t.Fatal("filesystem error received a seed hint")
	}
}
