package main

import (
	"context"
	"fmt"
	"os"
	"strings"

	"github.com/skillicinski/bo"
	"github.com/skillicinski/bo/internal/provider/deepseek"
	urlsource "github.com/skillicinski/bo/internal/source/url"
	"github.com/skillicinski/bo/internal/storage/local"
)

const usage = "usage: bo seed [--name <name>] | bo snap <name> <url>... | bo state <name> [--full] | bo synth <name> [options]"

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
	home, err := local.HomeDir()
	if err != nil {
		fail("seeding", err.Error())
	}
	created, err := bo.Seed(context.Background(), local.NewManager(home), name)
	if err != nil {
		fail("seeding", err.Error())
	}
	fmt.Printf("seeded: %s\n", created)
}

func runSnap(args []string) {
	if len(args) < 2 || strings.HasPrefix(args[0], "-") {
		fail("snap", "usage: bo snap <dir> <url>...")
	}
	home, err := local.HomeDir()
	if err != nil {
		fail("snap", err.Error())
	}
	target, err := local.ResolveTarget(home, args[0])
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
	outcomes, commandErr := bo.Snap(context.Background(), storage, urlsource.NewHTTP(), args[1:])
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
	home, err := local.HomeDir()
	if err != nil {
		fail("state", err.Error())
	}
	target, err := local.ResolveTarget(home, args[0])
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
	home, err := local.HomeDir()
	if err != nil {
		fail("synth", err.Error())
	}
	endpoint := os.Getenv("DEEPSEEK_API_URL")
	provider := deepseek.New(apiKey, endpoint)
	written, err := bo.Synthesize(context.Background(), local.NewManager(home), name, provider, config)
	if err != nil {
		fail("synth", err.Error())
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
