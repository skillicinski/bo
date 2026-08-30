package application

import (
	"context"
	"errors"
	"fmt"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func Synth(ctx context.Context, workspace Workspace, provider agent.CompletionProvider, config SynthesisOptions, mode SynthMode, options OperationOptions) (result SynthResult, returnErr error) {
	options = normalizeOperationOptions(options)
	if workspace == nil {
		return result, internalerrors.Request("workspace is not configured")
	}

	usageKnown, usageReceived := true, false
	mergeMetrics := func(metrics agent.Metrics) {
		result.Metrics.Turns += metrics.Turns
		result.Metrics.ToolCalls += metrics.ToolCalls
		result.Metrics.Duration += metrics.Duration
		if metrics.Turns == 0 {
			return
		}
		if metrics.Usage == nil {
			usageKnown = false
			return
		}
		if usageKnown {
			if result.Metrics.Usage == nil {
				result.Metrics.Usage = &agent.TokenUsage{}
			}
			result.Metrics.Usage.PromptTokens += metrics.Usage.PromptTokens
			result.Metrics.Usage.CompletionTokens += metrics.Usage.CompletionTokens
			result.Metrics.Usage.TotalTokens += metrics.Usage.TotalTokens
			usageReceived = true
		}
	}
	merge := func(summaries SynthesisResult, distill DistillResult) {
		result.SummariesWritten += summaries.SummariesWritten
		result.SummariesSkipped += summaries.SummariesSkipped
		written := distillationCount(distill)
		result.DistillationWritten += written
		result.DistillationSkipped += boolCount(distill.Skipped && written == 0)
		result.Committed = append(result.Committed, summaries.Committed...)
		result.Committed = append(result.Committed, distill.Committed...)
		mergeMetrics(summaries.Metrics)
		mergeMetrics(distill.Metrics)
	}

	switch mode {
	case SynthModeDefault, SynthModeSummarize:
		summaries, err := runSynthesis(ctx, workspace, provider, config, allSynthesisTools, options)
		merge(summaries, DistillResult{})
		returnErr = err
		if returnErr == nil && mode == SynthModeDefault {
			distill, err := runDistill(ctx, workspace, provider, config, allDistillTools, options)
			merge(SynthesisResult{}, distill)
			returnErr = err
		}
	case SynthModeDistill:
		distill, err := runDistill(ctx, workspace, provider, config, allDistillTools, options)
		merge(SynthesisResult{}, distill)
		returnErr = err
	default:
		returnErr = internalerrors.Validation(fmt.Sprintf("unknown synth mode: %s", mode))
	}
	if !usageKnown || !usageReceived {
		result.Metrics.Usage = nil
	}

	operation := newOperation(CommandSynth, options.Actor)
	operation.Metrics = &domain.OperationMetrics{
		Turns: result.Metrics.Turns, ToolCalls: result.Metrics.ToolCalls, Duration: result.Metrics.Duration,
		SummariesWritten: result.SummariesWritten, SummariesSkipped: result.SummariesSkipped,
		DistillationWritten: result.DistillationWritten, DistillationSkipped: result.DistillationSkipped,
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
