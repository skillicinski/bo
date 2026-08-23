package main

import (
	"context"
	"fmt"

	"github.com/skillicinski/bo"
)

type storage struct {
	state      bo.State
	generation bo.Generation
	documents  map[string][]byte
}

func (s *storage) CreateRaw(_ context.Context, name string, contents []byte) (bo.DocumentRef, error) {
	if s.documents == nil {
		s.documents = map[string][]byte{}
	}
	s.documents[name] = append([]byte(nil), contents...)
	return bo.RawRef(name), nil
}

func (s *storage) ReadDocument(_ context.Context, ref bo.DocumentRef) ([]byte, error) {
	contents, ok := s.documents[ref.Name]
	if !ok {
		return nil, fmt.Errorf("document not found: %s", ref.Name)
	}
	return append([]byte(nil), contents...), nil
}

func (s *storage) ReplaceSummary(_ context.Context, ref bo.DocumentRef, contents []byte) error {
	if s.documents == nil {
		s.documents = map[string][]byte{}
	}
	s.documents[ref.Name] = append([]byte(nil), contents...)
	return nil
}

func (s *storage) DeleteDocument(_ context.Context, ref bo.DocumentRef) error {
	delete(s.documents, ref.Name)
	return nil
}

func (s *storage) ReadState(context.Context) (bo.State, bo.Generation, error) {
	return s.state, s.generation, nil
}

func (s *storage) PublishState(_ context.Context, state bo.State, expected bo.Generation) (bo.Generation, error) {
	if !expected.Equal(s.generation) {
		return bo.Generation{}, bo.ConflictError("state generation changed")
	}
	s.state = state
	s.generation = bo.NewGeneration([]byte{byte(len(state.Sources))})
	return s.generation, nil
}

type workspace struct {
	name                 string
	store                bo.Storage
	rootPath, targetPath string
}

func (w workspace) Name() string { return w.name }
func (w workspace) RootPath() string {
	if w.rootPath != "" {
		return w.rootPath
	}
	return "."
}
func (w workspace) TargetPath() string {
	if w.targetPath != "" {
		return w.targetPath
	}
	return "."
}
func (w workspace) Storage() bo.Storage { return w.store }
func (w workspace) Close() error        { return nil }

type creator struct{}

func (creator) Create(context.Context, string) (string, error) { return "consumer", nil }

type operationLog struct{}

func (operationLog) Append(context.Context, bo.Operation) error { return nil }

func (operationLog) Read(_ context.Context, directory string, offset, limit int) (bo.OperationPage, error) {
	return bo.OperationPage{Directory: directory, Offset: offset, Limit: limit, NextOffset: offset}, nil
}
