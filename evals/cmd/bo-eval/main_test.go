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

func TestParseWorkflowArgsSupportsDistill(t *testing.T) {
	workflow, name, tools, err := parseWorkflowArgs([]string{"distill", "notes", "--tools", "read_document,write_distillation,skip_distill"})
	if err != nil || workflow != "distill" || name != "notes" || len(tools) != 3 || tools[2] != "skip_distill" {
		t.Fatalf("parseWorkflowArgs = %q, %q, %#v, %v", workflow, name, tools, err)
	}
}
