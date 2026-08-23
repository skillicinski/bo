package application

import (
	"context"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func ReadState(ctx context.Context, storage Storage, directory string, options OperationOptions) (state domain.State, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return domain.State{}, err
	}
	if storage == nil {
		return domain.State{}, internalerrors.Request("workspace storage is not configured")
	}
	defer func() {
		recordOperation(options, directory, CommandState, returnErr == nil, operationErrorDetails(returnErr))
	}()
	state, _, returnErr = storage.ReadState(ctx)
	returnErr = normalizeError(returnErr, internalerrors.KindFilesystem, "reading workspace state")
	return state, returnErr
}
