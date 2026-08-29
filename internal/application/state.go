package application

import (
	"context"
	"errors"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func ReadState(ctx context.Context, workspace Workspace, options OperationOptions) (state domain.State, revision Revision, returnErr error) {
	options = normalizeOperationOptions(options)
	if workspace == nil {
		return domain.State{}, Revision{}, internalerrors.Request("workspace is not configured")
	}
	state, revision, returnErr = workspace.ReadState(ctx)
	returnErr = normalizeError(returnErr, internalerrors.KindFilesystem, "reading workspace state")
	operation := newOperation(CommandState, options.Actor)
	if returnErr != nil {
		return state, revision, recordStateFailure(ctx, workspace, operation, returnErr)
	}
	operation = committedOperation(operation)
	if err := commitOperationEvent(ctx, workspace, operation); err != nil {
		return state, revision, err
	}
	return state, revision, nil
}

func recordStateFailure(ctx context.Context, workspace Workspace, operation Operation, cause error) error {
	if err := recordFailedOperation(ctx, workspace, operation, cause); err != nil {
		return errors.Join(cause, err)
	}
	return cause
}
