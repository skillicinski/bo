package main

import (
	"fmt"
	"strconv"

	"github.com/skillicinski/bo"
)

func agentUsage() string {
	return "usage: bo agent <dir> [--max-turns N] [--max-tool-calls N] [--max-tool-output-bytes N] [--max-response-tokens N] [--timeout-seconds N]"
}

func parseAgentOptions(args []string) (bo.AgentConfig, error) {
	config := bo.DefaultAgentConfig()
	for index := 0; index < len(args); index++ {
		option := args[index]
		switch option {
		case "--max-turns", "--max-tool-calls", "--max-tool-output-bytes", "--max-response-tokens", "--timeout-seconds":
		default:
			return bo.AgentConfig{}, fmt.Errorf("%s", agentUsage())
		}
		if index+1 >= len(args) {
			return bo.AgentConfig{}, fmt.Errorf("missing value for %s", option)
		}
		value := args[index+1]
		index++
		number, err := strconv.Atoi(value)
		if err != nil || number <= 0 {
			return bo.AgentConfig{}, fmt.Errorf("%s requires a positive integer", option)
		}
		switch option {
		case "--max-turns":
			config.MaxTurns = number
		case "--max-tool-calls":
			config.MaxToolCalls = number
		case "--max-tool-output-bytes":
			config.MaxToolOutputBytes = number
		case "--max-response-tokens":
			config.MaxResponseTokens = number
		case "--timeout-seconds":
			config.TimeoutSeconds = number
		}
	}
	return config, nil
}
