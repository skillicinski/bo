package file

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

type Transport struct{}

func NewTransport() *Transport { return &Transport{} }

func (t *Transport) Route(_ context.Context, input string) (source.Origin, error) {
	extension := filepath.Ext(input)
	if !strings.EqualFold(extension, ".md") {
		if extension != "" {
			return source.Origin{}, internalerrors.Unsupported("file extension must be .md")
		}
		return source.Origin{}, source.ErrNotHandled
	}
	filename := filepath.Base(input)
	return source.NewOrigin(source.OriginMarkdown, "raw:"+filename, input), nil
}

type MarkdownPlugin struct{}

func NewMarkdownPlugin() *MarkdownPlugin { return &MarkdownPlugin{} }
func NewMarkdown() *MarkdownPlugin       { return NewMarkdownPlugin() }

func (p *MarkdownPlugin) Type() source.OriginType { return source.OriginMarkdown }

func (p *MarkdownPlugin) Handle(_ context.Context, origin source.Origin) (domain.RawSnapshot, error) {
	data, err := os.ReadFile(origin.Value)
	if err != nil {
		return domain.RawSnapshot{}, internalerrors.Filesystem(fmt.Sprintf("reading %s failed: %v", origin.Value, err))
	}
	if len(strings.TrimSpace(string(data))) == 0 {
		return domain.RawSnapshot{}, internalerrors.Content("Markdown file is empty")
	}
	title := markdownTitle(data, filepath.Base(origin.Value))
	return domain.NewRawSnapshot(origin.SourceKey, title, data), nil
}

func markdownTitle(data []byte, filename string) string {
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "# ") {
			if title := strings.TrimSpace(strings.TrimPrefix(line, "# ")); title != "" {
				return title
			}
		}
	}
	return strings.TrimSuffix(filename, filepath.Ext(filename))
}
