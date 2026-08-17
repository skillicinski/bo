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

	"github.com/skillicinski/bo"
)

type Store struct {
	root *os.Root
	path string
	mu   sync.Mutex
}

func Open(path string) (*Store, error) {
	canonical, err := filepath.EvalSymlinks(path)
	if err != nil {
		return nil, bo.FilesystemError(fmt.Sprintf("canonicalizing %s failed: %v", path, err))
	}
	info, err := os.Stat(canonical)
	if err != nil {
		return nil, bo.FilesystemError(fmt.Sprintf("reading %s failed: %v", canonical, err))
	}
	if !info.IsDir() {
		return nil, bo.FilesystemError(fmt.Sprintf("target is not a directory: %s", canonical))
	}
	root, err := os.OpenRoot(canonical)
	if err != nil {
		return nil, bo.FilesystemError(fmt.Sprintf("opening %s failed: %v", canonical, err))
	}
	return &Store{root: root, path: canonical}, nil
}

func New(path string) (*Store, error) { return Open(path) }

func (s *Store) Close() error { return s.root.Close() }

func (s *Store) RootPath() string { return s.path }

func (s *Store) InitializeState(ctx context.Context, state bo.State) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, err := s.root.Lstat("state.json"); err == nil {
		return bo.FilesystemError("state file already exists")
	} else if !os.IsNotExist(err) {
		return filesystem("reading state.json", err)
	}
	data, err := bo.MarshalState(state)
	if err != nil {
		return bo.FilesystemError(fmt.Sprintf("serializing state.json failed: %v", err))
	}
	if err := s.writeAtomic("state.json", ".state.json.tmp", data); err != nil {
		return err
	}
	return nil
}

func (s *Store) CreateRaw(ctx context.Context, name string, contents []byte) (bo.DocumentRef, error) {
	if err := contextErr(ctx); err != nil {
		return bo.DocumentRef{}, err
	}
	if err := validMarkdownName(name); err != nil {
		return bo.DocumentRef{}, bo.InputError(err.Error())
	}
	file, err := s.root.OpenFile(name, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		if os.IsExist(err) {
			wrapped := bo.FilesystemError(fmt.Sprintf("creating %s failed: %v", filepath.Join(s.path, name), err))
			wrapped.Cause = bo.ErrAlreadyExists
			return bo.DocumentRef{}, wrapped
		}
		return bo.DocumentRef{}, filesystem(filepath.Join(s.path, name), err)
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
		return bo.DocumentRef{}, filesystem(fmt.Sprintf("writing %s", filepath.Join(s.path, name)), err)
	}
	return bo.RawRef(name), nil
}

func (s *Store) WriteRaw(ctx context.Context, ref bo.DocumentRef, contents []byte) error {
	if ref.Kind != bo.DocumentKindRaw {
		return bo.InputError("raw writes require a raw document")
	}
	created, err := s.CreateRaw(ctx, ref.Name, contents)
	if err != nil {
		return err
	}
	if created.Name != ref.Name {
		return bo.FilesystemError("raw document name changed")
	}
	return nil
}

func (s *Store) DeleteRaw(ctx context.Context, ref bo.DocumentRef) error {
	return s.DeleteDocument(ctx, ref)
}

func (s *Store) ListDocuments(ctx context.Context, kind bo.DocumentKind) ([]bo.DocumentRef, error) {
	return s.ListMarkdownDocuments(ctx, kind)
}

func (s *Store) ReadDocument(ctx context.Context, ref bo.DocumentRef) ([]byte, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	path, err := s.documentPath(ref)
	if err != nil {
		return nil, bo.InputError(err.Error())
	}
	info, err := s.root.Stat(path)
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, path), err)
	}
	if !info.Mode().IsRegular() {
		return nil, bo.FilesystemError(fmt.Sprintf("document is not a regular file: %s", filepath.Join(s.path, path)))
	}
	data, err := s.root.ReadFile(path)
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, path), err)
	}
	return data, nil
}

func (s *Store) ListMarkdownDocuments(ctx context.Context, kind bo.DocumentKind) ([]bo.DocumentRef, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	directory := "."
	if kind == bo.DocumentKindSummary {
		directory = "summaries"
	} else if kind != bo.DocumentKindRaw {
		return nil, bo.InputError("unsupported document kind")
	}
	entries, err := fs.ReadDir(s.root.FS(), directory)
	if err != nil {
		if kind == bo.DocumentKindSummary && errors.Is(err, fs.ErrNotExist) {
			return []bo.DocumentRef{}, nil
		}
		return nil, filesystem(filepath.Join(s.path, directory), err)
	}
	refs := make([]bo.DocumentRef, 0, len(entries))
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
		refs = append(refs, bo.DocumentRef{Kind: kind, Name: name})
	}
	sort.Slice(refs, func(i, j int) bool { return refs[i].Name < refs[j].Name })
	return refs, nil
}

