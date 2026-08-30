package main

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/skillicinski/bo"
)

func synthUsage() string {
	return "usage: bo synth <name> [summarize|distill] [--provider deepseek|gemini|vertex] [--max-turns N] [--max-tool-calls N] [--max-tool-output-bytes N] [--max-response-tokens N] [--timeout-seconds N]"
}

func parseSynthOptions(args []string) (bo.SynthesisOptions, error) {
	return parseAgentOptions(args, synthUsage())
}

func parseSynthMode(args []string) (bo.SynthMode, []string, error) {
	if len(args) == 0 || strings.HasPrefix(args[0], "-") {
		return bo.SynthModeDefault, args, nil
	}
	switch args[0] {
	case "summarize":
		return bo.SynthModeSummarize, args[1:], nil
	case "distill":
		return bo.SynthModeDistill, args[1:], nil
	default:
		return bo.SynthModeDefault, nil, fmt.Errorf("%s", synthUsage())
	}
}

func parseSynthProvider(args []string) (string, []string, error) {
	provider := "deepseek"
	remaining := make([]string, 0, len(args))
	for index := 0; index < len(args); index++ {
		if args[index] != "--provider" {
			remaining = append(remaining, args[index])
			continue
		}
		if index+1 >= len(args) {
			return "", nil, fmt.Errorf("missing value for --provider")
		}
		provider = args[index+1]
		index++
		switch provider {
		case "deepseek", "gemini", "vertex":
		default:
			return "", nil, fmt.Errorf("%s", synthUsage())
		}
	}
	return provider, remaining, nil
}

func parseAgentOptions(args []string, usage string) (bo.SynthesisOptions, error) {
	config := bo.DefaultSynthesisOptions()
	for index := 0; index < len(args); index++ {
		option := args[index]
		switch option {
		case "--max-turns", "--max-tool-calls", "--max-tool-output-bytes", "--max-response-tokens", "--timeout-seconds":
		default:
			return bo.SynthesisOptions{}, fmt.Errorf("%s", usage)
		}
		if index+1 >= len(args) {
			return bo.SynthesisOptions{}, fmt.Errorf("missing value for %s", option)
		}
		value := args[index+1]
		index++
		number, err := strconv.Atoi(value)
		if err != nil || number <= 0 {
			return bo.SynthesisOptions{}, fmt.Errorf("%s requires a positive integer", option)
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
			config.RuntimeTimeoutSeconds = number
		}
	}
	return config, nil
}
