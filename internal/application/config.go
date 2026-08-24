package application

import "github.com/skillicinski/bo/internal/agent"

type SynthesisOptions struct {
	// SynthesisOptions contains bounded synthesis runtime limits. Provider credentials and
	// model selection belong to the composition root and provider adapter.
	MaxTurns           int
	MaxToolCalls       int
	MaxToolOutputBytes int
	MaxResponseTokens  int
	TimeoutSeconds     int
}

type SynthesisResult struct {
	SummariesWritten int           `json:"summaries_written"`
	SummariesSkipped int           `json:"summaries_skipped"`
	Metrics          agent.Metrics `json:"metrics"`
}

func DefaultSynthesisOptions() SynthesisOptions {
	defaults := agent.DefaultOptions()
	return SynthesisOptions{
		MaxTurns: defaults.MaxTurns, MaxToolCalls: defaults.MaxToolCalls,
		MaxToolOutputBytes: defaults.MaxToolOutputBytes, MaxResponseTokens: defaults.MaxResponseTokens,
		TimeoutSeconds: 120,
	}
}