func (s *Store) ReplaceSummary(ctx context.Context, ref bo.DocumentRef, contents []byte) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	if ref.Kind != bo.DocumentKindSummary {
		return bo.InputError("summary writes require a summary document")
	}
	if err := validMarkdownName(ref.Name); err != nil {
		return bo.InputError(err.Error())
	}
	if info, err := s.root.Lstat("summaries"); err == nil {
		if info.Mode()&os.ModeSymlink != 0 {
			return bo.FilesystemError("summaries must not be a symlink")
		}
		if !info.IsDir() {
			return bo.FilesystemError("summaries is not a directory")
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
		return syncDirectory(s.root, "summaries")
	}
}

func (s *Store) DeleteDocument(ctx context.Context, ref bo.DocumentRef) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	if ref.Kind != bo.DocumentKindRaw {
		return bo.InputError("only raw documents can be deleted")
	}
	if err := validMarkdownName(ref.Name); err != nil {
		return bo.InputError(err.Error())
	}
	if err := s.root.Remove(ref.Name); err != nil {
		return filesystem(filepath.Join(s.path, ref.Name), err)
	}
	return nil
}

func (s *Store) ReadState(ctx context.Context) (bo.State, bo.Generation, error) {
	if err := contextErr(ctx); err != nil {
		return bo.State{}, bo.Generation{}, err
	}
	data, err := s.stateBytes()
	if err != nil {
		return bo.State{}, bo.Generation{}, err
	}
	state, err := bo.UnmarshalState(data)
	if err != nil {
		return bo.State{}, bo.Generation{}, bo.FilesystemError(fmt.Sprintf("parsing %s failed: %v", filepath.Join(s.path, "state.json"), err))
	}
	return state, bo.NewGeneration(data), nil
}

func (s *Store) PublishState(ctx context.Context, state bo.State, expected bo.Generation) (bo.Generation, error) {
	if err := contextErr(ctx); err != nil {
		return bo.Generation{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	current, err := s.stateBytes()
	if err != nil {
		return bo.Generation{}, err
	}
	if !bo.NewGeneration(current).Equal(expected) {
		return bo.Generation{}, bo.ConflictError("state generation changed")
	}
	data, err := bo.MarshalState(state)
	if err != nil {
		return bo.Generation{}, bo.FilesystemError(fmt.Sprintf("serializing %s failed: %v", filepath.Join(s.path, "state.json"), err))
	}
	if err := s.writeAtomic("state.json", ".state.json.tmp", data); err != nil {
		return bo.Generation{}, err
	}
	return bo.NewGeneration(data), nil
}

func (s *Store) stateBytes() ([]byte, error) {
	info, err := s.root.Lstat("state.json")
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, "state.json"), err)
	}
	if info.Mode()&os.ModeSymlink != 0 {
		return nil, bo.FilesystemError(fmt.Sprintf("state.json must not be a symlink: %s", filepath.Join(s.path, "state.json")))
	}
	if !info.Mode().IsRegular() {
		return nil, bo.FilesystemError(fmt.Sprintf("state.json is not a regular file: %s", filepath.Join(s.path, "state.json")))
	}
	data, err := s.root.ReadFile("state.json")
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, "state.json"), err)
	}
	return data, nil
}

func (s *Store) documentPath(ref bo.DocumentRef) (string, error) {
	if err := validMarkdownName(ref.Name); err != nil {
		return "", err
	}
	switch ref.Kind {
	case bo.DocumentKindRaw:
		return ref.Name, nil
	case bo.DocumentKindSummary:
		return filepath.Join("summaries", ref.Name), nil
	default:
		return "", fmt.Errorf("unsupported document kind")
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
	if name == "" || name == "." || name == ".." || strings.ContainsAny(name, `/\`) || strings.ContainsRune(name, 0) ||
		!strings.EqualFold(filepath.Ext(name), ".md") {
		return fmt.Errorf("document name must be a Markdown file name")
	}
	return nil
}

func filesystem(path string, err error) *bo.Error {
	return &bo.Error{Category: bo.CategoryFilesystem, Detail: fmt.Sprintf("%s failed: %v", path, err), Cause: err}
}

func contextErr(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
		return nil
	}
}

func syncRoot(root *os.Root) error {
	return syncDirectory(root, ".")
}

func syncDirectory(root *os.Root, path string) error {
	directory, err := root.Open(path)
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
