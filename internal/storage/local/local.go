// Package local implements the local filesystem storage adapter.
package local

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type Store struct {
	root *os.Root
	path string
	mu   sync.Mutex
}

func Open(path string) (*Store, error) {
	canonical, err := filepath.EvalSymlinks(path)
	if err != nil {
		return nil, filesystem(path, err)
	}
	info, err := os.Stat(canonical)
	if err != nil {
		return nil, filesystem(canonical, err)
	}
	if !info.IsDir() {
		return nil, internalerrors.Validation(fmt.Sprintf("target is not a directory: %s", canonical))
	}
	root, err := os.OpenRoot(canonical)
	if err != nil {
		return nil, filesystem(canonical, err)
	}
	return &Store{root: root, path: canonical}, nil
}

func New(path string) (*Store, error) { return Open(path) }

func (s *Store) Close() error { return s.root.Close() }

func (s *Store) RootPath() string { return s.path }

func (s *Store) InitializeState(ctx context.Context, state domain.State) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, err := s.root.Lstat("state.json"); err == nil {
		return internalerrors.AlreadyExists("state file already exists")
	} else if !os.IsNotExist(err) {
		return filesystem("reading state.json", err)
	}
	data, err := domain.MarshalState(state)
	if err != nil {
		return normalizeStorageError("serializing state.json", err)
	}
	if err := s.writeAtomic("state.json", ".state.json.tmp", data); err != nil {
		return err
	}
	return nil
}

func (s *Store) CreateRaw(ctx context.Context, name string, contents []byte) (domain.DocumentRef, error) {
	if err := contextErr(ctx); err != nil {
		return domain.DocumentRef{}, err
	}
	if err := validMarkdownName(name); err != nil {
		return domain.DocumentRef{}, err
	}
	file, err := s.root.OpenFile(name, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		if os.IsExist(err) {
			wrapped := internalerrors.Wrap(internalerrors.KindAlreadyExists, fmt.Sprintf("creating %s failed", filepath.Join(s.path, name)), internalerrors.ErrAlreadyExists)
			return domain.DocumentRef{}, wrapped
		}
		return domain.DocumentRef{}, filesystem(filepath.Join(s.path, name), err)
	}
	if _, err = file.Write(contents); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		_ = s.root.Remove(name)
		return domain.DocumentRef{}, filesystem(fmt.Sprintf("writing %s", filepath.Join(s.path, name)), err)
	}
	return domain.RawRef(name), nil
}

func (s *Store) WriteRaw(ctx context.Context, ref domain.DocumentRef, contents []byte) error {
	if ref.Kind != domain.DocumentKindRaw {
		return internalerrors.Validation("raw writes require a raw document")
	}
	created, err := s.CreateRaw(ctx, ref.Name, contents)
	if err != nil {
		return err
	}
	if created.Name != ref.Name {
		return internalerrors.Filesystem("raw document name changed")
	}
	return nil
}

func (s *Store) DeleteRaw(ctx context.Context, ref domain.DocumentRef) error {
	return s.DeleteDocument(ctx, ref)
}

func (s *Store) ListDocuments(ctx context.Context, kind domain.DocumentKind) ([]domain.DocumentRef, error) {
	return s.ListMarkdownDocuments(ctx, kind)
}

func (s *Store) ReadDocument(ctx context.Context, ref domain.DocumentRef) ([]byte, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	path, err := s.documentPath(ref)
	if err != nil {
		return nil, err
	}
	info, err := s.root.Stat(path)
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, path), err)
	}
	if !info.Mode().IsRegular() {
		return nil, internalerrors.Filesystem(fmt.Sprintf("document is not a regular file: %s", filepath.Join(s.path, path)))
	}
	data, err := s.root.ReadFile(path)
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, path), err)
	}
	return data, nil
}

