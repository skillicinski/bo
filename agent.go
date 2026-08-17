package bo

const (
	DefaultMaxTurns            = 32
	DefaultMaxToolCalls        = 64
	DefaultMaxToolOutputBytes  = 65_536
	DefaultMaxResponseTokens   = 4_096
	DefaultAgentTimeoutSeconds = 120
)

type AgentConfig struct {
	// AgentConfig contains bounded agent runtime limits. Provider credentials and
	// model selection belong to the composition root and provider adapter.
	MaxTurns           int
	MaxToolCalls       int
	MaxToolOutputBytes int
	MaxResponseTokens  int
	TimeoutSeconds     int
}

func DefaultAgentConfig() AgentConfig {
	return AgentConfig{
		MaxTurns: DefaultMaxTurns, MaxToolCalls: DefaultMaxToolCalls,
		MaxToolOutputBytes: DefaultMaxToolOutputBytes, MaxResponseTokens: DefaultMaxResponseTokens,
		TimeoutSeconds: DefaultAgentTimeoutSeconds,
	}
}
