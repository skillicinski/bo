package application

import (
	"context"
	"encoding/json"
	"fmt"
)

func StateOutput(ctx context.Context, storage Storage, full bool) (string, error) {
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
