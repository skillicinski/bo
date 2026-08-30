package application

import "github.com/skillicinski/bo/internal/agent"

type SynthesisOptions struct {
	// SynthesisOptions contains bounded per-agent-runtime limits. Provider credentials and
	// model selection belong to the composition root and provider adapter. RuntimeTimeoutSeconds
	// limits one agent runtime; the caller context controls the complete workflow.
	MaxTurns              int
	MaxToolCalls          int
	MaxToolOutputBytes    int
	MaxResponseTokens     int
	RuntimeTimeoutSeconds int
}

type SynthesisResult struct {
	SummariesWritten int           `json:"summaries_written"`
	SummariesSkipped int           `json:"summaries_skipped"`
	Committed        []Operation   `json:"-"`
	Metrics          agent.Metrics `json:"metrics"`
}

type SynthMode string

const (
	SynthModeDefault   SynthMode = ""
	SynthModeSummarize SynthMode = "summarize"
	SynthModeDistill   SynthMode = "distill"
)

type SynthResult struct {
	SummariesWritten    int
	SummariesSkipped    int
	DistillationWritten int
	DistillationSkipped int
	Committed           []Operation
	Metrics             agent.Metrics
}

func DefaultSynthesisOptions() SynthesisOptions {
	defaults := agent.DefaultOptions()
	return SynthesisOptions{
		MaxTurns: defaults.MaxTurns, MaxToolCalls: defaults.MaxToolCalls,
		MaxToolOutputBytes: defaults.MaxToolOutputBytes, MaxResponseTokens: defaults.MaxResponseTokens,
		RuntimeTimeoutSeconds: 120,
	}
}
