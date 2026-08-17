package bo_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/skillicinski/bo"
	"github.com/skillicinski/bo/storage/local"
)

type pageSource map[string]bo.Page

func (s pageSource) Fetch(_ context.Context, url string) (bo.Page, error) {
	return s[url], nil
}

func seededStore(t *testing.T) (*local.Store, string) {
	t.Helper()
	home := t.TempDir()
	target, err := local.Seed(home, stringPtr("notes"))
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

func stringPtr(value string) *string { return &value }

func TestStateJSONIsStable(t *testing.T) {
	data, err := bo.MarshalState(bo.State{})
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "{\n  \"raw\": [],\n  \"summaries\": []\n}\n" {
		t.Fatalf("unexpected state: %q", data)
	}
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

func TestSnapPublishesStateSequentially(t *testing.T) {
	store, target := seededStore(t)
	source := pageSource{
		"https://example.test/one": {Title: "First Page", Markdown: "# First Page\n\ncontent\n"},
		"https://example.test/two": {Title: "Second Page", Markdown: "# Second Page\n\ncontent\n"},
	}
	outcomes, err := bo.Snap(context.Background(), store, source, []string{"https://example.test/one", "https://example.test/two"})
	if err != nil {
		t.Fatal(err)
	}
	if len(outcomes) != 2 || outcomes[0].Filename != "first-page.md" || outcomes[1].Filename != "second-page.md" {
		t.Fatalf("unexpected outcomes: %#v", outcomes)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Raw) != 2 || state.Raw[0].URL != "https://example.test/one" {
		t.Fatalf("unexpected state: %#v", state)
	}
	if _, err := os.Stat(filepath.Join(target, "first-page.md")); err != nil {
		t.Fatal(err)
	}
}

func TestSnapRollsBackRawWhenStatePublicationFails(t *testing.T) {
	store, target := seededStore(t)
	if err := os.Mkdir(filepath.Join(target, ".state.json.tmp"), 0o755); err != nil {
		t.Fatal(err)
	}
	_, err := bo.Snap(context.Background(), store, pageSource{"url": {Title: "Page", Markdown: "content\n"}}, []string{"url"})
	if err == nil {
		t.Fatal("Snap succeeded")
	}
	var commandErr *bo.SnapCommandError
	if !errors.As(err, &commandErr) || commandErr.SourceURL != "url" {
		t.Fatalf("unexpected error: %v", err)
	}
	if _, statErr := os.Stat(filepath.Join(target, "page.md")); !os.IsNotExist(statErr) {
		t.Fatalf("raw document remains: %v", statErr)
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

func TestKebabCase(t *testing.T) {
	got, err := bo.KebabCase(" Hello, World! ")
	if err != nil || got != "hello-world" {
		t.Fatalf("got %q, %v", got, err)
	}
	if _, err := bo.KebabCase("!!!"); !bo.IsCategory(err, bo.CategoryContent) {
		t.Fatalf("expected content error, got %v", err)
	}
}
