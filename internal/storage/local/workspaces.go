package local

import (
	"context"
	"path/filepath"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
)

// Manager resolves named local workspaces for application use cases.
type Manager struct {
	home string
}

func NewManager(home string) *Manager { return &Manager{home: home} }

func (m *Manager) Create(ctx context.Context, name string) (string, error) {
	if err := contextErr(ctx); err != nil {
		return "", err
	}
	var requested *string
	if name != "" {
		requested = &name
	}
	path, err := Seed(m.home, requested)
	if err != nil {
		return "", err
	}
	return filepath.Base(path), nil
}

func (m *Manager) Open(ctx context.Context, name string) (*Workspace, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	target, err := ResolveTarget(m.home, name)
	if err != nil {
		return nil, err
	}
	store, err := Open(target)
	if err != nil {
		return nil, err
	}
	return &Workspace{name: filepath.Base(target), store: store}, nil
}

type Workspace struct {
	name  string
	store *Store
}

func (w *Workspace) Name() string { return w.name }

func (w *Workspace) ListDocuments(ctx context.Context, kind domain.DocumentKind) ([]domain.DocumentRef, error) {
	return w.store.ListDocuments(ctx, kind)
}

func (w *Workspace) ReadDocument(ctx context.Context, ref domain.DocumentRef) ([]byte, error) {
	return w.store.ReadDocument(ctx, ref)
}

func (w *Workspace) ReadState(ctx context.Context) (domain.State, application.Revision, error) {
	return w.store.ReadState(ctx)
}

func (w *Workspace) CommitSnapshot(ctx context.Context, commit application.SnapshotCommit, expected application.Revision) (domain.State, application.Revision, error) {
	return w.store.CommitSnapshot(ctx, commit, expected)
}

func (w *Workspace) CommitSummary(ctx context.Context, commit application.SummaryCommit, expected application.Revision) (domain.State, application.Revision, error) {
	return w.store.CommitSummary(ctx, commit, expected)
}

func (w *Workspace) Close() error { return w.store.Close() }
