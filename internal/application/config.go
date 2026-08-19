package application

import "github.com/skillicinski/bo/internal/agent"

const (
	DefaultMaxTurns                = agent.DefaultMaxTurns
	DefaultMaxToolCalls            = agent.DefaultMaxToolCalls
	DefaultMaxToolOutputBytes      = agent.DefaultMaxToolOutputBytes
	DefaultMaxResponseTokens       = agent.DefaultMaxResponseTokens
	DefaultSynthesisTimeoutSeconds = 120
)

type SynthesisOptions struct {
	// SynthesisOptions contains bounded synthesis runtime limits. Provider credentials and
	// model selection belong to the composition root and provider adapter.
	MaxTurns           int
	MaxToolCalls       int
	MaxToolOutputBytes int
	MaxResponseTokens  int
	TimeoutSeconds     int
}

func DefaultSynthesisOptions() SynthesisOptions {
	return SynthesisOptions{
		MaxTurns: DefaultMaxTurns, MaxToolCalls: DefaultMaxToolCalls,
		MaxToolOutputBytes: DefaultMaxToolOutputBytes, MaxResponseTokens: DefaultMaxResponseTokens,
		TimeoutSeconds: DefaultSynthesisTimeoutSeconds,
	}
}
