package main

import (
	"fmt"
	"strconv"

	"github.com/skillicinski/bo"
)

func synthUsage() string {
	return "usage: bo synth <name> [--max-turns N] [--max-tool-calls N] [--max-tool-output-bytes N] [--max-response-tokens N] [--timeout-seconds N]"
}

func distillUsage() string {
	return "usage: bo distill <name> [--max-turns N] [--max-tool-calls N] [--max-tool-output-bytes N] [--max-response-tokens N] [--timeout-seconds N]"
}

func parseSynthOptions(args []string) (bo.SynthesisOptions, error) {
	return parseAgentOptions(args, synthUsage())
}

func parseDistillOptions(args []string) (bo.SynthesisOptions, error) {
	return parseAgentOptions(args, distillUsage())
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
			config.TimeoutSeconds = number
		}
	}
	return config, nil
}
