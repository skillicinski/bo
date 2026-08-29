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

type DistillRequest struct {
	Workspace  Workspace
	Provider   agent.CompletionProvider
	Options    SynthesisOptions
	ToolNames  []string
	Operations OperationOptions
}

type DistillResult struct {
	Filename  string        `json:"filename,omitempty"`
	Skipped   bool          `json:"skipped"`
	Reason    string        `json:"reason,omitempty"`
	Committed []Operation   `json:"-"`
	Filenames []string      `json:"-"`
	Metrics   agent.Metrics `json:"metrics"`
}

func Distill(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, options OperationOptions) (DistillResult, error) {
	return DistillWithTools(ctx, workspace, provider, config, allDistillTools, options)
}

func DistillWithTools(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (result DistillResult, returnErr error) {
	options = normalizeOperationOptions(options)
	var err error
	toolNames, err = normalizeDistillTools(toolNames)
	if err != nil {
		return DistillResult{}, distillFailure(ctx, workspace, options.Actor, internalerrors.Validation(err.Error()))
	}
	if workspace == nil {
		return DistillResult{}, internalerrors.Request("workspace is not configured")
	}
	result, returnErr = runDistill(ctx, workspace, provider, config, toolNames, options)
	written := distillationCount(result)
	operation := newOperation(CommandDistill, options.Actor)
	operation.Metrics = &domain.OperationMetrics{
		Turns: result.Metrics.Turns, ToolCalls: result.Metrics.ToolCalls, Duration: result.Metrics.Duration,
		DistillationWritten: written, DistillationSkipped: boolCount(result.Skipped && written == 0),
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

func boolCount(value bool) int {
	if value {
		return 1
	}
	return 0
}

func distillationCount(result DistillResult) int {
	if len(result.Filenames) > 0 {
		return len(result.Filenames)
	}
	return boolCount(result.Filename != "")
}

func distillFailure(ctx context.Context, workspace Workspace, actor string, cause error) error {
	operation := failedOperation(newOperation(CommandDistill, actor), cause)
	if workspace == nil {
		return cause
	}
	if err := commitOperationEvent(ctx, workspace, operation); err != nil {
		return errors.Join(cause, err)
	}
	return cause
}

func runDistill(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, toolNames []string, options OperationOptions) (DistillResult, error) {
	if workspace == nil {
		return DistillResult{}, internalerrors.Request("workspace is not configured")
	}
	if provider == nil {
		return DistillResult{}, internalerrors.Request("distill provider is not configured")
	}
	config = normalizedSynthesisOptions(config)
	state, revision, err := workspace.ReadState(ctx)
	if err != nil {
		return DistillResult{}, normalizeError(err, internalerrors.KindFilesystem, "reading workspace state")
	}
	catalog, err := buildDistillCatalog(ctx, workspace, state)
	if err != nil {
		return DistillResult{}, err
	}
	if len(catalog.sources) == 0 {
		return DistillResult{}, internalerrors.MissingResource("no current raw Markdown documents in workspace")
	}
	if len(catalog.sources) < 2 {
		return DistillResult{Skipped: true, Reason: "at least two source identities are required"}, nil
	}
	result := DistillResult{}
	readLogsEnabled := false
	for _, name := range toolNames {
		if name == toolReadLogs {
			readLogsEnabled = true
			break
		}
	}
	var events []Operation
	if readLogsEnabled {
		events, err = readRecentSynthesisEvents(ctx, workspace)
		if err != nil {
			return DistillResult{}, err
		}
	}
	contextState := &distillContext{
		directory: workspace.Name(), workspace: workspace, options: options,
		catalog: catalog, state: state, revision: revision, maxOutputBytes: config.MaxToolOutputBytes,
		readDocuments: map[string][]byte{}, readRefs: map[string]bool{}, readDistillations: map[string]bool{}, mutationOps: map[string]Operation{},
	}
	if readLogsEnabled {
		contextState.logEvents = scopedSynthesisEvents(events, catalog.sources)
		contextState.logWindowLoaded = true
	}
	names := make([]string, 0, len(catalog.sources))
	for _, source := range catalog.sources {
		names = append(names, source.LatestFilename)
	}
	sort.Strings(names)
	keys := make([]string, 0, len(catalog.sources))
	for sourceKey := range catalog.sources {
		keys = append(keys, sourceKey)
	}
	sort.Strings(keys)
	message := fmt.Sprintf("Process every useful cross-source theme supported by at least two distinct source identities. For a new topic, call write_distillation once; for an existing topic with changed evidence, call edit_distillation once after reading it. When no unprocessed theme remains, call skip_distill exactly once. Current source identities: %s. Latest raw documents: %s.", strings.Join(keys, ", "), strings.Join(names, ", "))
	runtime := agent.Runtime{
		Provider: provider,
		Tools:    distillTools(contextState, toolNames),
		Done: func() bool {
			return contextState.skipped
		},
	}
	runtimeContext, cancel := context.WithTimeout(ctx, time.Duration(config.RuntimeTimeoutSeconds)*time.Second)
	contextState.ctx = runtimeContext
	runtimeResult, runtimeErr := runtime.Run(runtimeContext, []agent.ChatMessage{
		{Role: "system", Content: distillSystemPrompt(contextState, readLogsEnabled)},
		{Role: "user", Content: message},
	}, agent.Options{
		MaxTurns: config.MaxTurns, MaxToolCalls: config.MaxToolCalls,
		MaxToolOutputBytes: config.MaxToolOutputBytes, MaxResponseTokens: config.MaxResponseTokens,
	})
	cancel()
	result.Metrics = runtimeResult.Metrics
	result.Committed = append(result.Committed, contextState.committed...)
	result.Filenames = append(result.Filenames, contextState.filenames...)
	if contextState.eventFailure != nil {
		if runtimeErr != nil {
			return result, errors.Join(contextState.eventFailure, runtimeErr)
		}
		return result, contextState.eventFailure
	}
	if runtimeErr != nil {
		return result, runtimeErr
	}
	if !contextState.skipped {
		return result, internalerrors.ProviderMalformed("model stopped without skip_distill", nil)
	}
	if len(result.Filenames) > 0 {
		result.Filename = result.Filenames[len(result.Filenames)-1]
	} else {
		result.Filename = contextState.filename
	}
	result.Skipped = contextState.skipped && len(result.Filenames) == 0
	result.Reason = contextState.reason
	return result, nil
}
