package application

import (
	"context"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func Synthesize(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, options OperationOptions) (SynthesisResult, error) {
	return SynthesizeWithTools(ctx, workspace, provider, config, allSynthesisTools, options)
}

func SynthesizeWithTools(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (result SynthesisResult, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return SynthesisResult{}, err
	}
	directory := ""
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
		return SynthesisResult{}, internalerrors.Validation(err.Error())
	}
	if workspace == nil {
		return SynthesisResult{}, internalerrors.Request("workspace is not configured")
	}
	directory = workspace.Name()
	result, returnErr = runSynthesis(ctx, directory, workspace, provider, config, toolNames, options)
	return result, returnErr
}

func runSynthesis(ctx context.Context, directory string, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (SynthesisResult, error) {
	if workspace == nil {
		return SynthesisResult{}, internalerrors.Request("workspace is not configured")
	}
	if provider == nil {
		return SynthesisResult{}, internalerrors.Request("synthesis provider is not configured")
	}
	config = normalizedSynthesisOptions(config)
	runContext, cancel := context.WithTimeout(ctx, time.Duration(config.TimeoutSeconds)*time.Second)
	defer cancel()
	documents, err := DiscoverDocuments(runContext, workspace)
	if err != nil {
		return SynthesisResult{}, err
	}
	if len(documents) == 0 {
		return SynthesisResult{}, internalerrors.MissingResource("no raw Markdown documents in workspace")
	}
	state, revision, err := workspace.ReadState(runContext)
	if err != nil {
		return SynthesisResult{}, normalizeError(err, internalerrors.KindFilesystem, "reading workspace state")
	}
	sources := sourceGroups(documents, state)
	completed := map[string]bool{}
	written := map[string]bool{}
	contextState := &agentContext{
		ctx: runContext, workspace: workspace, documents: documents, sources: sources,
		state: state, revision: revision, maxOutputBytes: config.MaxToolOutputBytes,
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
		return result, internalerrors.ProviderMalformed(fmt.Sprintf("model stopped with missing summaries: %s", strings.Join(missingSources(sources, completed), ", ")), nil)
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
