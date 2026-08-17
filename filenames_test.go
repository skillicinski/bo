package bo_test

import (
	"testing"

	"github.com/skillicinski/bo"
)

func TestKebabCase(t *testing.T) {
	got, err := bo.KebabCase(" Hello, World! ")
	if err != nil || got != "hello-world" {
		t.Fatalf("got %q, %v", got, err)
	}
	if _, err := bo.KebabCase("!!!"); !bo.IsCategory(err, bo.CategoryContent) {
		t.Fatalf("expected content error, got %v", err)
	}
}
