package source_test

import (
	"context"
	"errors"
	"testing"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

type transportFunc func(context.Context, string) (source.Origin, error)

func (f transportFunc) Route(ctx context.Context, input string) (source.Origin, error) {
	return f(ctx, input)
}

type pluginFunc func(context.Context, source.Origin) (domain.RawSnapshot, error)

func (f pluginFunc) Handle(ctx context.Context, origin source.Origin) (domain.RawSnapshot, error) {
	return f(ctx, origin)
}

func TestWorkflowRoutesInOrderAndUsesOriginType(t *testing.T) {
	called := false
	workflow := source.NewWorkflow(
		[]source.Transport{
			transportFunc(func(context.Context, string) (source.Origin, error) { return source.Origin{}, source.ErrNotHandled }),
			transportFunc(func(context.Context, string) (source.Origin, error) {
				return source.NewOrigin(source.OriginMarkdown, "raw:note.md", "note.md"), nil
			}),
		},
		map[source.OriginType]source.Plugin{
			source.OriginMarkdown: pluginFunc(func(_ context.Context, origin source.Origin) (domain.RawSnapshot, error) {
				called = true
				return domain.RawSnapshot{Title: origin.Value}, nil
			}),
		},
	)
	snapshot, err := workflow.Fetch(context.Background(), "note.md")
	if err != nil || !called || snapshot.SourceKey != "raw:note.md" || snapshot.Title != "note.md" {
		t.Fatalf("snapshot = %#v, called = %t, err = %v", snapshot, called, err)
	}
}

func TestWorkflowReportsUnsupportedAndMissingPlugins(t *testing.T) {
	workflow := source.NewWorkflow([]source.Transport{transportFunc(func(context.Context, string) (source.Origin, error) {
		return source.Origin{}, source.ErrNotHandled
	})}, nil)
	if _, err := workflow.Fetch(context.Background(), "input"); !internalerrors.IsKind(err, internalerrors.KindSource) {
		t.Fatalf("unsupported error = %v", err)
	}
	workflow = source.NewWorkflow([]source.Transport{transportFunc(func(context.Context, string) (source.Origin, error) {
		return source.NewOrigin(source.OriginHTML, "key", "value"), nil
	})}, nil)
	if _, err := workflow.Fetch(context.Background(), "input"); !internalerrors.IsKind(err, internalerrors.KindSource) {
		t.Fatalf("missing plugin error = %v", err)
	}
}

func TestWorkflowReturnsPluginError(t *testing.T) {
	want := errors.New("plugin failed")
	workflow := source.NewWorkflow([]source.Transport{transportFunc(func(context.Context, string) (source.Origin, error) {
		return source.NewOrigin(source.OriginHTML, "key", "value"), nil
	})}, map[source.OriginType]source.Plugin{
		source.OriginHTML: pluginFunc(func(context.Context, source.Origin) (domain.RawSnapshot, error) {
			return domain.RawSnapshot{}, want
		}),
	})
	if _, err := workflow.Fetch(context.Background(), "input"); !errors.Is(err, want) {
		t.Fatalf("plugin error = %v", err)
	}
}
