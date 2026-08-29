package local

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

func setWorkspaceNames(t *testing.T, names ...string) *int {
	t.Helper()
	original := randomWorkspaceName
	calls := 0
	randomWorkspaceName = func() (string, error) {
		if calls == len(names) {
			return "", errors.New("unexpected workspace name attempt")
		}
		name := names[calls]
		calls++
		return name, nil
	}
	t.Cleanup(func() { randomWorkspaceName = original })
	return &calls
}

func TestSeedRetriesGeneratedNameCollision(t *testing.T) {
	home := t.TempDir()
	if err := os.MkdirAll(filepath.Join(home, ".bo", "first"), 0o700); err != nil {
		t.Fatal(err)
	}
	calls := setWorkspaceNames(t, "first", "second")
	target, err := Seed(home, nil)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Base(target) != "second" || *calls != 2 {
		t.Fatalf("target = %q, name attempts = %d", target, *calls)
	}
}

func TestSeedExhaustsGeneratedNameCollisions(t *testing.T) {
	home := t.TempDir()
	names := []string{"one", "two", "three", "four", "five", "six", "seven", "eight"}
	for _, name := range names {
		if err := os.MkdirAll(filepath.Join(home, ".bo", name), 0o700); err != nil {
			t.Fatal(err)
		}
	}
	calls := setWorkspaceNames(t, names...)
	target, err := Seed(home, nil)
	if target != "" || !internalerrors.IsKind(err, internalerrors.KindAlreadyExists) || !errors.Is(err, internalerrors.ErrAlreadyExists) {
		t.Fatalf("target = %q, error = %v", target, err)
	}
	if *calls != maxGeneratedWorkspaceAttempts {
		t.Fatalf("name attempts = %d", *calls)
	}
}

func TestSeedExplicitNameDoesNotRetryCollision(t *testing.T) {
	home := t.TempDir()
	name := "requested"
	if err := os.MkdirAll(filepath.Join(home, ".bo", name), 0o700); err != nil {
		t.Fatal(err)
	}
	calls := setWorkspaceNames(t, "unused")
	target, err := Seed(home, &name)
	if target != "" || !internalerrors.IsKind(err, internalerrors.KindAlreadyExists) || !errors.Is(err, internalerrors.ErrAlreadyExists) {
		t.Fatalf("target = %q, error = %v", target, err)
	}
	if *calls != 0 {
		t.Fatalf("generated name attempts = %d", *calls)
	}
}
