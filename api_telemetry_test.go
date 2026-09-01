package bo

import (
	"testing"

	"github.com/skillicinski/bo/internal/agent"
	app "github.com/skillicinski/bo/internal/application"
)

func TestPublicSynthResultIncludesTelemetry(t *testing.T) {
	thoughts := 7
	got := publicSynthResult(app.SynthResult{Telemetry: []app.StageTelemetry{{
		Workflow: "distill", TerminalReason: "done", TerminalDetail: "no more themes",
		ProviderRetries: 1, ProviderRetryReasons: []string{"malformed_function_call"},
		Usage: &agent.TokenUsage{ThoughtsTokens: thoughts},
		ToolCalls: []agent.ToolCallTelemetry{{
			Turn: 1, Index: 1, Name: "read_document", ArgumentsPreview: `{"filename":"one.md"}`,
		}},
	}}})
	if len(got.Telemetry) != 1 || got.Telemetry[0].Workflow != "distill" || got.Telemetry[0].TerminalReason != "done" || got.Telemetry[0].TerminalDetail != "no more themes" || got.Telemetry[0].ProviderRetries != 1 || len(got.Telemetry[0].ProviderRetryReasons) != 1 || got.Telemetry[0].ProviderRetryReasons[0] != "malformed_function_call" {
		t.Fatalf("telemetry = %#v", got.Telemetry)
	}
	if got.Telemetry[0].Usage == nil || got.Telemetry[0].Usage.ThoughtsTokens != thoughts {
		t.Fatalf("usage telemetry = %#v", got.Telemetry[0].Usage)
	}
	if len(got.Telemetry[0].ToolCalls) != 1 || got.Telemetry[0].ToolCalls[0].Name != "read_document" || got.Telemetry[0].ToolCalls[0].ArgumentsPreview != `{"filename":"one.md"}` {
		t.Fatalf("tool telemetry = %#v", got.Telemetry[0].ToolCalls)
	}
}
