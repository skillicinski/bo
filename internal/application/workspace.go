package application

import (
	"context"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func Seed(ctx context.Context, creator WorkspaceCreator, name string, options OperationOptions) (created string, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return "", err
	}
	defer func() {
		directory := name
		if created != "" {
			directory = created
		}
		details := map[string]any{"name": directory}
		for key, value := range operationErrorDetails(returnErr) {
			details[key] = value
		}
		recordOperation(options, directory, CommandSeed, returnErr == nil, details)
	}()
	if creator == nil {
		return "", internalerrors.Request("workspace creator is not configured")
	}
	created, returnErr = creator.Create(ctx, name)
	return created, normalizeError(returnErr, internalerrors.KindFilesystem, "creating workspace")
}
