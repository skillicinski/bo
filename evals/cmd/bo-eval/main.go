package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/skillicinski/bo"
)

const usage = "usage: bo-eval capture --name NAME --corpus FILE | run --name NAME --workflow summarize|distill|end-to-end --provider deepseek|gemini"

type captureOptions struct {
	name   string
	corpus string
}

type runOptions struct {
	name     string
	workflow string
	provider string
}

type report struct {
	Command  string      `json:"command"`
	Workflow string      `json:"workflow,omitempty"`
	Provider string      `json:"provider,omitempty"`
	Result   interface{} `json:"result,omitempty"`
	Error    *errorInfo  `json:"error,omitempty"`
}

type errorInfo struct {
	Kind      string `json:"kind,omitempty"`
	Detail    string `json:"detail,omitempty"`
	Retryable bool   `json:"retryable,omitempty"`
	Message   string `json:"message"`
}

type snapReport struct {
	Name     string        `json:"name"`
	Outcomes []snapOutcome `json:"outcomes"`
}

type snapOutcome struct {
	SourceKey string `json:"source_key"`
	Filename  string `json:"filename,omitempty"`
	Error     string `json:"error,omitempty"`
}

func main() {
	if err := execute(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "bo-eval failed: %s\n", err)
		os.Exit(1)
	}
}

func execute(args []string) error {
	if len(args) == 0 {
		return errors.New(usage)
	}
	switch args[0] {
	case "capture":
		options, err := parseCaptureArgs(args[1:])
		if err != nil {
			return err
		}
		return capture(options)
	case "run":
		options, err := parseRunArgs(args[1:])
		if err != nil {
			return err
		}
		return run(options)
	default:
		return errors.New(usage)
	}
}

func parseCaptureArgs(args []string) (captureOptions, error) {
	options := captureOptions{}
	for index := 0; index < len(args); index++ {
		if index+1 >= len(args) {
			return captureOptions{}, errors.New(usage)
		}
		switch args[index] {
		case "--name":
			if options.name != "" {
				return captureOptions{}, errors.New(usage)
			}
			options.name = args[index+1]
		case "--corpus":
			if options.corpus != "" {
				return captureOptions{}, errors.New(usage)
			}
			options.corpus = args[index+1]
		default:
			return captureOptions{}, errors.New(usage)
		}
		index++
	}
	if options.name == "" || options.corpus == "" {
		return captureOptions{}, errors.New(usage)
	}
	return options, nil
}

func parseRunArgs(args []string) (runOptions, error) {
	options := runOptions{provider: "deepseek"}
	for index := 0; index < len(args); index++ {
		if index+1 >= len(args) {
			return runOptions{}, errors.New(usage)
		}
		switch args[index] {
		case "--name":
			if options.name != "" {
				return runOptions{}, errors.New(usage)
			}
			options.name = args[index+1]
		case "--workflow":
			if options.workflow != "" {
				return runOptions{}, errors.New(usage)
			}
			options.workflow = args[index+1]
		case "--provider":
			options.provider = args[index+1]
		default:
			return runOptions{}, errors.New(usage)
		}
		index++
	}
	if options.name == "" || !validWorkflow(options.workflow) || (options.provider != "deepseek" && options.provider != "gemini") {
		return runOptions{}, errors.New(usage)
	}
	return options, nil
}

func validWorkflow(workflow string) bool {
	return workflow == "summarize" || workflow == "distill" || workflow == "end-to-end"
}

