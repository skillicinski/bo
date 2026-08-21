package application

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
)

func Synthesize(ctx context.Context, opener WorkspaceOpener, workspaceName string, provider agent.CompletionProvider, config SynthesisOptions, options OperationOptions) (SynthesisResult, error) {
	return SynthesizeWithTools(ctx, opener, workspaceName, provider, config, allSynthesisTools, options)
}

func SynthesizeWithTools(ctx context.Context, opener WorkspaceOpener, workspaceName string, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (result SynthesisResult, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return SynthesisResult{}, err
	}
	directory := workspaceName
	defer func() {
		details := map[string]any{
			"turns":             result.Metrics.Turns,
			"tool_calls":        result.Metrics.ToolCalls,
			"duration":          result.Metrics.Duration,
			"summaries_written": result.SummariesWritten,
			"summaries_skipped": result.SummariesSkipped,
		}
		if result.Metrics.Usage != nil {
			details["usage"] = result.Metrics.Usage
		}
		for key, value := range operationErrorDetails(returnErr) {
			details[key] = value
		}
		recordOperation(options, directory, CommandSynth, returnErr == nil, details)
	}()
	toolNames, err = normalizeSynthesisTools(toolNames)
	if err != nil {
		return SynthesisResult{}, InputError(err.Error())
	}
	if opener == nil {
		return SynthesisResult{}, RequestError("workspace opener is not configured")
	}
	workspace, err := opener.Open(ctx, workspaceName)
	if err != nil {
		return SynthesisResult{}, err
	}
	if workspace == nil {
		return SynthesisResult{}, RequestError("workspace opener returned no workspace")
	}
	defer workspace.Close()
	if name := workspace.Name(); name != "" {
		directory = name
	}
	result, returnErr = runSynthesis(ctx, directory, workspace.RootPath(), workspace.TargetPath(), workspace.Storage(), provider, config, toolNames, options)
	return result, returnErr
}

func runSynthesis(ctx context.Context, directory, rootPath, targetPath string, storage Storage, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (SynthesisResult, error) {
	if provider == nil {
		return SynthesisResult{}, RequestError("synthesis provider is not configured")
	}
	config = normalizedSynthesisOptions(config)
	runContext, cancel := context.WithTimeout(ctx, time.Duration(config.TimeoutSeconds)*time.Second)
	defer cancel()
	root, err := filepath.EvalSymlinks(rootPath)
	if err != nil {
		return SynthesisResult{}, fmt.Errorf("canonicalizing %s failed: %w", rootPath, err)
	}
	target, err := filepath.EvalSymlinks(targetPath)
	if err != nil {
		return SynthesisResult{}, fmt.Errorf("canonicalizing %s failed: %w", targetPath, err)
	}
	if err := ensureInside(target, root); err != nil {
		return SynthesisResult{}, err
	}
	info, err := os.Stat(target)
	if err != nil {
		return SynthesisResult{}, fmt.Errorf("reading %s failed: %w", target, err)
	}
	if !info.IsDir() {
		return SynthesisResult{}, fmt.Errorf("target is not a directory: %s", target)
	}
	documents, err := DiscoverDocuments(root, target)
	if err != nil {
		return SynthesisResult{}, err
	}
	if len(documents) == 0 {
		return SynthesisResult{}, fmt.Errorf("no raw Markdown documents in %s", target)
	}
	state, generation, err := storage.ReadState(runContext)
	if err != nil {
		return SynthesisResult{}, err
	}
	sources := sourceGroups(documents, state)
	completed := map[string]bool{}
	written := map[string]bool{}
	contextState := &agentContext{
		ctx: runContext, target: target, storage: storage, documents: documents, sources: sources,
		state: state, generation: generation, maxOutputBytes: config.MaxToolOutputBytes,
		directory: directory, actor: options.Actor, operationLog: options.Log,
		completed: completed, written: written,
	}
	names := make([]string, 0, len(sources))
	for _, source := range sources {
		names = append(names, source.LatestFilename)
	}
	sort.Strings(names)
	sourceKeys := make([]string, 0, len(sources))
	for sourceKey := range sources {
		sourceKeys = append(sourceKeys, sourceKey)
	}
	sort.Strings(sourceKeys)
	messages := []agent.ChatMessage{
		{Role: "system", Content: systemPrompt(contextState, names)},
		{Role: "user", Content: fmt.Sprintf("Produce one concise Markdown summary for every source identity. Use the newest raw snapshot as evidence and preserve each source's epistemic status. Source identities: %s", strings.Join(sourceKeys, ", "))},
	}
	runtime := agent.Runtime{
		Provider: provider,
		Tools:    synthTools(contextState, toolNames),
		Done: func() bool {
			return len(completed) == len(sources)
		},
	}
	runtimeResult, err := runtime.Run(runContext, messages, agent.Options{
		MaxTurns: config.MaxTurns, MaxToolCalls: config.MaxToolCalls,
		MaxToolOutputBytes: config.MaxToolOutputBytes, MaxResponseTokens: config.MaxResponseTokens,
	})
	result := SynthesisResult{SummariesWritten: len(written), SummariesSkipped: len(completed) - len(written), Metrics: runtimeResult.Metrics}
	if err != nil {
		return result, err
	}
	if len(completed) != len(sources) {
		return result, fmt.Errorf("model stopped with missing summaries: %s", strings.Join(missingSources(sources, completed), ", "))
	}
	return result, nil
}

func missingSources(sources map[string]agentSource, summarized map[string]bool) []string {
	missing := make([]string, 0)
	for sourceKey := range sources {
		if !summarized[sourceKey] {
			missing = append(missing, sourceKey)
		}
	}
	sort.Strings(missing)
	return missing
}

func normalizedSynthesisOptions(config SynthesisOptions) SynthesisOptions {
	defaults := DefaultSynthesisOptions()
	if config.MaxTurns <= 0 {
		config.MaxTurns = defaults.MaxTurns
	}
	if config.MaxToolCalls <= 0 {
		config.MaxToolCalls = defaults.MaxToolCalls
	}
	if config.MaxToolOutputBytes <= 0 {
		config.MaxToolOutputBytes = defaults.MaxToolOutputBytes
	}
	if config.MaxResponseTokens <= 0 {
		config.MaxResponseTokens = defaults.MaxResponseTokens
	}
	if config.TimeoutSeconds <= 0 {
		config.TimeoutSeconds = defaults.TimeoutSeconds
	}
	return config
}