func (s *Store) ListMarkdownDocuments(ctx context.Context, kind domain.DocumentKind) ([]domain.DocumentRef, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	directory := "."
	if kind == domain.DocumentKindSummary {
		directory = "summaries"
	} else if kind != domain.DocumentKindRaw {
		return nil, internalerrors.Validation("unsupported document kind")
	}
	entries, err := fs.ReadDir(s.root.FS(), directory)
	if err != nil {
		if kind == domain.DocumentKindSummary && errors.Is(err, fs.ErrNotExist) {
			return []domain.DocumentRef{}, nil
		}
		return nil, filesystem(filepath.Join(s.path, directory), err)
	}
	refs := make([]domain.DocumentRef, 0, len(entries))
	for _, entry := range entries {
		name := entry.Name()
		if !strings.EqualFold(filepath.Ext(name), ".md") {
			continue
		}
		path := filepath.Join(directory, name)
		info, err := s.root.Stat(path)
		if err != nil {
			return nil, filesystem(filepath.Join(s.path, path), err)
		}
		if !info.Mode().IsRegular() {
			continue
		}
		refs = append(refs, domain.DocumentRef{Kind: kind, Name: name})
	}
	sort.Slice(refs, func(i, j int) bool { return refs[i].Name < refs[j].Name })
	return refs, nil
}

func (s *Store) ReplaceSummary(ctx context.Context, ref domain.DocumentRef, contents []byte) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	if ref.Kind != domain.DocumentKindSummary {
		return internalerrors.Validation("summary writes require a summary document")
	}
	if err := validMarkdownName(ref.Name); err != nil {
		return err
	}
	if info, err := s.root.Lstat("summaries"); err == nil {
		if info.Mode()&os.ModeSymlink != 0 {
			return internalerrors.Filesystem("summaries must not be a symlink")
		}
		if !info.IsDir() {
			return internalerrors.Filesystem("summaries is not a directory")
		}
	} else if os.IsNotExist(err) {
		if err := s.root.Mkdir("summaries", 0o755); err != nil {
			return filesystem(filepath.Join(s.path, "summaries"), err)
		}
	} else {
		return filesystem(filepath.Join(s.path, "summaries"), err)
	}
	temporary := fmt.Sprintf("summaries/.bo-summary-tmp-%d-%d", os.Getpid(), time.Now().UnixNano())
	for attempt := 0; ; attempt++ {
		if attempt > 0 {
			temporary = fmt.Sprintf("summaries/.bo-summary-tmp-%d-%d-%d", os.Getpid(), time.Now().UnixNano(), attempt)
		}
		file, err := s.root.OpenFile(temporary, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if err != nil {
			if os.IsExist(err) {
				continue
			}
			return filesystem(filepath.Join(s.path, temporary), err)
		}
		if _, err = file.Write(contents); err == nil {
			err = file.Sync()
		}
		closeErr := file.Close()
		if err == nil {
			err = closeErr
		}
		if err == nil {
			err = s.root.Rename(temporary, filepath.Join("summaries", ref.Name))
		}
		if err != nil {
			_ = s.root.Remove(temporary)
			return filesystem(filepath.Join(s.path, "summaries", ref.Name), err)
		}
		return syncFilesystem(filepath.Join(s.path, "summaries"), syncDirectory(s.root, "summaries"))
	}
}

func (s *Store) DeleteDocument(ctx context.Context, ref domain.DocumentRef) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	if ref.Kind != domain.DocumentKindRaw {
		return internalerrors.Validation("only raw documents can be deleted")
	}
	if err := validMarkdownName(ref.Name); err != nil {
		return err
	}
	if err := s.root.Remove(ref.Name); err != nil {
		return filesystem(filepath.Join(s.path, ref.Name), err)
	}
	return nil
}

