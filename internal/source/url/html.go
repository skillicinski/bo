package url

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"strings"
	"unicode"

	htmltomarkdown "github.com/JohannesKaufmann/html-to-markdown/v2"
	"golang.org/x/net/html"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

type HTMLPlugin struct {
	Client    Requester
	UserAgent string
}

func NewHTML(client Requester) *HTMLPlugin {
	return &HTMLPlugin{Client: client, UserAgent: DefaultUserAgent}
}

func (p *HTMLPlugin) Handle(ctx context.Context, origin source.Origin) (domain.RawSnapshot, error) {
	client := p.client()
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, origin.Value, nil)
	if err != nil {
		return domain.RawSnapshot{}, internalerrors.Wrap(internalerrors.KindValidation, "invalid URL", err)
	}
	request.Header.Set("User-Agent", p.userAgent())
	response, err := client.Do(request)
	if err != nil {
		return domain.RawSnapshot{}, sourceFailure("request failed", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return domain.RawSnapshot{}, internalerrors.Source(fmt.Sprintf("source returned HTTP %d", response.StatusCode))
	}
	contentType := response.Header.Get("Content-Type")
	if !isHTMLContentType(contentType) {
		return domain.RawSnapshot{}, internalerrors.Source(fmt.Sprintf("not HTML (Content-Type: %s)", contentType))
	}
	body, err := source.ReadAll(response.Body)
	if err != nil {
		return domain.RawSnapshot{}, sourceFailure("reading response failed", err)
	}
	snapshot, err := ExtractHTML(string(body))
	if err != nil {
		return domain.RawSnapshot{}, err
	}
	snapshot.SourceKey = origin.SourceKey
	return snapshot, nil
}

func (p *HTMLPlugin) client() Requester {
	if p == nil || p.Client == nil {
		return http.DefaultClient
	}
	return p.Client
}

func (p *HTMLPlugin) userAgent() string {
	if p == nil || p.UserAgent == "" {
		return DefaultUserAgent
	}
	return p.UserAgent
}

func isHTMLContentType(value string) bool {
	mediaType := strings.ToLower(strings.TrimSpace(strings.SplitN(value, ";", 2)[0]))
	return mediaType == "text/html" || mediaType == "application/xhtml+xml"
}

var skippedTags = map[string]bool{
	"aside": true, "footer": true, "form": true, "header": true, "nav": true,
	"noscript": true, "script": true, "style": true, "svg": true, "template": true,
}

func ExtractHTML(input string) (domain.RawSnapshot, error) {
	document, err := html.Parse(strings.NewReader(input))
	if err != nil {
		return domain.RawSnapshot{}, internalerrors.Wrap(internalerrors.KindSource, "HTML parsing failed", err)
	}
	title := strings.Join(strings.Fields(textContent(findElement(document, "title"))), " ")
	if title == "" {
		return domain.RawSnapshot{}, internalerrors.Source("page has no title")
	}
	removeSkipped(document)
	for _, tag := range []string{"article", "main", "body"} {
		element := findElement(document, tag)
		if element == nil {
			continue
		}
		var rendered bytes.Buffer
		if err := html.Render(&rendered, element); err != nil {
			return domain.RawSnapshot{}, internalerrors.Wrap(internalerrors.KindSource, "HTML rendering failed", err)
		}
		markdown, err := htmltomarkdown.ConvertString(rendered.String())
		if err != nil {
			return domain.RawSnapshot{}, internalerrors.Wrap(internalerrors.KindSource, "HTML conversion failed", err)
		}
		markdown = removeMatchingTitleHeading(markdown, title)
		if hasAlphaNumeric(markdown) {
			return domain.NewRawSnapshot("", title, []byte(fmt.Sprintf("# %s\n\n%s\n", title, strings.TrimSpace(markdown)))), nil
		}
	}
	return domain.RawSnapshot{}, internalerrors.Source("page has no readable content")
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
