package main

import (
	"context"
	"strconv"

	"github.com/skillicinski/bo"
)

type storage struct {
	state     bo.State
	revision  bo.Revision
	mutations uint64
	documents map[string][]byte
}

func (s *storage) advanceRevision() bo.Revision {
	s.mutations++
	return bo.NewRevision([]byte(strconv.FormatUint(s.mutations, 10)))
}

func (s *storage) ListDocuments(_ context.Context, kind bo.DocumentKind) ([]bo.DocumentRef, error) {
	refs := []bo.DocumentRef{}
	for _, source := range s.state.Sources {
		for _, snapshot := range source.Snapshots {
			if kind == bo.DocumentKindRaw {
				refs = append(refs, bo.RawRef(snapshot.Filename))
			}
		}
		if kind == bo.DocumentKindSummary && source.Summary != nil {
			refs = append(refs, bo.SummaryRef(source.Summary.Filename))
		}
	}
	return refs, nil
}

func (s *storage) ReadDocument(_ context.Context, ref bo.DocumentRef) ([]byte, error) {
	contents, ok := s.documents[ref.Name]
	if !ok {
		return nil, bo.NewError(bo.ErrorKindMissingResource, "document not found: "+ref.Name)
	}
	return append([]byte(nil), contents...), nil
}

func (s *storage) ReadState(context.Context) (bo.State, bo.Revision, error) {
	return s.state, s.revision, nil
}

func (s *storage) CommitSnapshot(_ context.Context, commit bo.SnapshotCommit, expected bo.Revision) (bo.State, bo.Revision, error) {
	if !expected.Equal(s.revision) {
		return bo.State{}, bo.Revision{}, bo.NewError(bo.ErrorKindConflict, "workspace revision changed")
	}
	if s.documents == nil {
		s.documents = map[string][]byte{}
	}
	if _, exists := s.documents[commit.Filename]; exists {
		return bo.State{}, bo.Revision{}, bo.NewError(bo.ErrorKindAlreadyExists, "document already exists")
	}
	s.documents[commit.Filename] = append([]byte(nil), commit.Contents...)
	for index := range s.state.Sources {
		if s.state.Sources[index].SourceKey == commit.SourceKey {
			s.state.Sources[index].Snapshots = append(s.state.Sources[index].Snapshots, bo.RawRecord{Filename: commit.Filename, WrittenAt: commit.WrittenAt})
			s.revision = s.advanceRevision()
			return s.state, s.revision, nil
		}
	}
	s.state.Sources = append(s.state.Sources, bo.SourceRecord{SourceKey: commit.SourceKey, Snapshots: []bo.RawRecord{{Filename: commit.Filename, WrittenAt: commit.WrittenAt}}})
	s.revision = s.advanceRevision()
	return s.state, s.revision, nil
}

func (s *storage) CommitSummary(_ context.Context, commit bo.SummaryCommit, expected bo.Revision) (bo.State, bo.Revision, error) {
	if !expected.Equal(s.revision) {
		return bo.State{}, bo.Revision{}, bo.NewError(bo.ErrorKindConflict, "workspace revision changed")
	}
	if s.documents == nil {
		s.documents = map[string][]byte{}
	}
	s.documents[commit.Filename] = append([]byte(nil), commit.Contents...)
	for index := range s.state.Sources {
		if s.state.Sources[index].SourceKey == commit.SourceKey {
			if len(s.state.Sources[index].Snapshots) == 0 {
				s.state.Sources[index].Snapshots = []bo.RawRecord{{Filename: commit.DerivedFrom, WrittenAt: commit.RawWrittenAt}}
			}
			s.state.Sources[index].Summary = &bo.SummaryRecord{Filename: commit.Filename, DerivedFrom: commit.DerivedFrom, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt}
			s.revision = s.advanceRevision()
			return s.state, s.revision, nil
		}
	}
	s.state.Sources = append(s.state.Sources, bo.SourceRecord{SourceKey: commit.SourceKey, Snapshots: []bo.RawRecord{{Filename: commit.DerivedFrom, WrittenAt: commit.RawWrittenAt}}, Summary: &bo.SummaryRecord{Filename: commit.Filename, DerivedFrom: commit.DerivedFrom, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt}})
	s.revision = s.advanceRevision()
	return s.state, s.revision, nil
}

type workspace struct {
	name  string
	store *storage
}

func (w workspace) Name() string { return w.name }
func (w workspace) ListDocuments(ctx context.Context, kind bo.DocumentKind) ([]bo.DocumentRef, error) {
	return w.store.ListDocuments(ctx, kind)
}
func (w workspace) ReadDocument(ctx context.Context, ref bo.DocumentRef) ([]byte, error) {
	return w.store.ReadDocument(ctx, ref)
}
func (w workspace) ReadState(ctx context.Context) (bo.State, bo.Revision, error) {
	return w.store.ReadState(ctx)
}
func (w workspace) CommitSnapshot(ctx context.Context, commit bo.SnapshotCommit, expected bo.Revision) (bo.State, bo.Revision, error) {
	return w.store.CommitSnapshot(ctx, commit, expected)
}
func (w workspace) CommitSummary(ctx context.Context, commit bo.SummaryCommit, expected bo.Revision) (bo.State, bo.Revision, error) {
	return w.store.CommitSummary(ctx, commit, expected)
}
func (w workspace) Close() error { return nil }

type creator struct{}

func (creator) Create(context.Context, string) (string, error) { return "consumer", nil }

type operationLog struct{}

func (operationLog) Append(context.Context, bo.Operation) error { return nil }

func (operationLog) Read(_ context.Context, directory string, offset, limit int) (bo.OperationPage, error) {
	return bo.OperationPage{Directory: directory, Offset: offset, Limit: limit, NextOffset: offset}, nil
}
