package bo_test

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/skillicinski/bo"
	"github.com/skillicinski/bo/storage/local"
)

func seededStore(t *testing.T) (*local.Store, string) {
	t.Helper()
	home := t.TempDir()
	name := "notes"
	target, err := local.Seed(home, &name)
	if err != nil {
		t.Fatal(err)
	}
	store, err := local.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })
	return store, target
}

func TestValidateNameUsesPortableRules(t *testing.T) {
	for _, name := range []string{"", ".", "..", "../escape", "sub/name", "sub\\name", "CON", "CON.txt", "COM1", "LPT9", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "name.", "name ", "line\nbreak"} {
		if err := local.ValidateName(name); err == nil {
			t.Errorf("ValidateName(%q) succeeded", name)
		}
	}
	for _, name := range []string{"test", "hello-world", "has space", ".hidden", "café"} {
		if err := local.ValidateName(name); err != nil {
			t.Errorf("ValidateName(%q): %v", name, err)
		}
	}
}

func TestLocalGenerationConflict(t *testing.T) {
	store, target := seededStore(t)
	state, generation, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "state.json"), []byte("changed\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err = store.PublishState(context.Background(), state, generation)
	if !bo.IsConflict(err) {
		t.Fatalf("expected conflict, got %v", err)
	}
}
