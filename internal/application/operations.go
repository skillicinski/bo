package application

import (
	"context"
	"errors"
	"time"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const operationEventWriteTimeout = 5 * time.Second

func normalizeOperationOptions(options OperationOptions) OperationOptions {
	if options.Actor == "" {
		options.Actor = "system"
	}
	return options
}

func newOperation(command OperationCommand, actor string) Operation {
	return Operation{
		OperationID: domain.NewOperationID(),
		Attempt:     1,
		Timestamp:   time.Now().UTC().Format(time.RFC3339Nano),
		Actor:       actor,
		Command:     command,
	}
}

func committedOperation(operation Operation) Operation {
	operation.Outcome = domain.OutcomeCommitted
	operation.Error = nil
	return operation
}

func failedOperation(operation Operation, cause error) Operation {
	operation.Outcome = domain.OutcomeFailed
	operation.Error = operationError(cause)
	return operation
}

func operationError(err error) *domain.OperationError {
	if err == nil {
		return nil
	}
	result := &domain.OperationError{Kind: "unknown"}
	var categorized *internalerrors.Error
	if errors.As(err, &categorized) {
		result.Kind = operationErrorKind(categorized.Kind)
		result.Retryable = categorized.Retryable
	}
	if contextErr := internalerrors.Context(err); contextErr != nil {
		result.Kind = operationErrorKind(contextErr.Kind)
		result.Retryable = contextErr.Retryable
	}
	return result
}

func operationErrorKind(kind internalerrors.Kind) string {
	switch kind {
	case internalerrors.KindRequest, internalerrors.KindValidation, internalerrors.KindSource,
		internalerrors.KindFilesystem, internalerrors.KindMissingResource, internalerrors.KindConflict,
		internalerrors.KindAlreadyExists, internalerrors.KindProviderTransport, internalerrors.KindProviderRejected,
		internalerrors.KindProviderMalformed, internalerrors.KindCanceled, internalerrors.KindDeadline:
		return string(kind)
	default:
		return "unknown"
	}
}

func commitOperationEvent(ctx context.Context, workspace Workspace, operation Operation) error {
	operation.Normalize()
	eventContext, cancel := context.WithTimeout(context.WithoutCancel(ctx), operationEventWriteTimeout)
	defer cancel()
	if err := workspace.CommitEvent(eventContext, operation); err != nil {
		return normalizeError(err, internalerrors.KindFilesystem, "committing operation event")
	}
	return nil
}

func recordFailedOperation(ctx context.Context, workspace Workspace, operation Operation, cause error) error {
	failed := failedOperation(operation, cause)
	return commitOperationEvent(ctx, workspace, failed)
}
