package source

import (
	"context"
	"errors"
	"fmt"
	"io"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const MaxSourceBytes = 10 << 20

var ErrNotHandled = errors.New("source not handled")

func ReadAll(reader io.Reader) ([]byte, error) {
	data, err := io.ReadAll(io.LimitReader(reader, MaxSourceBytes+1))
	if err != nil {
		return nil, err
	}
	if len(data) > MaxSourceBytes {
		return nil, internalerrors.Source(fmt.Sprintf("source exceeds %d-byte limit", MaxSourceBytes))
	}
	return data, nil
}

type OriginType string

const (
	OriginHTML     OriginType = "html"
	OriginYouTube  OriginType = "youtube"
	OriginMarkdown OriginType = "markdown"
)

type Origin struct {
	Type      OriginType
	SourceKey string
	Value     string
}

func NewOrigin(kind OriginType, sourceKey, value string) Origin {
	return Origin{Type: kind, SourceKey: sourceKey, Value: value}
}

type Transport interface {
	Route(context.Context, string) (Origin, error)
}

type Plugin interface {
	Handle(context.Context, Origin) (domain.RawSnapshot, error)
}

type Fetcher interface {
	Fetch(context.Context, string) (domain.RawSnapshot, error)
}

type Workflow struct {
	transports []Transport
	plugins    map[OriginType]Plugin
}

func NewWorkflow(transports []Transport, plugins map[OriginType]Plugin) *Workflow {
	registered := make(map[OriginType]Plugin, len(plugins))
	for kind, plugin := range plugins {
		registered[kind] = plugin
	}
	return &Workflow{transports: append([]Transport(nil), transports...), plugins: registered}
}

func (w *Workflow) Fetch(ctx context.Context, input string) (domain.RawSnapshot, error) {
	if w == nil {
		return domain.RawSnapshot{}, internalerrors.Request("source workflow is not configured")
	}
	if err := ctx.Err(); err != nil {
		return domain.RawSnapshot{}, internalerrors.Context(err)
	}
	for _, transport := range w.transports {
		origin, err := transport.Route(ctx, input)
		if errors.Is(err, ErrNotHandled) {
			continue
		}
		if err != nil {
			return domain.RawSnapshot{}, normalizeError(err, "routing source")
		}
		kind := origin.Type
		plugin, ok := w.plugins[kind]
		if !ok || plugin == nil {
			return domain.RawSnapshot{}, internalerrors.Source(fmt.Sprintf("no plugin for source type %q", kind))
		}
		snapshot, err := plugin.Handle(ctx, origin)
		if err != nil {
			return domain.RawSnapshot{}, normalizeError(err, "handling source")
		}
		if snapshot.SourceKey == "" {
			snapshot.SourceKey = origin.SourceKey
		}
		return snapshot, nil
	}
	return domain.RawSnapshot{}, internalerrors.Source("input is not a supported source")
}

func normalizeError(err error, detail string) error {
	if err == nil {
		return nil
	}
	var categorized *internalerrors.Error
	if errors.As(err, &categorized) {
		return err
	}
	if contextErr := internalerrors.Context(err); contextErr != nil {
		return contextErr
	}
	return internalerrors.Wrap(internalerrors.KindSource, detail, err)
}
