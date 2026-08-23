package application

import (
	"context"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func ReadState(ctx context.Context, workspace Workspace, directory string, options OperationOptions) (state domain.State, revision Revision, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return domain.State{}, Revision{}, err
	}
	if workspace == nil {
		return domain.State{}, Revision{}, internalerrors.Request("workspace is not configured")
	}
	defer func() {
		recordOperation(options, directory, CommandState, returnErr == nil, operationErrorDetails(returnErr))
	}()
	state, revision, returnErr = workspace.ReadState(ctx)
	returnErr = normalizeError(returnErr, internalerrors.KindFilesystem, "reading workspace state")
	return state, revision, returnErr
}
