package url

import (
	"context"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"

	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

func TestExtractHTMLRemovesChromeAndDuplicateTitle(t *testing.T) {
	page, err := ExtractHTML("<html><head><title> Example   Page </title><script>bad</script></head><body><header>menu</header><article><h1>Example Page</h1><p>Hello <strong>world</strong>.</p><nav>skip</nav></article><footer>footer</footer></body></html>")
	if err != nil {
		t.Fatal(err)
	}
	if page.Title != "Example Page" {
		t.Fatalf("title = %q", page.Title)
	}
	if string(page.Markdown) != "# Example Page\n\nHello **world**.\n" {
		t.Fatalf("markdown = %q", page.Markdown)
	}
}

func TestExtractHTMLUsesMainAndPreservesCode(t *testing.T) {
	page, err := ExtractHTML("<html><head><title>Code</title></head><body><main><p>Before</p><pre><code>one\ntwo</code></pre></main></body></html>")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(page.Markdown), "one\ntwo") {
		t.Fatalf("code block was not preserved: %q", page.Markdown)
	}
}

func TestExtractHTMLErrorsOnMissingTitleOrContent(t *testing.T) {
	if _, err := ExtractHTML("<html><body><article>content</article></body></html>"); err == nil || !strings.Contains(err.Error(), "no title") {
		t.Fatalf("missing title error = %v", err)
	}
	if _, err := ExtractHTML("<html><head><title>Empty</title></head><body><article> </article></body></html>"); err == nil || !strings.Contains(err.Error(), "no readable content") {
		t.Fatalf("missing content error = %v", err)
	}
}

func TestURLWorkflowFetchesHTMLAndClassifiesResponses(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		header := make(http.Header)
		if request.URL.Path == "/missing" {
			header.Set("X-Request-Id", "request-123")
			return &http.Response{StatusCode: http.StatusNotFound, Header: header, Body: io.NopCloser(strings.NewReader("private"))}, nil
		}
		header.Set("Content-Type", "text/html; charset=utf-8")
		return &http.Response{StatusCode: http.StatusOK, Header: header, Body: io.NopCloser(strings.NewReader("<html><head><title>Page</title></head><body><article>content</article></body></html>"))}, nil
	})}
	workflow := source.NewWorkflow(
		[]source.Transport{NewTransport()},
		map[source.OriginType]source.Plugin{source.OriginHTML: NewHTML(client)},
	)
	page, err := workflow.Fetch(context.Background(), "https://example.test/ok")
	if err != nil || page.SourceKey != "https://example.test/ok" || string(page.Markdown) != "# Page\n\ncontent\n" {
		t.Fatalf("page = %#v, err = %v", page, err)
	}
	_, err = workflow.Fetch(context.Background(), "https://example.test/missing")
	if err == nil || !internalerrors.IsKind(err, internalerrors.KindSource) {
		t.Fatalf("HTTP error = %v", err)
	}
}

func TestURLWorkflowRejectsCredentialBearingURLsBeforeRequest(t *testing.T) {
	requests := 0
	client := &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		requests++
		return nil, errors.New("unexpected HTTP request")
	})}
	workflow := source.NewWorkflow(
		[]source.Transport{NewTransport()},
		map[source.OriginType]source.Plugin{source.OriginHTML: NewHTML(client)},
	)
	for _, input := range []string{
		"https://user:secret@example.test/article",
		"https://example.test/article?token=secret",
	} {
		_, err := workflow.Fetch(context.Background(), input)
		if !internalerrors.IsKind(err, internalerrors.KindValidation) {
			t.Fatalf("%s error = %v", input, err)
		}
		if requests != 0 {
			t.Fatalf("%s made %d HTTP requests", input, requests)
		}
	}
}

func TestHTMLPluginRejectsOversizedResponses(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
		header := make(http.Header)
		header.Set("Content-Type", "text/html")
		return &http.Response{StatusCode: http.StatusOK, Header: header, Body: io.NopCloser(strings.NewReader(strings.Repeat("x", source.MaxSourceBytes+1)))}, nil
	})}
	workflow := source.NewWorkflow(
		[]source.Transport{NewTransport()},
		map[source.OriginType]source.Plugin{source.OriginHTML: NewHTML(client)},
	)
	if _, err := workflow.Fetch(context.Background(), "https://example.test/large"); !internalerrors.IsKind(err, internalerrors.KindSource) {
		t.Fatalf("oversized HTML error = %v", err)
	}
}

func TestTransportRoutesURLsAndRejectsUnsupportedYouTubeURLs(t *testing.T) {
	transport := NewTransport()
	origin, err := transport.Route(context.Background(), "https://example.test/article")
	if err != nil || origin.Type != source.OriginHTML || origin.SourceKey != "https://example.test/article" {
		t.Fatalf("origin = %#v, err = %v", origin, err)
	}
	origin, err = transport.Route(context.Background(), "https://www.youtube.com/watch?v=a1mhk7mAetk")
	if err != nil || origin.Type != source.OriginYouTube {
		t.Fatalf("YouTube origin = %#v, err = %v", origin, err)
	}
	if _, err := transport.Route(context.Background(), "https://www.youtube.com/playlist?list=x"); err == nil || !strings.Contains(err.Error(), "out of scope") {
		t.Fatalf("unsupported YouTube error = %v", err)
	}
	if _, err := transport.Route(context.Background(), "https://example.test/article#credential"); !internalerrors.IsKind(err, internalerrors.KindValidation) {
		t.Fatalf("fragment URL error = %v", err)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }
