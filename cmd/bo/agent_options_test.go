package main

import "testing"

func TestParseAgentOptions(t *testing.T) {
	config, err := parseAgentOptions([]string{"--max-turns", "2", "--max-tool-calls", "3", "--max-tool-output-bytes", "4", "--max-response-tokens", "5", "--timeout-seconds", "6"})
	if err != nil {
		t.Fatal(err)
	}
	if config.MaxTurns != 2 || config.MaxToolCalls != 3 || config.MaxToolOutputBytes != 4 || config.MaxResponseTokens != 5 || config.TimeoutSeconds != 6 {
		t.Fatalf("config = %#v", config)
	}
	if _, err := parseAgentOptions([]string{"--unknown", "1"}); err == nil {
		t.Fatal("unknown option succeeded")
	}
	if _, err := parseAgentOptions([]string{"--max-turns"}); err == nil {
		t.Fatal("missing value succeeded")
	}
	if _, err := parseAgentOptions([]string{"--max-turns", "zero"}); err == nil {
		t.Fatal("zero succeeded")
	}
}
