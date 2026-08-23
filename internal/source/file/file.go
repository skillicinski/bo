package file

import (
	"context"
	"errors"
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
			return source.Origin{}, internalerrors.Source("file extension must be .md")
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

func (p *MarkdownPlugin) Handle(ctx context.Context, origin source.Origin) (domain.RawSnapshot, error) {
	if err := ctx.Err(); err != nil {
		return domain.RawSnapshot{}, internalerrors.Context(err)
	}
	data, err := os.ReadFile(origin.Value)
	if err != nil {
		kind := internalerrors.KindFilesystem
		if errors.Is(err, os.ErrNotExist) {
			kind = internalerrors.KindMissingResource
		}
		return domain.RawSnapshot{}, internalerrors.Wrap(kind, fmt.Sprintf("reading %s failed", origin.Value), err)
	}
	if len(strings.TrimSpace(string(data))) == 0 {
		return domain.RawSnapshot{}, internalerrors.Source("Markdown file is empty")
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
