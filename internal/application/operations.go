package application

import (
	"context"
	"errors"
	"time"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func normalizeOperationOptions(options OperationOptions) (OperationOptions, error) {
	if options.Log == nil {
		return OperationOptions{}, internalerrors.Request("operation log is not configured")
	}
	if options.Actor == "" {
		options.Actor = "system"
	}
	return options, nil
}

func recordOperation(options OperationOptions, directory string, command OperationCommand, success bool, details map[string]any) {
	if details == nil {
		details = map[string]any{}
	}
	_ = options.Log.Append(context.Background(), Operation{
		Timestamp: time.Now().UTC().Format(time.RFC3339Nano),
		Actor:     options.Actor,
		Directory: directory,
		Command:   command,
		Success:   success,
		Details:   details,
	})
}

func operationErrorDetails(err error) map[string]any {
	if err == nil {
		return map[string]any{}
	}
	details := map[string]any{"error": err.Error()}
	var categorized *internalerrors.Error
	if errors.As(err, &categorized) {
		details["error"] = categorized.Detail
		details["error_kind"] = string(categorized.Kind)
	}
	return details
}
