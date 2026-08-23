package bo_test

import (
	"context"
	"errors"
	"testing"

	"github.com/skillicinski/bo"
)

func TestErrorContractWrapsCause(t *testing.T) {
	cause := errors.New("disk unavailable")
	err := bo.WrapError(bo.ErrorKindFilesystem, "reading state", cause)
	var typed *bo.Error
	if !errors.As(err, &typed) || typed.Kind != bo.ErrorKindFilesystem {
		t.Fatalf("error = %#v", err)
	}
	if !errors.Is(err, cause) {
		t.Fatal("wrapped cause is not discoverable")
	}
}

func TestLocalMissingWorkspaceUsesStableKind(t *testing.T) {
	_, err := bo.NewLocalManager(t.TempDir()).Open(context.Background(), "missing")
	if !bo.IsKind(err, bo.ErrorKindMissingResource) {
		t.Fatalf("error = %v", err)
	}
}

func TestSeedBridgesPublicCreatorErrors(t *testing.T) {
	manager := bo.NewLocalManager(t.TempDir())
	options := bo.OperationOptions{Log: errorLog{}}

	if _, err := bo.Seed(context.Background(), bo.SeedRequest{
		Creator: manager, Name: "bad/name", Operations: options,
	}); !bo.IsKind(err, bo.ErrorKindValidation) {
		t.Fatalf("invalid seed error = %v", err)
	}
	if _, err := bo.Seed(context.Background(), bo.SeedRequest{
		Creator: manager, Name: "notes", Operations: options,
	}); err != nil {
		t.Fatalf("first seed error = %v", err)
	}
	if _, err := bo.Seed(context.Background(), bo.SeedRequest{
		Creator: manager, Name: "notes", Operations: options,
	}); !bo.IsKind(err, bo.ErrorKindAlreadyExists) {
		t.Fatalf("duplicate seed error = %v", err)
	}
}

func TestPublicErrorPreservesJoinedCauses(t *testing.T) {
	first := errors.New("publication failed")
	second := errors.New("cleanup failed")
	_, err := bo.Seed(context.Background(), bo.SeedRequest{
		Creator: workspaceCreatorFunc(func(context.Context, string) (string, error) {
			return "", bo.WrapError(bo.ErrorKindConflict, "workspace update failed", errors.Join(first, second))
		}),
		Name:       "notes",
		Operations: bo.OperationOptions{Log: errorLog{}},
	})
	if !bo.IsKind(err, bo.ErrorKindConflict) || !errors.Is(err, first) || !errors.Is(err, second) {
		t.Fatalf("error tree = %v", err)
	}
}

func TestWorkflowPreservesContextErrorIdentity(t *testing.T) {
	for _, test := range []struct {
		name  string
		cause error
		kind  bo.ErrorKind
	}{
		{name: "canceled", cause: context.Canceled, kind: bo.ErrorKindCanceled},
		{name: "deadline", cause: context.DeadlineExceeded, kind: bo.ErrorKindDeadline},
	} {
		t.Run(test.name, func(t *testing.T) {
			workspace := errorWorkspace{storage: errorStorage{err: test.cause}}
			_, err := bo.Snap(context.Background(), bo.SnapRequest{
				Workspace:  workspace,
				Sources:    []string{"https://example.test/source"},
				Operations: bo.OperationOptions{Log: errorLog{}},
			})
			if !errors.Is(err, test.cause) {
				t.Fatalf("error = %v", err)
			}
			var typed *bo.Error
			if !errors.As(err, &typed) || typed.Kind != test.kind {
				t.Fatalf("kind = %v", err)
			}
		})
	}
}

type errorStorage struct{ err error }

type workspaceCreatorFunc func(context.Context, string) (string, error)

func (f workspaceCreatorFunc) Create(ctx context.Context, name string) (string, error) {
	return f(ctx, name)
}

func (s errorStorage) CreateRaw(context.Context, string, []byte) (bo.DocumentRef, error) {
	return bo.DocumentRef{}, nil
}

func (s errorStorage) ReadDocument(context.Context, bo.DocumentRef) ([]byte, error) {
	return nil, s.err
}

func (s errorStorage) ReplaceSummary(context.Context, bo.DocumentRef, []byte) error { return s.err }

func (s errorStorage) DeleteDocument(context.Context, bo.DocumentRef) error { return s.err }

func (s errorStorage) ReadState(context.Context) (bo.State, bo.Generation, error) {
	return bo.State{}, bo.NewGeneration(nil), s.err
}

func (s errorStorage) PublishState(context.Context, bo.State, bo.Generation) (bo.Generation, error) {
	return bo.Generation{}, s.err
}

type errorWorkspace struct{ storage bo.Storage }

func (w errorWorkspace) Name() string        { return "test" }
func (w errorWorkspace) RootPath() string    { return "." }
func (w errorWorkspace) TargetPath() string  { return "." }
func (w errorWorkspace) Storage() bo.Storage { return w.storage }
func (w errorWorkspace) Close() error        { return nil }

type errorLog struct{}

func (errorLog) Append(context.Context, bo.Operation) error { return nil }

func (errorLog) Read(context.Context, string, int, int) (bo.OperationPage, error) {
	return bo.OperationPage{}, nil
}