func capture(options captureOptions) error {
	sources, err := readCorpus(options.corpus)
	if err != nil {
		return err
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	ctx, cancel := evaluationContext()
	defer cancel()
	manager := bo.NewLocalManager(home)
	seeded, err := bo.Seed(ctx, bo.SeedRequest{
		Creator:    manager,
		Name:       options.name,
		Operations: bo.OperationOptions{Actor: "eval-harness"},
	})
	if err != nil {
		return emitError("capture", "", "", err)
	}
	workspace, err := manager.Open(ctx, seeded.Name)
	if err != nil {
		return emitError("capture", "", "", err)
	}
	defer workspace.Close()
	result, snapErr := bo.Snap(ctx, bo.SnapRequest{
		Workspace:  workspace,
		Sources:    sources,
		Operations: bo.OperationOptions{Actor: "eval-harness"},
	})
	output := snapReport{Name: seeded.Name, Outcomes: make([]snapOutcome, 0, len(result.Outcomes))}
	failed := 0
	for _, outcome := range result.Outcomes {
		item := snapOutcome{SourceKey: outcome.SourceKey, Filename: outcome.Filename}
		if outcome.Err != nil {
			item.Error = outcome.Err.Error()
			failed++
		}
		output.Outcomes = append(output.Outcomes, item)
	}
	if err := emit(report{Command: "capture", Result: output}); err != nil {
		return err
	}
	if snapErr != nil {
		return snapErr
	}
	if failed != 0 || len(output.Outcomes) != len(sources) {
		return fmt.Errorf("capture completed with %d successful sources out of %d", len(output.Outcomes)-failed, len(sources))
	}
	return nil
}

func run(options runOptions) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	ctx, cancel := evaluationContext()
	defer cancel()
	workspace, err := bo.NewLocalManager(home).Open(ctx, options.name)
	if err != nil {
		return emitError("run", options.workflow, options.provider, err)
	}
	defer workspace.Close()
	provider, err := newProvider(options.provider)
	if err != nil {
		return emitError("run", options.workflow, options.provider, err)
	}
	mode := bo.SynthModeDefault
	switch options.workflow {
	case "summarize":
		mode = bo.SynthModeSummarize
	case "distill":
		mode = bo.SynthModeDistill
	}
	result, synthErr := bo.Synth(ctx, bo.SynthRequest{
		Workspace:  workspace,
		Provider:   provider,
		Mode:       mode,
		Options:    bo.DefaultSynthesisOptions(),
		Operations: bo.OperationOptions{Actor: "eval-harness"},
	})
	output := report{Command: "run", Workflow: options.workflow, Provider: options.provider, Result: result}
	if synthErr != nil {
		output.Error = errorInfoFor(synthErr)
	}
	if err := emit(output); err != nil {
		return err
	}
	return synthErr
}

func newProvider(name string) (bo.Provider, error) {
	switch name {
	case "deepseek":
		key := os.Getenv("DEEPSEEK_API_KEY")
		if key == "" {
			return bo.Provider{}, errors.New("DEEPSEEK_API_KEY is not set")
		}
		return bo.NewDeepSeekProvider(bo.DeepSeekConfig{
			APIKey: key, Endpoint: os.Getenv("DEEPSEEK_API_URL"), Model: os.Getenv("DEEPSEEK_MODEL"),
		}), nil
	case "gemini":
		key := os.Getenv("GEMINI_API_KEY")
		if key == "" {
			key = os.Getenv("GOOGLE_API_KEY")
		}
		if key == "" {
			return bo.Provider{}, errors.New("GEMINI_API_KEY or GOOGLE_API_KEY is not set")
		}
		thinkingBudget, err := geminiThinkingBudget()
		if err != nil {
			return bo.Provider{}, err
		}
		return bo.NewGeminiProvider(bo.GeminiConfig{
			APIKey: key, Endpoint: os.Getenv("GEMINI_API_URL"), Model: os.Getenv("GEMINI_MODEL"), ThinkingBudget: thinkingBudget,
		}), nil
	default:
		return bo.Provider{}, errors.New(usage)
	}
}

func geminiThinkingBudget() (*int, error) {
	value := os.Getenv("GEMINI_THINKING_BUDGET")
	if value == "" {
		return nil, nil
	}
	budget, err := strconv.Atoi(value)
	if err != nil || budget < -1 {
		return nil, errors.New("GEMINI_THINKING_BUDGET must be -1 or a non-negative integer")
	}
	return &budget, nil
}

func readCorpus(path string) ([]string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read corpus: %w", err)
	}
	var sources []string
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		sources = append(sources, line)
	}
	if len(sources) == 0 {
		return nil, errors.New("corpus has no sources")
	}
	return sources, nil
}

func evaluationContext() (context.Context, context.CancelFunc) {
	seconds := 900
	if value, err := strconv.Atoi(os.Getenv("BO_EVAL_TIMEOUT_SECONDS")); err == nil && value > 0 {
		seconds = value
	}
	return context.WithTimeout(context.Background(), time.Duration(seconds)*time.Second)
}

func emit(value interface{}) error {
	return json.NewEncoder(os.Stdout).Encode(value)
}

func emitError(command, workflow, provider string, err error) error {
	if emit(report{Command: command, Workflow: workflow, Provider: provider, Error: errorInfoFor(err)}) != nil {
		return err
	}
	return err
}

func errorInfoFor(err error) *errorInfo {
	if err == nil {
		return nil
	}
	info := &errorInfo{Message: err.Error()}
	var publicErr *bo.Error
	if errors.As(err, &publicErr) {
		info.Kind = string(publicErr.Kind)
		info.Detail = publicErr.Detail
		info.Retryable = publicErr.Retryable
	}
	return info
}
