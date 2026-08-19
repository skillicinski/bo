package url

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestExtractHTMLRemovesChromeAndDuplicateTitle(t *testing.T) {
	page, err := ExtractHTML("<html><head><title> Example   Page </title><script>bad</script></head><body><header>menu</header><article><h1>Example Page</h1><p>Hello <strong>world</strong>.</p><nav>skip</nav></article><footer>footer</footer></body></html>")
	if err != nil {
		t.Fatal(err)
	}
	if page.Title != "Example Page" {
		t.Fatalf("title = %q", page.Title)
	}
	if page.Markdown != "# Example Page\n\nHello **world**.\n" {
		t.Fatalf("markdown = %q", page.Markdown)
	}
}

func TestExtractHTMLUsesMainAndPreservesCode(t *testing.T) {
	page, err := ExtractHTML("<html><head><title>Code</title></head><body><main><p>Before</p><pre><code>one\ntwo</code></pre></main></body></html>")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(page.Markdown, "one\ntwo") {
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

func TestHTTPFetchClassifiesResponses(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		header := make(http.Header)
		if request.URL.Path == "/missing" {
			header.Set("X-Request-Id", "request-123")
			return &http.Response{StatusCode: http.StatusNotFound, Header: header, Body: io.NopCloser(strings.NewReader("private"))}, nil
		}
		header.Set("Content-Type", "text/html; charset=utf-8")
		return &http.Response{StatusCode: http.StatusOK, Header: header, Body: io.NopCloser(strings.NewReader("<html><head><title>Page</title></head><body><article>content</article></body></html>"))}, nil
	})}
	httpSource := New(client)
	page, err := httpSource.Fetch(context.Background(), "https://example.test/ok")
	if err != nil || page.SourceURL != "https://example.test/ok" || page.Markdown != "# Page\n\ncontent\n" {
		t.Fatalf("page = %#v, err = %v", page, err)
	}
	_, err = httpSource.Fetch(context.Background(), "https://example.test/missing")
	if err == nil || err.Error() != "http: HTTP 404 (request_id: request-123)" {
		t.Fatalf("HTTP error = %v", err)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }
