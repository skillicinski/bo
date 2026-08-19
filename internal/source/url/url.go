package url

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode"

	htmltomarkdown "github.com/JohannesKaufmann/html-to-markdown/v2"
	"golang.org/x/net/html"

	"github.com/skillicinski/bo/internal/application"
)

const DefaultUserAgent = "bo/0.1"

type HTTP struct {
	Client         *http.Client
	UserAgent      string
	PlayerEndpoint string
}

func NewHTTP() *HTTP {
	return &HTTP{
		Client:         &http.Client{Timeout: 30 * time.Second},
		UserAgent:      DefaultUserAgent,
		PlayerEndpoint: playerEndpoint,
	}
}

func New(client *http.Client) *HTTP {
	if client == nil {
		return NewHTTP()
	}
	return &HTTP{Client: client, UserAgent: DefaultUserAgent, PlayerEndpoint: playerEndpoint}
}

func (h *HTTP) Fetch(ctx context.Context, input string) (application.Page, error) {
	match := ClassifyYouTubeURL(input)
	if match.Kind == YouTubeSupported {
		return h.fetchYouTube(ctx, match)
	}
	if match.Kind == YouTubeUnsupported {
		return application.Page{}, application.UnsupportedError("YouTube URL: " + match.Reason)
	}
	parsed, err := url.Parse(input)
	if err != nil {
		return application.Page{}, application.InputError(fmt.Sprintf("invalid URL: %v", err))
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return application.Page{}, application.InputError("URL scheme must be http or https")
	}
	client := h.client()
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, parsed.String(), nil)
	if err != nil {
		return application.Page{}, application.InputError(fmt.Sprintf("invalid URL: %v", err))
	}
	request.Header.Set("User-Agent", h.userAgent())
	response, err := client.Do(request)
	if err != nil {
		return application.Page{}, application.RequestError(fmt.Sprintf("request failed: %v", err))
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return application.Page{}, application.HTTPError(response.StatusCode, response.Header.Get("X-Request-Id"))
	}
	contentType := response.Header.Get("Content-Type")
	if !isHTMLContentType(contentType) {
		return application.Page{}, application.ContentError(fmt.Sprintf("not HTML (Content-Type: %s)", contentType))
	}
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return application.Page{}, application.ContentError(fmt.Sprintf("reading response failed: %v", err))
	}
	page, err := ExtractHTML(string(body))
	if err != nil {
		return application.Page{}, err
	}
	page.SourceURL = input
	return page, nil
}

func (h *HTTP) client() *http.Client {
	if h == nil || h.Client == nil {
		return NewHTTP().Client
	}
	return h.Client
}

func (h *HTTP) userAgent() string {
	if h == nil || h.UserAgent == "" {
		return DefaultUserAgent
	}
	return h.UserAgent
}

func isHTMLContentType(value string) bool {
	mediaType := strings.ToLower(strings.TrimSpace(strings.SplitN(value, ";", 2)[0]))
	return mediaType == "text/html" || mediaType == "application/xhtml+xml"
}

var skippedTags = map[string]bool{
	"aside": true, "footer": true, "form": true, "header": true, "nav": true,
	"noscript": true, "script": true, "style": true, "svg": true, "template": true,
}

func ExtractHTML(input string) (application.Page, error) {
	document, err := html.Parse(strings.NewReader(input))
	if err != nil {
		return application.Page{}, application.ContentError(fmt.Sprintf("HTML parsing failed: %v", err))
	}
	title := strings.Join(strings.Fields(textContent(findElement(document, "title"))), " ")
	if title == "" {
		return application.Page{}, application.ContentError("page has no title")
	}
	removeSkipped(document)
	for _, tag := range []string{"article", "main", "body"} {
		element := findElement(document, tag)
		if element == nil {
			continue
		}
		var rendered bytes.Buffer
		if err := html.Render(&rendered, element); err != nil {
			return application.Page{}, application.ContentError(fmt.Sprintf("HTML rendering failed: %v", err))
		}
		markdown, err := htmltomarkdown.ConvertString(rendered.String())
		if err != nil {
			return application.Page{}, application.ContentError(fmt.Sprintf("HTML conversion failed: %v", err))
		}
		markdown = removeMatchingTitleHeading(markdown, title)
		if hasAlphaNumeric(markdown) {
			return application.Page{Title: title, Markdown: fmt.Sprintf("# %s\n\n%s\n", title, strings.TrimSpace(markdown))}, nil
		}
	}
	return application.Page{}, application.ContentError("page has no readable content")
}

func findElement(node *html.Node, wanted string) *html.Node {
	if node.Type == html.ElementNode && node.Data == wanted {
		return node
	}
	for child := node.FirstChild; child != nil; child = child.NextSibling {
		if found := findElement(child, wanted); found != nil {
			return found
		}
	}
	return nil
}

func textContent(node *html.Node) string {
	if node == nil {
		return ""
	}
	if node.Type == html.TextNode {
		return node.Data
	}
	var builder strings.Builder
	for child := node.FirstChild; child != nil; child = child.NextSibling {
		builder.WriteString(textContent(child))
	}
	return builder.String()
}

func removeSkipped(node *html.Node) {
	for child := node.FirstChild; child != nil; {
		next := child.NextSibling
		if child.Type == html.ElementNode && skippedTags[child.Data] {
			node.RemoveChild(child)
		} else {
			removeSkipped(child)
		}
		child = next
	}
}

func removeMatchingTitleHeading(markdown, title string) string {
	lines := strings.Split(markdown, "\n")
	if len(lines) == 0 {
		return ""
	}
	if strings.HasPrefix(lines[0], "# ") && strings.EqualFold(strings.TrimSpace(strings.TrimPrefix(lines[0], "# ")), strings.TrimSpace(title)) {
		lines = lines[1:]
	}
	return strings.TrimSpace(strings.Join(lines, "\n"))
}

func hasAlphaNumeric(value string) bool {
	for _, r := range value {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			return true
		}
	}
	return false
}
