package application

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func Synthesize(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, options OperationOptions) (SynthesisResult, error) {
	return SynthesizeWithTools(ctx, workspace, provider, config, allSynthesisTools, options)
}

func SynthesizeWithTools(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (result SynthesisResult, returnErr error) {
	options = normalizeOperationOptions(options)
	var err error
	toolNames, err = normalizeSynthesisTools(toolNames)
	if err != nil {
		return SynthesisResult{}, synthFailure(ctx, workspace, options.Actor, internalerrors.Validation(err.Error()))
	}
	if workspace == nil {
		return SynthesisResult{}, internalerrors.Request("workspace is not configured")
	}
	result, returnErr = runSynthesis(ctx, workspace, provider, config, toolNames, options)
	operation := newOperation(CommandSynth, options.Actor)
	operation.Metrics = &domain.OperationMetrics{
		Turns: result.Metrics.Turns, ToolCalls: result.Metrics.ToolCalls, Duration: result.Metrics.Duration,
		SummariesWritten: result.SummariesWritten, SummariesSkipped: result.SummariesSkipped,
	}
	if result.Metrics.Usage != nil {
		operation.Metrics.Usage = &domain.TokenUsage{
			PromptTokens: result.Metrics.Usage.PromptTokens, CompletionTokens: result.Metrics.Usage.CompletionTokens, TotalTokens: result.Metrics.Usage.TotalTokens,
		}
	}
	if returnErr == nil {
		operation = committedOperation(operation)
	} else {
		operation = failedOperation(operation, returnErr)
	}
	if eventErr := commitOperationEvent(ctx, workspace, operation); eventErr != nil {
		returnErr = errors.Join(returnErr, eventErr)
	}
	return result, returnErr
}

func synthFailure(ctx context.Context, workspace Workspace, actor string, cause error) error {
	operation := failedOperation(newOperation(CommandSynth, actor), cause)
	if workspace != nil {
		if err := commitOperationEvent(ctx, workspace, operation); err != nil {
			return errors.Join(cause, err)
		}
		return cause
	}
	return cause
}

func runSynthesis(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (SynthesisResult, error) {
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
	written := map[string]bool{}
	skipped := map[string]bool{}
	result := SynthesisResult{}
	batchLimit := synthesisBatchLimit(config)
	usageKnown, usageReceived := true, false
	var usage agent.TokenUsage
	var events []Operation
	readLogsEnabled := false
	for _, name := range toolNames {
		if name == toolReadLogs {
			readLogsEnabled = true
			break
		}
	}
	eventsLoaded := !readLogsEnabled
	state, revision, err := workspace.ReadState(runContext)
	if err != nil {
		return result, normalizeError(err, internalerrors.KindFilesystem, "reading workspace state")
	}
	for {
		sources := sourceGroups(documents, state)
		completed := completedSynthesisSources(state, sources)
		for sourceKey := range skipped {
			if !completed[sourceKey] {
				delete(skipped, sourceKey)
			}
		}
		for sourceKey := range completed {
			if !written[sourceKey] {
				skipped[sourceKey] = true
			}
		}
		pending := missingSources(sources, completed)
		if len(pending) == 0 {
			break
		}
		if readLogsEnabled && !eventsLoaded {
			events, err = readRecentSynthesisEvents(runContext, workspace)
			if err != nil {
				return result, err
			}
			eventsLoaded = true
		}
		batchCount := batchLimit
		if batchCount > len(pending) {
			batchCount = len(pending)
		}
		batchKeys := pending[:batchCount]
		batchSources := make(map[string]agentSource, batchCount)
		for _, sourceKey := range batchKeys {
			batchSources[sourceKey] = sources[sourceKey]
		}
		contextState := &agentContext{
			ctx: runContext, workspace: workspace, documents: documents, sources: batchSources,
			state: state, revision: revision, maxOutputBytes: config.MaxToolOutputBytes,
			directory: workspace.Name(), options: options,
			completed: map[string]bool{}, written: map[string]bool{}, mutationOps: map[string]Operation{},
		}
		if readLogsEnabled {
			contextState.logEvents = scopedSynthesisEvents(events, batchSources)
			contextState.logWindowLoaded = true
		}
		names := make([]string, 0, len(batchSources))
		for _, source := range batchSources {
			names = append(names, source.LatestFilename)
		}
		sort.Strings(names)
		sourceKeys := append([]string{}, batchKeys...)
		messages := []agent.ChatMessage{
			{Role: "system", Content: systemPrompt(contextState, names, readLogsEnabled)},
			{Role: "user", Content: fmt.Sprintf("Produce one concise Markdown summary for every source identity. Use the newest raw snapshot as evidence and preserve each source's epistemic status. Source identities: %s", strings.Join(sourceKeys, ", "))},
		}
		runtime := agent.Runtime{
			Provider: provider,
			Tools:    synthTools(contextState, toolNames),
			Done: func() bool {
				return len(contextState.completed) == len(batchSources)
			},
		}
		runtimeResult, runtimeErr := runtime.Run(runContext, messages, agent.Options{
			MaxTurns: config.MaxTurns, MaxToolCalls: config.MaxToolCalls,
			MaxToolOutputBytes: config.MaxToolOutputBytes, MaxResponseTokens: config.MaxResponseTokens,
		})
		result.Metrics.Turns += runtimeResult.Metrics.Turns
		result.Metrics.ToolCalls += runtimeResult.Metrics.ToolCalls
		result.Metrics.Duration += runtimeResult.Metrics.Duration
		if runtimeResult.Metrics.Usage == nil {
			usageKnown = false
		} else if usageKnown {
			usage.PromptTokens += runtimeResult.Metrics.Usage.PromptTokens
			usage.CompletionTokens += runtimeResult.Metrics.Usage.CompletionTokens
			usage.TotalTokens += runtimeResult.Metrics.Usage.TotalTokens
			usageReceived = true
		}
		for sourceKey := range contextState.written {
			written[sourceKey] = true
			delete(skipped, sourceKey)
		}
		for sourceKey := range contextState.completed {
			if !contextState.written[sourceKey] && !written[sourceKey] {
				skipped[sourceKey] = true
			}
		}
		result.Metrics.Usage = aggregateUsage(usage, usageKnown, usageReceived)
		result.SummariesWritten, result.SummariesSkipped = len(written), len(skipped)
		if contextState.eventFailure != nil {
			if runtimeErr != nil {
				return result, errors.Join(contextState.eventFailure, runtimeErr)
			}
			return result, contextState.eventFailure
		}
		if runtimeErr != nil {
			return result, runtimeErr
		}
		if len(contextState.completed) != len(batchSources) {
			return result, internalerrors.ProviderMalformed(fmt.Sprintf("model stopped with missing summaries: %s", strings.Join(missingSources(batchSources, contextState.completed), ", ")), nil)
		}
		if batchCount == len(pending) {
			break
		}
		state, revision, err = workspace.ReadState(runContext)
		if err != nil {
			result.Metrics.Usage = aggregateUsage(usage, usageKnown, usageReceived)
			result.SummariesWritten, result.SummariesSkipped = len(written), len(skipped)
			return result, normalizeError(err, internalerrors.KindFilesystem, "reading workspace state")
		}
	}
	result.Metrics.Usage = aggregateUsage(usage, usageKnown, usageReceived)
	result.SummariesWritten, result.SummariesSkipped = len(written), len(skipped)
	return result, nil
}

func synthesisBatchLimit(config SynthesisOptions) int {
	// ponytail: reserve one setup turn/call and two calls per source; add per-tool cost accounting if setup grows.
	limit := config.MaxToolCalls - 1
	if config.MaxTurns-1 < limit {
		limit = config.MaxTurns - 1
	}
	limit /= 2
	if limit < 1 {
		return 1
	}
	return limit
}

func aggregateUsage(usage agent.TokenUsage, known, received bool) *agent.TokenUsage {
	if !known || !received {
		return nil
	}
	return &agent.TokenUsage{PromptTokens: usage.PromptTokens, CompletionTokens: usage.CompletionTokens, TotalTokens: usage.TotalTokens}
}

const synthesisEventWindow = MaxOperationPageLimit

func readRecentSynthesisEvents(ctx context.Context, workspace Workspace) ([]Operation, error) {
	events, err := workspace.ReadRecentEvents(ctx, synthesisEventWindow)
	if err != nil {
		return nil, normalizeError(err, internalerrors.KindFilesystem, "reading recent operation log")
	}
	if len(events) > synthesisEventWindow {
		return nil, internalerrors.Validation("recent operation log exceeds its fixed window")
	}
	for index, event := range events {
		if err := event.Validate(); err != nil {
			return nil, internalerrors.Wrap(internalerrors.KindValidation, fmt.Sprintf("recent operation log entry %d is invalid", index), err)
		}
	}
	return events, nil
}

func completedSynthesisSources(state domain.State, sources map[string]agentSource) map[string]bool {
	completed := make(map[string]bool, len(sources))
	for sourceKey, source := range sources {
		summary := summaryRecord(state, sourceKey)
		if summary != nil && summary.DerivedFrom == source.LatestFilename {
			completed[sourceKey] = true
		}
	}
	return completed
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
