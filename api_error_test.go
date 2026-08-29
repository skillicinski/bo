package bo_test

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/skillicinski/bo"
)

func TestStateResultRevisionUsesOpaqueJSONString(t *testing.T) {
	revision := bo.NewRevision([]byte("revision"))
	data, err := json.Marshal(bo.StateResult{Revision: revision})
	if err != nil {
		t.Fatal(err)
	}
	var encoded struct {
		Revision string `json:"revision"`
	}
	if err := json.Unmarshal(data, &encoded); err != nil {
		t.Fatal(err)
	}
	if encoded.Revision != revision.String() {
		t.Fatalf("revision JSON = %q, want %q", encoded.Revision, revision.String())
	}
	var decoded bo.Revision
	if err := json.Unmarshal([]byte(`"`+revision.String()+`"`), &decoded); err != nil {
		t.Fatal(err)
	}
	if !decoded.Equal(revision) {
		t.Fatal("decoded revision differs")
	}
}

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
	options := bo.OperationOptions{}

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
		Operations: bo.OperationOptions{},
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
			workspace := errorWorkspace{err: test.cause}
			_, err := bo.Snap(context.Background(), bo.SnapRequest{
				Workspace:  workspace,
				Sources:    []string{"https://example.test/source"},
				Operations: bo.OperationOptions{},
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

func TestSnapPublicResultPreservesCanceledSource(t *testing.T) {
	path := filepath.Join(t.TempDir(), "source.md")
	if err := os.WriteFile(path, []byte("# Article\n\ncontent\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	workspace := &canceledCommitWorkspace{}
	result, err := bo.Snap(context.Background(), bo.SnapRequest{
		Workspace: workspace,
		Sources:   []string{path},
		SourceConfig: &bo.SnapSourceConfig{
			AllowLocalFiles: true,
		},
	})
	if err == nil || !errors.Is(err, context.Canceled) || !result.Aborted || result.FailedSource != path || len(result.Outcomes) != 0 {
		t.Fatalf("result = %#v, err = %v", result, err)
	}
	var typed *bo.Error
	if !errors.As(err, &typed) || typed.Kind != bo.ErrorKindCanceled {
		t.Fatalf("error kind = %v", err)
	}
	if len(workspace.events) != 1 || workspace.events[0].Outcome != bo.OutcomeFailed || workspace.events[0].Error == nil || workspace.events[0].Error.Kind != string(bo.ErrorKindCanceled) {
		t.Fatalf("events = %#v", workspace.events)
	}
}

type errorWorkspace struct{ err error }

type canceledCommitWorkspace struct {
	errorWorkspace
	events []bo.Operation
}

func (w *canceledCommitWorkspace) ReadState(context.Context) (bo.State, bo.Revision, error) {
	return bo.State{}, bo.NewRevision(nil), nil
}

func (w *canceledCommitWorkspace) CommitEvent(_ context.Context, event bo.Operation) error {
	w.events = append(w.events, event)
	return nil
}

func (w *canceledCommitWorkspace) CommitSnapshot(context.Context, bo.SnapshotCommit, bo.Revision) (bo.State, bo.Revision, error) {
	return bo.State{}, bo.Revision{}, context.Canceled
}

type workspaceCreatorFunc func(context.Context, string) (string, error)

func (f workspaceCreatorFunc) Create(ctx context.Context, name string, _ bo.Operation) (string, error) {
	return f(ctx, name)
}

func (w errorWorkspace) Name() string { return "test" }

func (w errorWorkspace) ListDocuments(context.Context, bo.DocumentKind) ([]bo.DocumentRef, error) {
	return nil, w.err
}

func (w errorWorkspace) ReadDocument(context.Context, bo.DocumentRef) ([]byte, error) {
	return nil, w.err
}

func (w errorWorkspace) ReadState(context.Context) (bo.State, bo.Revision, error) {
	return bo.State{}, bo.NewRevision(nil), w.err
}

func (w errorWorkspace) ReadEvents(context.Context, int, int) (bo.OperationPage, error) {
	return bo.OperationPage{}, w.err
}

func (w errorWorkspace) ReadRecentEvents(context.Context, int) ([]bo.Operation, error) {
	return nil, w.err
}

func (w errorWorkspace) CommitEvent(context.Context, bo.Operation) error {
	return w.err
}

func (w errorWorkspace) CommitSnapshot(context.Context, bo.SnapshotCommit, bo.Revision) (bo.State, bo.Revision, error) {
	return bo.State{}, bo.Revision{}, w.err
}

func (w errorWorkspace) CommitSummary(context.Context, bo.SummaryCommit, bo.Revision) (bo.State, bo.Revision, error) {
	return bo.State{}, bo.Revision{}, w.err
}

func (w errorWorkspace) CommitSynthesized(context.Context, bo.SynthesizedCommit, bo.Revision) (bo.State, bo.Revision, error) {
	return bo.State{}, bo.Revision{}, w.err
}

func (w errorWorkspace) Close() error { return nil }
