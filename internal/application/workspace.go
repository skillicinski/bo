package application

import (
	"context"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func Seed(ctx context.Context, creator WorkspaceCreator, name string, options OperationOptions) (created string, returnErr error) {
	options = normalizeOperationOptions(options)
	operation := newOperation(CommandSeed, options.Actor)
	if creator == nil {
		return "", internalerrors.Request("workspace creator is not configured")
	}
	created, returnErr = creator.Create(ctx, name, committedOperation(operation))
	returnErr = normalizeError(returnErr, internalerrors.KindFilesystem, "creating workspace")
	if returnErr != nil {
		return created, returnErr
	}
	return created, nil
}
