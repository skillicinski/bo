package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/skillicinski/bo"
)

const usage = "usage: bo seed [--name <name>] | bo snap <name> <source>... | bo state <name> [--full] | bo synth <name> [options] | bo distill <name> [options]"

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
	case "distill":
		runDistill(args[1:])
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
	config, err := parseSynthOptions(args[1:])
	if err != nil {
		fail("synth", err.Error())
	}
	apiKey := os.Getenv("DEEPSEEK_API_KEY")
	if apiKey == "" {
		fail("synth", "DEEPSEEK_API_KEY is not set")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fail("synth", err.Error())
	}
	endpoint := os.Getenv("DEEPSEEK_API_URL")
	workspace, err := bo.NewLocalManager(home).Open(context.Background(), name)
	if err != nil {
		fail("synth", err.Error())
	}
	defer workspace.Close()
	provider := bo.NewDeepSeekProvider(bo.DeepSeekConfig{APIKey: apiKey, Endpoint: endpoint})
	result, err := bo.Synth(context.Background(), bo.SynthRequest{
		Workspace:  workspace,
		Provider:   provider,
		Options:    config,
		Operations: bo.OperationOptions{Actor: "cli"},
	})
	if err != nil {
		fail("synth", err.Error())
	}
	fmt.Printf("%d summaries written\n", result.SummariesWritten)
}

func runDistill(args []string) {
	if len(args) == 0 || strings.HasPrefix(args[0], "-") {
		fail("distill", distillUsage())
	}
	name := args[0]
	config, err := parseDistillOptions(args[1:])
	if err != nil {
		fail("distill", err.Error())
	}
	apiKey := os.Getenv("DEEPSEEK_API_KEY")
	if apiKey == "" {
		fail("distill", "DEEPSEEK_API_KEY is not set")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		fail("distill", err.Error())
	}
	endpoint := os.Getenv("DEEPSEEK_API_URL")
	workspace, err := bo.NewLocalManager(home).Open(context.Background(), name)
	if err != nil {
		fail("distill", err.Error())
	}
	defer workspace.Close()
	provider := bo.NewDeepSeekProvider(bo.DeepSeekConfig{APIKey: apiKey, Endpoint: endpoint})
	result, err := bo.Distill(context.Background(), bo.DistillRequest{
		Workspace:  workspace,
		Provider:   provider,
		Options:    config,
		Operations: bo.OperationOptions{Actor: "cli"},
	})
	if err != nil {
		fail("distill", err.Error())
	}
	if result.Skipped {
		fmt.Printf("distill skipped: %s\n", result.Reason)
		return
	}
	fmt.Printf("distilled: %s\n", result.Filename)
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
