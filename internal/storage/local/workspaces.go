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

func (m *Manager) Create(ctx context.Context, name string, event application.Operation) (string, error) {
	if err := contextErr(ctx); err != nil {
		return "", err
	}
	var requested *string
	if name != "" {
		requested = &name
	}
	path, err := SeedWithEvent(m.home, requested, event)
	if err != nil {
		return "", err
	}
	return filepath.Base(path), nil
}

func (m *Manager) Open(ctx context.Context, name string) (*Store, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	target, err := ResolveTarget(m.home, name)
	if err != nil {
		return nil, err
	}
	return Open(target)
}