func (s *Store) ReadState(ctx context.Context) (domain.State, application.Generation, error) {
	if err := contextErr(ctx); err != nil {
		return domain.State{}, application.Generation{}, err
	}
	data, err := s.stateBytes()
	if err != nil {
		return domain.State{}, application.Generation{}, err
	}
	state, err := domain.UnmarshalState(data)
	if err != nil {
		return domain.State{}, application.Generation{}, normalizeStorageError("parsing "+filepath.Join(s.path, "state.json"), err)
	}
	return state, application.NewGeneration(data), nil
}

func (s *Store) PublishState(ctx context.Context, state domain.State, expected application.Generation) (application.Generation, error) {
	if err := contextErr(ctx); err != nil {
		return application.Generation{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	current, err := s.stateBytes()
	if err != nil {
		return application.Generation{}, err
	}
	if !application.NewGeneration(current).Equal(expected) {
		return application.Generation{}, internalerrors.Conflict("state generation changed")
	}
	data, err := domain.MarshalState(state)
	if err != nil {
		return application.Generation{}, normalizeStorageError("serializing "+filepath.Join(s.path, "state.json"), err)
	}
	if err := s.writeAtomic("state.json", ".state.json.tmp", data); err != nil {
		return application.Generation{}, err
	}
	return application.NewGeneration(data), nil
}

func (s *Store) stateBytes() ([]byte, error) {
	info, err := s.root.Lstat("state.json")
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, "state.json"), err)
	}
	if info.Mode()&os.ModeSymlink != 0 {
		return nil, internalerrors.Filesystem(fmt.Sprintf("state.json must not be a symlink: %s", filepath.Join(s.path, "state.json")))
	}
	if !info.Mode().IsRegular() {
		return nil, internalerrors.Filesystem(fmt.Sprintf("state.json is not a regular file: %s", filepath.Join(s.path, "state.json")))
	}
	data, err := s.root.ReadFile("state.json")
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, "state.json"), err)
	}
	return data, nil
}

func (s *Store) documentPath(ref domain.DocumentRef) (string, error) {
	if err := validMarkdownName(ref.Name); err != nil {
		return "", err
	}
	switch ref.Kind {
	case domain.DocumentKindRaw:
		return ref.Name, nil
	case domain.DocumentKindSummary:
		return filepath.Join("summaries", ref.Name), nil
	default:
		return "", internalerrors.Validation("unsupported document kind")
	}
}

func (s *Store) writeAtomic(destination, temporary string, data []byte) error {
	flags := os.O_WRONLY | os.O_CREATE | os.O_EXCL
	file, err := s.root.OpenFile(temporary, flags, 0o600)
	if err != nil {
		return filesystem(filepath.Join(s.path, temporary), err)
	}
	if _, err = file.Write(data); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err == nil {
		err = s.root.Rename(temporary, destination)
	}
	if err != nil {
		_ = s.root.Remove(temporary)
		return filesystem(filepath.Join(s.path, destination), err)
	}
	return syncRoot(s.root)
}

func validMarkdownName(name string) error {
	return domain.ValidateDocumentName(name)
}

func filesystem(path string, err error) error {
	kind := internalerrors.KindFilesystem
	if errors.Is(err, os.ErrNotExist) {
		kind = internalerrors.KindMissingResource
	}
	return internalerrors.Wrap(kind, fmt.Sprintf("%s failed", path), err)
}

func normalizeStorageError(operation string, err error) error {
	if err == nil {
		return nil
	}
	var categorized *internalerrors.Error
	if errors.As(err, &categorized) {
		return err
	}
	return internalerrors.Wrap(internalerrors.KindFilesystem, operation+" failed", err)
}

func syncFilesystem(path string, err error) error {
	if err == nil {
		return nil
	}
	return filesystem(path, err)
}

func contextErr(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return internalerrors.Context(ctx.Err())
	default:
		return nil
	}
}

func syncRoot(root *os.Root) error {
	return syncFilesystem("workspace root", syncDirectory(root, "."))
}

func syncDirectory(root *os.Root, path string) error {
	directory, err := root.Open(path)
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
