package application

import (
	"context"
	"encoding/json"
	"fmt"
)

func StateOutput(ctx context.Context, storage Storage, directory string, full bool, options OperationOptions) (output string, returnErr error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return "", err
	}
	defer func() {
		details := map[string]any{"full": full}
		for key, value := range operationErrorDetails(returnErr) {
			details[key] = value
		}
		recordOperation(options, directory, CommandState, returnErr == nil, details)
	}()
	state, _, err := storage.ReadState(ctx)
	if err != nil {
		return "", err
	}
	if !full {
		return fmt.Sprintf("%d documents snapped", len(state.Raw)), nil
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return "", err
	}
	return string(data), nil
}
