package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/skillicinski/bo"
	"github.com/skillicinski/bo/deepseek"
	"github.com/skillicinski/bo/local"
	"github.com/skillicinski/bo/source"
)

const usage = "usage: bo seed [--name <name>] | bo snap <dir> <url>... | bo state <name> [--full] | bo agent <dir> [options]"

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
	case "agent":
		runAgent(args[1:])
	default:
		fmt.Fprintln(os.Stderr, usage)
		os.Exit(1)
	}
}

func runSeed(args []string) {
	var name *string
	for index := 0; index < len(args); index++ {
		if args[index] != "--name" || name != nil {
			fail("seeding", "usage: bo seed [--name <name>]")
		}
		if index+1 >= len(args) {
			fail("seeding", "missing value for --name")
		}
		value := args[index+1]
		name = &value
		index++
	}
	home, err := bo.HomeDir()
	if err != nil {
		fail("seeding", err.Error())
	}
	path, err := bo.Seed(home, name)
	if err != nil {
		fail("seeding", err.Error())
	}
	fmt.Printf("seeded at %s\n", path)
}

func runSnap(args []string) {
	if len(args) < 2 || strings.HasPrefix(args[0], "-") {
		fail("snap", "usage: bo snap <dir> <url>...")
	}
	home, err := bo.HomeDir()
	if err != nil {
		fail("snap", err.Error())
	}
	target, err := bo.ResolveTarget(home, args[0])
	if err != nil {
		if strings.HasPrefix(err.Error(), "target directory does not exist:") {
			err = fmt.Errorf("%s (run bo seed --name %s)", err, args[0])
		}
		fail("snap", err.Error())
	}
	storage, err := local.Open(target)
	if err != nil {
		fail("snap", err.Error())
	}
	defer storage.Close()
	outcomes, commandErr := bo.Snap(context.Background(), storage, source.NewHTTP(), args[1:])
	if commandErr != nil {
		fatal, ok := commandErr.(*bo.SnapCommandError)
		if !ok {
			fmt.Fprintf(os.Stderr, "snap failed: %v\n", commandErr)
			os.Exit(1)
		}
		if len(fatal.Completed) > 0 || fatal.SourceURL != "" {
			if len(fatal.Completed) > 0 {
				outcomes = fatal.Completed
			}
			printSnapReport(outcomes, fatal)
		} else {
			fmt.Fprintf(os.Stderr, "snap failed: %v\n", fatal)
		}
		os.Exit(1)
	}
	if printSnapReport(outcomes, nil) {
		os.Exit(1)
	}
}

func runState(args []string) {
	if len(args) == 0 || strings.HasPrefix(args[0], "-") || len(args) > 2 || len(args) == 2 && args[1] != "--full" {
		fail("state", "usage: bo state <name> [--full]")
	}
	home, err := bo.HomeDir()
	if err != nil {
		fail("state", err.Error())
	}
	target, err := bo.ResolveTarget(home, args[0])
	if err != nil {
		fail("state", err.Error())
	}
	storage, err := local.Open(target)
	if err != nil {
		fail("state", err.Error())
	}
	defer storage.Close()
	output, err := bo.StateOutput(context.Background(), storage, len(args) == 2)
	if err != nil {
		fail("state", err.Error())
	}
	fmt.Println(output)
}

func runAgent(args []string) {
	if len(args) == 0 || strings.HasPrefix(args[0], "-") {
		fail("agent", bo.AgentUsage())
	}
	name := args[0]
	config, err := bo.ParseAgentOptions(args[1:])
	if err != nil {
		fail("agent", err.Error())
	}
	apiKey := os.Getenv("DEEPSEEK_API_KEY")
	if apiKey == "" {
		fail("agent", "DEEPSEEK_API_KEY is not set")
	}
	home, err := bo.HomeDir()
	if err != nil {
		fail("agent", err.Error())
	}
	target, err := bo.ResolveTarget(home, name)
	if err != nil {
		fail("agent", err.Error())
	}
	root, err := filepath.EvalSymlinks(filepath.Dir(target))
	if err != nil {
		fail("agent", err.Error())
	}
	storage, err := local.Open(target)
	if err != nil {
		fail("agent", err.Error())
	}
	defer storage.Close()
	endpoint := os.Getenv("DEEPSEEK_API_URL")
	provider := deepseek.New(apiKey, endpoint)
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(config.TimeoutSeconds)*time.Second)
	defer cancel()
	written, err := bo.RunAgent(ctx, root, target, storage, provider, config)
	if err != nil {
		fail("agent", err.Error())
	}
	fmt.Printf("%d summaries written\n", written)
}

func printSnapReport(outcomes []bo.SnapOutcome, fatal *bo.SnapCommandError) bool {
	total := len(outcomes)
	failed := 0
	for _, outcome := range outcomes {
		if outcome.Err != nil {
			failed++
			fmt.Fprintf(os.Stderr, "failed: %s (%v)\n", outcome.SourceURL, outcome.Err)
		} else {
			fmt.Printf("snapped: %s -> %s\n", outcome.SourceURL, outcome.Filename)
		}
	}
	aborted := fatal != nil
	if fatal != nil {
		if fatal.SourceURL != "" {
			fmt.Fprintf(os.Stderr, "failed: %s (%v)\n", fatal.SourceURL, fatal.Err)
		} else {
			fmt.Fprintf(os.Stderr, "snap failed: %v\n", fatal.Err)
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
