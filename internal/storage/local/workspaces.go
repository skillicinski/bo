package local

import (
	"context"
	"path/filepath"

	"github.com/skillicinski/bo/internal/application"
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

func (m *Manager) Open(ctx context.Context, name string) (application.Workspace, error) {
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
	return &Workspace{name: filepath.Base(target), rootPath: filepath.Dir(target), targetPath: target, store: store}, nil
}

type Workspace struct {
	name       string
	rootPath   string
	targetPath string
	store      *Store
}

func (w *Workspace) Name() string                 { return w.name }
func (w *Workspace) RootPath() string             { return w.rootPath }
func (w *Workspace) TargetPath() string           { return w.targetPath }
func (w *Workspace) Storage() application.Storage { return w.store }
func (w *Workspace) Close() error                 { return w.store.Close() }
