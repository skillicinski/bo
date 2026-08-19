package application_test

import (
	"testing"

	"github.com/skillicinski/bo/internal/application"
)

func TestKebabCase(t *testing.T) {
	got, err := application.KebabCase(" Hello, World! ")
	if err != nil || got != "hello-world" {
		t.Fatalf("got %q, %v", got, err)
	}
	if _, err := application.KebabCase("!!!"); !application.IsCategory(err, application.CategoryContent) {
		t.Fatalf("expected content error, got %v", err)
	}
}
