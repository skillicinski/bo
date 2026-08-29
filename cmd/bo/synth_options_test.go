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
	if config.MaxTurns != 2 || config.MaxToolCalls != 3 || config.MaxToolOutputBytes != 4 || config.MaxResponseTokens != 5 || config.RuntimeTimeoutSeconds != 6 {
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

func TestParseSynthMode(t *testing.T) {
	mode, args, err := parseSynthMode([]string{"summarize", "--max-turns", "2"})
	if err != nil || mode != bo.SynthModeSummarize || len(args) != 2 {
		t.Fatalf("summarize mode = %v, %v, %v", mode, args, err)
	}
	mode, args, err = parseSynthMode([]string{"distill"})
	if err != nil || mode != bo.SynthModeDistill || len(args) != 0 {
		t.Fatalf("distill mode = %v, %v, %v", mode, args, err)
	}
	mode, args, err = parseSynthMode([]string{"--max-turns", "2"})
	if err != nil || mode != bo.SynthModeDefault || len(args) != 2 {
		t.Fatalf("default mode = %v, %v, %v", mode, args, err)
	}
	if _, _, err := parseSynthMode([]string{"unknown"}); err == nil {
		t.Fatal("unknown mode succeeded")
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
