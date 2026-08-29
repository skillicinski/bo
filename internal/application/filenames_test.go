package application_test

import (
	"testing"

	"github.com/skillicinski/bo/internal/application"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func TestKebabCase(t *testing.T) {
	got, err := application.KebabCase(" Hello, World! ")
	if err != nil || got != "hello-world" {
		t.Fatalf("got %q, %v", got, err)
	}
	if _, err := application.KebabCase("!!!"); !internalerrors.IsKind(err, internalerrors.KindValidation) {
		t.Fatalf("expected validation error, got %v", err)
	}
}
