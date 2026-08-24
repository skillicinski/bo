package main

import (
	"context"
	"fmt"
	"os"
	"strings"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/provider/deepseek"
	"github.com/skillicinski/bo/internal/storage/local"
)

const usage = "usage: bo-eval synth <name> --tools all|name,name,..."

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "bo-eval failed: %s\n", err)
		os.Exit(1)
	}
}

func run(args []string) error {
	name, toolNames, err := parseArgs(args)
	if err != nil {
		return err
	}
	apiKey := os.Getenv("DEEPSEEK_API_KEY")
	if apiKey == "" {
		return fmt.Errorf("DEEPSEEK_API_KEY is not set")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	workspace, err := local.NewManager(home).Open(context.Background(), name)
	if err != nil {
		return err
	}
	defer workspace.Close()
	result, err := application.SynthesizeWithTools(
		context.Background(),
		workspace,
		deepseek.New(apiKey, os.Getenv("DEEPSEEK_API_URL")),
		application.DefaultSynthesisOptions(),
		toolNames,
		application.OperationOptions{Actor: "eval"},
	)
	if err != nil {
		return err
	}
	fmt.Printf("%d summaries written\n", result.SummariesWritten)
	return nil
}

func parseArgs(args []string) (string, []string, error) {
	if len(args) < 3 || args[0] != "synth" || args[1] == "" {
		return "", nil, fmt.Errorf("%s", usage)
	}
	var toolset string
	for index := 2; index < len(args); index++ {
		if args[index] != "--tools" || toolset != "" || index+1 >= len(args) {
			return "", nil, fmt.Errorf("%s", usage)
		}
		toolset = args[index+1]
		index++
	}
	if toolset == "" {
		return "", nil, fmt.Errorf("%s", usage)
	}
	if toolset == "all" {
		return args[1], nil, nil
	}
	return args[1], strings.Split(toolset, ","), nil
}
