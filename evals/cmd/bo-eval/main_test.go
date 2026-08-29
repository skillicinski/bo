package main

import "testing"

func TestParseArgsRequiresExplicitTools(t *testing.T) {
	name, tools, err := parseArgs([]string{"synth", "notes", "--tools", "read_logs,write_summary"})
	if err != nil || name != "notes" || len(tools) != 2 || tools[0] != "read_logs" || tools[1] != "write_summary" {
		t.Fatalf("parseArgs = %q, %#v, %v", name, tools, err)
	}
	name, tools, err = parseArgs([]string{"synth", "notes", "--tools", "all"})
	if err != nil || name != "notes" || tools != nil {
		t.Fatalf("all tools = %q, %#v, %v", name, tools, err)
	}
	if _, _, err := parseArgs([]string{"synth", "notes"}); err == nil {
		t.Fatal("missing --tools succeeded")
	}
}

func TestParseTaskArgsSupportsDistill(t *testing.T) {
	task, name, tools, err := parseTaskArgs([]string{"distill", "notes", "--tools", "read_document,write_distillation"})
	if err != nil || task != "distill" || name != "notes" || len(tools) != 2 || tools[1] != "write_distillation" {
		t.Fatalf("parseTaskArgs = %q, %q, %#v, %v", task, name, tools, err)
	}
}
