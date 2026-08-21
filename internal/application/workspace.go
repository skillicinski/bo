package application

import (
	"context"
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
		return "", RequestError("workspace creator is not configured")
	}
	return creator.Create(ctx, name)
}
