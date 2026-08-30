package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/skillicinski/bo"
)

const usage = "usage: bo seed [--name <name>] | bo snap <name> <source>... | bo state <name> [--full] | bo synth <name> [summarize|distill] [options]"

func main() {
	args := os.Args[1:]
	if len(args) == 0 {
		fmt.Fprintln(os.Stderr, usage)
		os.Exit(1)
	}
	switch args[0] {
	case "seed":
		runSeed(args[1:])
	case "snap":
		runSnap(args[1:])
	case "state":
		runState(args[1:])
	case "synth":
		runSynth(args[1:])
	default:
		fmt.Fprintln(os.Stderr, usage)
		os.Exit(1)
	}
}

func runSeed(args []string) {
	name := ""
	for index := 0; index < len(args); index++ {
		if args[index] != "--name" || name != "" {
			fail("seeding", "usage: bo seed [--name <name>]")
		}
		if index+1 >= len(args) {
			fail("seeding", "missing value for --name")
		}
		name = args[index+1]
		index++
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fail("seeding", err.Error())
	}
	result, err := bo.Seed(context.Background(), bo.SeedRequest{
		Creator:    bo.NewLocalManager(home),
		Name:       name,
		Operations: bo.OperationOptions{Actor: "cli"},
	})
	if err != nil {
		fail("seeding", err.Error())
	}
	fmt.Printf("seeded: %s\n", result.Name)
}

func runSnap(args []string) {
	if len(args) < 2 || strings.HasPrefix(args[0], "-") {
		fail("snap", "usage: bo snap <dir> <source>...")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fail("snap", err.Error())
	}
	workspace, err := bo.NewLocalManager(home).Open(context.Background(), args[0])
	if err != nil {
		err = addSeedHint(err, args[0])
		fail("snap", err.Error())
	}
	defer workspace.Close()
	result, err := bo.Snap(context.Background(), bo.SnapRequest{
		Workspace:  workspace,
		Sources:    args[1:],
		Operations: bo.OperationOptions{Actor: "cli"},
	})
	if err != nil {
		printSnapReport(result, err)
		os.Exit(1)
	}
	if printSnapReport(result, nil) {
		os.Exit(1)
	}
}

func runState(args []string) {
	if len(args) == 0 || strings.HasPrefix(args[0], "-") || len(args) > 2 || len(args) == 2 && args[1] != "--full" {
		fail("state", "usage: bo state <name> [--full]")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fail("state", err.Error())
	}
	workspace, err := bo.NewLocalManager(home).Open(context.Background(), args[0])
	if err != nil {
		fail("state", err.Error())
	}
	defer workspace.Close()
	result, err := bo.ReadState(context.Background(), bo.StateRequest{
		Workspace:  workspace,
		Operations: bo.OperationOptions{Actor: "cli"},
	})
	if err != nil {
		fail("state", err.Error())
	}
	if len(args) == 2 {
		data, err := json.MarshalIndent(result.State, "", "  ")
		if err != nil {
			fail("state", err.Error())
		}
		fmt.Println(string(data))
		return
	}
	fmt.Printf("%d documents snapped\n", result.State.SnapshotCount())
}

func runSynth(args []string) {
	if len(args) == 0 || strings.HasPrefix(args[0], "-") {
		fail("synth", synthUsage())
	}
	name := args[0]
	mode, optionArgs, err := parseSynthMode(args[1:])
	if err != nil {
		fail("synth", err.Error())
	}
	providerName, optionArgs, err := parseSynthProvider(optionArgs)
	if err != nil {
		fail("synth", err.Error())
	}
	config, err := parseSynthOptions(optionArgs)
	if err != nil {
		fail("synth", err.Error())
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fail("synth", err.Error())
	}
	workspace, err := bo.NewLocalManager(home).Open(context.Background(), name)
	if err != nil {
		fail("synth", err.Error())
	}
	defer workspace.Close()
	provider, err := synthProvider(providerName)
	if err != nil {
		fail("synth", err.Error())
	}
	result, err := bo.Synth(context.Background(), bo.SynthRequest{
		Workspace:  workspace,
		Provider:   provider,
		Mode:       mode,
		Options:    config,
		Operations: bo.OperationOptions{Actor: "cli"},
	})
	if err != nil {
		printSynthReport(result)
		fail("synth", err.Error())
	}
	printSynthReport(result)
}

