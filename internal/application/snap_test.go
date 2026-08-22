package application_test

import (
	"context"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/source"
	filesource "github.com/skillicinski/bo/internal/source/file"
	urlsource "github.com/skillicinski/bo/internal/source/url"
)

type rawSource map[string]domain.RawSnapshot

func (s rawSource) Fetch(_ context.Context, input string) (domain.RawSnapshot, error) {
	return s[input], nil
}

func TestSnapPublishesStateSequentially(t *testing.T) {
	store, target := seededStore(t)
	source := rawSource{
		"https://example.test/one": {Title: "First Page", Markdown: []byte("# First Page\n\ncontent\n")},
		"https://example.test/two": {Title: "Second Page", Markdown: []byte("# Second Page\n\ncontent\n")},
	}
	outcomes, err := application.SnapWithWorkflow(context.Background(), store, source, "notes", []string{"https://example.test/one", "https://example.test/two"}, operationOptionsFor(target))
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
	_, err := application.SnapWithWorkflow(context.Background(), store, rawSource{"url": {Title: "Page", Markdown: []byte("content\n")}}, "notes", []string{"url"}, operationOptionsFor(target))
	if err == nil {
		t.Fatal("Snap succeeded")
	}
	var commandErr *application.SnapCommandError
	if !errors.As(err, &commandErr) || commandErr.SourceKey != "url" {
		t.Fatalf("unexpected error: %v", err)
	}
	if _, statErr := os.Stat(filepath.Join(target, "page.md")); !os.IsNotExist(statErr) {
		t.Fatalf("raw document remains: %v", statErr)
	}
}

type failingRawSource struct{}

func (failingRawSource) Fetch(context.Context, string) (domain.RawSnapshot, error) {
	return domain.RawSnapshot{}, errors.New("caption unavailable")
}

func TestSnapDoesNotStoreFailedFetch(t *testing.T) {
	store, target := seededStore(t)
	outcomes, err := application.SnapWithWorkflow(context.Background(), store, failingRawSource{}, "notes", []string{"https://www.youtube.com/watch?v=a1mhk7mAetk"}, operationOptionsFor(target))
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

func TestSnapAcceptsMixedURLAndMarkdownInputs(t *testing.T) {
	store, target := seededStore(t)
	client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		header := make(http.Header)
		header.Set("Content-Type", "text/html")
		return &http.Response{StatusCode: http.StatusOK, Header: header, Body: io.NopCloser(strings.NewReader("<html><head><title>Remote</title></head><body><article>remote content</article></body></html>"))}, nil
	})}
	path := filepath.Join(t.TempDir(), "local.md")
	if err := os.WriteFile(path, []byte("# Local\n\nlocal content\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	workflow := source.NewWorkflow(
		[]source.Transport{urlsource.NewTransport(), filesource.NewTransport()},
		map[source.OriginType]source.Plugin{
			source.OriginHTML:     urlsource.NewHTML(client),
			source.OriginMarkdown: filesource.NewMarkdownPlugin(),
		},
	)
	remote := "https://example.test/remote"
	outcomes, err := application.SnapWithWorkflow(context.Background(), store, workflow, "notes", []string{remote, path}, operationOptionsFor(target))
	if err != nil || len(outcomes) != 2 {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
	if outcomes[0].SourceKey != remote || outcomes[1].SourceKey != "raw:local.md" {
		t.Fatalf("outcomes = %#v", outcomes)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || len(state.Raw) != 2 || state.Raw[1].URL != "raw:local.md" {
		t.Fatalf("state = %#v, err = %v", state, err)
	}
}

func TestSnapDefaultWorkflowAcceptsMarkdown(t *testing.T) {
	store, target := seededStore(t)
	path := filepath.Join(t.TempDir(), "default.md")
	if err := os.WriteFile(path, []byte("# Default\n\ncontent\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	outcomes, err := application.Snap(context.Background(), store, "notes", []string{path}, operationOptionsFor(target))
	if err != nil || len(outcomes) != 1 || outcomes[0].SourceKey != "raw:default.md" {
		t.Fatalf("outcomes = %#v, err = %v", outcomes, err)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }
