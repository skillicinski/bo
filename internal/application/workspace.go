package application

import (
	"context"
)

func Seed(ctx context.Context, creator WorkspaceCreator, name string) (string, error) {
	if creator == nil {
		return "", RequestError("workspace creator is not configured")
	}
	return creator.Create(ctx, name)
}