func synthProvider(name string) (bo.Provider, error) {
	switch name {
	case "deepseek":
		apiKey := os.Getenv("DEEPSEEK_API_KEY")
		if apiKey == "" {
			return bo.Provider{}, fmt.Errorf("DEEPSEEK_API_KEY is not set")
		}
		return bo.NewDeepSeekProvider(bo.DeepSeekConfig{APIKey: apiKey, Endpoint: os.Getenv("DEEPSEEK_API_URL"), Model: os.Getenv("DEEPSEEK_MODEL")}), nil
	case "gemini":
		apiKey := geminiAPIKey()
		if apiKey == "" {
			return bo.Provider{}, fmt.Errorf("GEMINI_API_KEY or GOOGLE_API_KEY is not set")
		}
		return bo.NewGeminiProvider(bo.GeminiConfig{APIKey: apiKey, Endpoint: os.Getenv("GEMINI_API_URL"), Model: os.Getenv("GEMINI_MODEL")}), nil
	case "vertex":
		projectID := os.Getenv("GOOGLE_CLOUD_PROJECT")
		location := os.Getenv("GOOGLE_CLOUD_LOCATION")
		if projectID == "" || location == "" {
			return bo.Provider{}, fmt.Errorf("GOOGLE_CLOUD_PROJECT and GOOGLE_CLOUD_LOCATION are required for vertex")
		}
		return bo.NewVertexGeminiProvider(context.Background(), bo.GeminiConfig{
			ProjectID: projectID, Location: location, Endpoint: os.Getenv("GEMINI_API_URL"), Model: os.Getenv("GEMINI_MODEL"),
		})
	default:
		return bo.Provider{}, fmt.Errorf("%s", synthUsage())
	}
}

func geminiAPIKey() string {
	if key := os.Getenv("GEMINI_API_KEY"); key != "" {
		return key
	}
	return os.Getenv("GOOGLE_API_KEY")
}

func printSynthReport(result bo.SynthResult) {
	if len(result.Report) == 0 {
		fmt.Println("no committed actions")
		return
	}
	for _, operation := range result.Report {
		fmt.Printf("%s:\n", operation.Operation)
		for _, document := range operation.Documents {
			fmt.Printf("  - %s\n", document.Filename)
		}
	}
}

func addSeedHint(err error, name string) error {
	if !bo.IsKind(err, bo.ErrorKindMissingResource) {
		return err
	}
	return fmt.Errorf("%w (run bo seed --name %s)", err, name)
}

func printSnapReport(result bo.SnapResult, fatal error) bool {
	total := len(result.Outcomes)
	failed := 0
	for _, outcome := range result.Outcomes {
		if outcome.Err != nil {
			failed++
			fmt.Fprintf(os.Stderr, "failed: %s (%v)\n", outcome.SourceKey, outcome.Err)
		} else {
			fmt.Printf("snapped: %s -> %s\n", outcome.SourceKey, outcome.Filename)
		}
	}
	aborted := result.Aborted || fatal != nil
	if fatal != nil {
		if result.FailedSource != "" {
			fmt.Fprintf(os.Stderr, "failed: %s (%v)\n", result.FailedSource, fatal)
		} else {
			fmt.Fprintf(os.Stderr, "snap failed: %v\n", fatal)
		}
	}
	totalFailed := failed
	if aborted {
		totalFailed++
	}
	if aborted {
		fmt.Fprintf(os.Stderr, "%d succeeded / %d failed; batch aborted\n", total-failed, totalFailed)
	} else {
		fmt.Fprintf(os.Stderr, "%d succeeded / %d failed\n", total-failed, totalFailed)
	}
	return aborted || failed > 0
}

func fail(operation, detail string) {
	fmt.Fprintf(os.Stderr, "%s failed: %s\n", operation, detail)
	os.Exit(1)
}
