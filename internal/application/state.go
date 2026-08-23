package application

import (
	"context"

	"github.com/skillicinski/bo/internal/domain"
)

func ReadState(ctx context.Context, storage Storage, directory string, options OperationOptions) (state domain.State, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return domain.State{}, err
	}
	defer func() {
		recordOperation(options, directory, CommandState, returnErr == nil, operationErrorDetails(returnErr))
	}()
	state, _, returnErr = storage.ReadState(ctx)
	return state, returnErr
}
