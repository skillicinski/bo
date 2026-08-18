package bo_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/skillicinski/bo"
)

type pageSource map[string]bo.Page

func (s pageSource) Fetch(_ context.Context, url string) (bo.Page, error) {
	return s[url], nil
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

type failingPageSource struct{}

func (failingPageSource) Fetch(context.Context, string) (bo.Page, error) {
	return bo.Page{}, errors.New("caption unavailable")
}

func TestSnapDoesNotStoreFailedFetch(t *testing.T) {
	store, target := seededStore(t)
	outcomes, err := bo.Snap(context.Background(), store, failingPageSource{}, []string{"https://www.youtube.com/watch?v=a1mhk7mAetk"})
	if err != nil {
		t.Fatal(err)
	}
	if len(outcomes) != 1 || outcomes[0].Err == nil {
		t.Fatalf("outcomes = %#v", outcomes)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Raw) != 0 {
		t.Fatalf("state = %#v", state)
	}
	entries, err := os.ReadDir(target)
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if filepath.Ext(entry.Name()) == ".md" {
			t.Fatalf("failed snapshot remains: %s", entry.Name())
		}
	}
}
