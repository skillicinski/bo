// Package local implements the local filesystem workspace adapter.
package local

import (
	"bufio"
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
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
	mu   *sync.Mutex
}

var (
	workspaceLocksMu sync.Mutex
	workspaceLocks   = map[string]*sync.Mutex{}
)

const (
	workspaceTransactionFile             = ".bo-transaction.json"
	workspaceEventFile                   = "log.jsonl"
	defaultOperationPageLimit            = 20
	maxOperationEventBytes               = 1 << 20
	workspaceTransactionVersion          = 1
	workspaceTransactionPhaseReady       = "prepared"
	workspaceTransactionPhaseCommit      = "commit"
	workspaceTransactionKindSnapshot     = "snapshot"
	workspaceTransactionKindSummary      = "summary"
	workspaceTransactionKindDistillation = "distillation"
)

type workspaceTransaction struct {
	Version              int    `json:"version"`
	Phase                string `json:"phase"`
	Kind                 string `json:"kind"`
	DocumentName         string `json:"document_name"`
	DocumentTemporary    string `json:"document_temporary"`
	StateTemporary       string `json:"state_temporary"`
	TransactionTemporary string `json:"transaction_temporary"`
	HadOldDocument       bool   `json:"had_old_document"`
	OldDocument          []byte `json:"old_document,omitempty"`
	NewDocument          []byte `json:"new_document"`
	OldState             []byte `json:"old_state"`
	NewState             []byte `json:"new_state"`
	EventsTracked        bool   `json:"events_tracked,omitempty"`
	EventLine            []byte `json:"event_line,omitempty"`
	OldEventsPresent     bool   `json:"old_events_present,omitempty"`
	NewEventsPresent     bool   `json:"new_events_present,omitempty"`
	OldEventsSize        int64  `json:"old_events_size,omitempty"`
	NewEventsSize        int64  `json:"new_events_size,omitempty"`
}

type ledgerSnapshot struct {
	present bool
	size    int64
}

var (
	stopLedgerScan                  = errors.New("stop workspace event scan")
	errWorkspaceEventLineTooLarge   = errors.New("workspace event line is too large")
	errWorkspaceEventLineIncomplete = errors.New("workspace event ledger has an incomplete line")
)

// ponytail: one process-wide lock per workspace; use an inter-process lock if local storage becomes multi-process.
func lockForWorkspace(path string) *sync.Mutex {
	workspaceLocksMu.Lock()
	defer workspaceLocksMu.Unlock()
	if lock := workspaceLocks[path]; lock != nil {
		return lock
	}
	lock := &sync.Mutex{}
	workspaceLocks[path] = lock
	return lock
}

func Open(path string) (*Store, error) {
	absolute, err := filepath.Abs(path)
	if err != nil {
		return nil, filesystem(path, err)
	}
	canonical, err := filepath.EvalSymlinks(absolute)
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
	store := &Store{root: root, path: canonical, mu: lockForWorkspace(canonical)}
	store.mu.Lock()
	recoveryErr := store.recoverWorkspaceTransaction()
	store.mu.Unlock()
	if recoveryErr != nil {
		_ = root.Close()
		return nil, recoveryErr
	}
	return store, nil
}

func (s *Store) Close() error { return s.root.Close() }

func (s *Store) Name() string { return filepath.Base(s.path) }

func (s *Store) ListDocuments(ctx context.Context, kind domain.DocumentKind) ([]domain.DocumentRef, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.recoverWorkspaceTransaction(); err != nil {
		return nil, err
	}
	return s.listDocuments(kind)
}

func (s *Store) listDocuments(kind domain.DocumentKind) ([]domain.DocumentRef, error) {
	directory := "."
	switch kind {
	case domain.DocumentKindRaw:
	case domain.DocumentKindSummary, domain.DocumentKindDistillation:
		if kind == domain.DocumentKindSummary {
			directory = "summaries"
		} else {
			directory = "distillations"
		}
		if info, err := s.root.Lstat(directory); err != nil {
			if os.IsNotExist(err) {
				return []domain.DocumentRef{}, nil
			}
			return nil, filesystem(filepath.Join(s.path, directory), err)
		} else if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
			return nil, internalerrors.Filesystem(fmt.Sprintf("%s must be a directory: %s", directory, filepath.Join(s.path, directory)))
		}
	default:
		return nil, internalerrors.Validation("unsupported document kind")
	}
	entries, err := fs.ReadDir(s.root.FS(), directory)
	if err != nil {
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
		if info.Mode().IsRegular() {
			refs = append(refs, domain.DocumentRef{Kind: kind, Name: name})
		}
	}
	sort.Slice(refs, func(i, j int) bool { return refs[i].Name < refs[j].Name })
	return refs, nil
}

func (s *Store) ReadDocument(ctx context.Context, ref domain.DocumentRef) ([]byte, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.recoverWorkspaceTransaction(); err != nil {
		return nil, err
	}
	return s.readDocument(ref)
}

func (s *Store) readDocument(ref domain.DocumentRef) ([]byte, error) {
	path, _, err := s.documentInfo(ref)
	if err != nil {
		return nil, err
	}
	data, err := s.root.ReadFile(path)
	if err != nil {
		return nil, filesystem(filepath.Join(s.path, path), err)
	}
	return data, nil
}

func (s *Store) ReadState(ctx context.Context) (domain.State, application.Revision, error) {
	if err := contextErr(ctx); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	state, _, revision, err := s.readState()
	return state, revision, err
}

func (s *Store) ReadEvents(ctx context.Context, offset, limit int) (application.OperationPage, error) {
	if err := contextErr(ctx); err != nil {
		return application.OperationPage{}, err
	}
	if offset < 0 {
		return application.OperationPage{}, internalerrors.Validation("operation event offset must not be negative")
	}
	if limit <= 0 {
		limit = defaultOperationPageLimit
	}
	if limit > application.MaxOperationPageLimit {
		limit = application.MaxOperationPageLimit
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.recoverWorkspaceTransaction(); err != nil {
		return application.OperationPage{}, err
	}
	entries := make([]domain.Operation, 0, limit)
	hasMore := false
	err := s.scanWorkspaceEvents(ctx, func(index int, event domain.Operation) error {
		if index >= offset {
			if len(entries) < limit {
				entries = append(entries, event)
			} else {
				hasMore = true
				return stopLedgerScan
			}
		}
		return nil
	})
	if err != nil {
		return application.OperationPage{}, err
	}
	return application.OperationPage{
		Directory:  s.Name(),
		Entries:    entries,
		Offset:     offset,
		Limit:      limit,
		NextOffset: offset + len(entries),
		HasMore:    hasMore,
	}, nil
}

func (s *Store) ReadRecentEvents(ctx context.Context, limit int) ([]application.Operation, error) {
	if err := contextErr(ctx); err != nil {
		return nil, err
	}
	if limit <= 0 {
		limit = defaultOperationPageLimit
	}
	if limit > application.MaxOperationPageLimit {
		limit = application.MaxOperationPageLimit
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.recoverWorkspaceTransaction(); err != nil {
		return nil, err
	}
	ring := make([]application.Operation, limit)
	count, next := 0, 0
	err := s.scanWorkspaceEvents(ctx, func(_ int, event domain.Operation) error {
		ring[next] = event
		next = (next + 1) % limit
		if count < limit {
			count++
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	entries := make([]application.Operation, count)
	start := 0
	if count == limit {
		start = next
	}
	for index := range entries {
		entries[index] = ring[(start+index)%limit]
	}
	return entries, nil
}

func (s *Store) CommitEvent(ctx context.Context, event application.Operation) error {
	if err := contextErr(ctx); err != nil {
		return err
	}
	event.Normalize()
	if err := event.Validate(); err != nil {
		return internalerrors.Wrap(internalerrors.KindValidation, "invalid operation event", err)
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.recoverWorkspaceTransaction(); err != nil {
		return err
	}
	line, err := marshalEventLine(event)
	if err != nil {
		return err
	}
	before, err := s.ledgerMetadata()
	if err != nil {
		return err
	}
	if err := s.appendLedgerLine(line); err != nil {
		return errors.Join(err, s.restoreLedger(before))
	}
	return nil
}

func (s *Store) CommitSnapshot(ctx context.Context, commit application.SnapshotCommit, expected application.Revision) (domain.State, application.Revision, error) {
	if err := contextErr(ctx); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := commit.Validate(); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	current, oldState, currentRevision, err := s.readState()
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if !currentRevision.Equal(expected) {
		return domain.State{}, application.Revision{}, internalerrors.Conflict("workspace revision changed")
	}
	if _, err := s.root.Lstat(commit.Filename); err == nil {
		return domain.State{}, application.Revision{}, internalerrors.Wrap(internalerrors.KindAlreadyExists, "raw document already exists", internalerrors.ErrAlreadyExists)
	} else if !os.IsNotExist(err) {
		return domain.State{}, application.Revision{}, filesystem(filepath.Join(s.path, commit.Filename), err)
	}
	next, err := current.ApplySnapshot(commit)
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	data, err := domain.MarshalState(next)
	if err != nil {
		return domain.State{}, application.Revision{}, normalizeStorageError("serializing state.json", err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSnapshot, commit.Filename, nil, false, commit.Contents, oldState, data)
	if err := s.trackTransactionEvent(&transaction, commit.Event); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.beginWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.writeNewRaw(commit.Filename, transaction.DocumentTemporary, commit.Contents); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.ensureDocumentBaseline(&next, domain.RawRef(commit.Filename), commit.Contents); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	data, err = domain.MarshalState(next)
	if err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, normalizeStorageError("serializing state.json", err))
	}
	transaction.NewState = data
	if err := s.writeWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.writeAtomic("state.json", transaction.StateTemporary, data); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.publishTransactionEvent(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	revision, err := s.workspaceRevision(data)
	if err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.publishWorkspaceCommit(transaction); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.removeWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	return next, revision, nil
}

func (s *Store) CommitSummary(ctx context.Context, commit application.SummaryCommit, expected application.Revision) (domain.State, application.Revision, error) {
	if err := contextErr(ctx); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := commit.Validate(); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	current, oldState, currentRevision, err := s.readState()
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if !currentRevision.Equal(expected) {
		return domain.State{}, application.Revision{}, internalerrors.Conflict("workspace revision changed")
	}
	rawRef := domain.RawRef(commit.DerivedFrom)
	rawContents, err := s.readDocument(rawRef)
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if snapshotRecord(current, commit.SourceKey, commit.DerivedFrom) != nil {
		if err := s.ensureDocumentBaseline(&current, rawRef, rawContents); err != nil {
			return domain.State{}, application.Revision{}, err
		}
	}
	if err := s.ensureSummaryDirectory(); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	path := filepath.Join("summaries", commit.Filename)
	oldContents, hadOldContents, err := s.optionalDocument(domain.SummaryRef(commit.Filename))
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	summary := summaryRecord(current, commit.SourceKey)
	if summary != nil {
		if !hadOldContents {
			return domain.State{}, application.Revision{}, internalerrors.MissingResource("referenced summary is missing")
		}
		if err := s.ensureDocumentBaseline(&current, domain.SummaryRef(commit.Filename), oldContents); err != nil {
			return domain.State{}, application.Revision{}, err
		}
	}
	if summary == nil && hadOldContents {
		return domain.State{}, application.Revision{}, internalerrors.Conflict("summary exists outside workspace state")
	}
	next, err := current.ApplySummary(commit)
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.ensureDocumentBaseline(&next, rawRef, rawContents); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	data, err := domain.MarshalState(next)
	if err != nil {
		return domain.State{}, application.Revision{}, normalizeStorageError("serializing state.json", err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindSummary, commit.Filename, oldContents, hadOldContents, commit.Contents, oldState, data)
	if err := s.trackTransactionEvent(&transaction, commit.Event); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.beginWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.writeAtomic(path, transaction.DocumentTemporary, commit.Contents); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.ensureDocumentBaseline(&next, domain.SummaryRef(commit.Filename), commit.Contents); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	data, err = domain.MarshalState(next)
	if err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, normalizeStorageError("serializing state.json", err))
	}
	transaction.NewState = data
	if err := s.writeWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.writeAtomic("state.json", transaction.StateTemporary, data); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.publishTransactionEvent(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	revision, err := s.workspaceRevision(data)
	if err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.publishWorkspaceCommit(transaction); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.removeWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	return next, revision, nil
}

func (s *Store) CommitDistillation(ctx context.Context, commit application.DistillationCommit, expected application.Revision) (domain.State, application.Revision, error) {
	if err := contextErr(ctx); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := commit.Validate(); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	current, oldState, currentRevision, err := s.readState()
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if !currentRevision.Equal(expected) {
		return domain.State{}, application.Revision{}, internalerrors.Conflict("workspace revision changed")
	}
	if err := s.ensureDistillationDirectory(); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	path := filepath.Join("distillations", commit.Filename)
	oldContents, hadOldContents, err := s.optionalDocument(domain.DistillationRef(commit.Filename))
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if !commit.Update && hadOldContents {
		return domain.State{}, application.Revision{}, internalerrors.Wrap(internalerrors.KindAlreadyExists, "distillation document already exists", internalerrors.ErrAlreadyExists)
	}
	if commit.Update && !hadOldContents {
		return domain.State{}, application.Revision{}, internalerrors.MissingResource("distillation document is missing")
	}
	if commit.Update {
		if err := s.ensureDocumentBaseline(&current, domain.DistillationRef(commit.Filename), oldContents); err != nil {
			return domain.State{}, application.Revision{}, err
		}
	}
	next, err := s.applyDistillation(current, commit)
	if err != nil {
		return domain.State{}, application.Revision{}, err
	}
	data, err := domain.MarshalState(next)
	if err != nil {
		return domain.State{}, application.Revision{}, normalizeStorageError("serializing state.json", err)
	}
	transaction := newWorkspaceTransaction(workspaceTransactionKindDistillation, commit.Filename, oldContents, hadOldContents, commit.Contents, oldState, data)
	if err := s.trackTransactionEvent(&transaction, commit.Event); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.beginWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.writeAtomic(path, transaction.DocumentTemporary, commit.Contents); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.ensureDocumentBaseline(&next, domain.DistillationRef(commit.Filename), commit.Contents); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	data, err = domain.MarshalState(next)
	if err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, normalizeStorageError("serializing state.json", err))
	}
	transaction.NewState = data
	if err := s.writeWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.writeAtomic("state.json", transaction.StateTemporary, data); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.publishTransactionEvent(transaction); err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	revision, err := s.workspaceRevision(data)
	if err != nil {
		return domain.State{}, application.Revision{}, s.abortWorkspaceTransaction(transaction, err)
	}
	if err := s.publishWorkspaceCommit(transaction); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	if err := s.removeWorkspaceTransaction(transaction); err != nil {
		return domain.State{}, application.Revision{}, err
	}
	return next, revision, nil
}

func (s *Store) scanWorkspaceEvents(ctx context.Context, visit func(int, domain.Operation) error) error {
	info, err := s.root.Lstat(workspaceEventFile)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return internalerrors.Filesystem(fmt.Sprintf("%s must be a regular file: %s", workspaceEventFile, filepath.Join(s.path, workspaceEventFile)))
	}
	file, err := s.root.Open(workspaceEventFile)
	if err != nil {
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	defer file.Close()
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 64*1024), maxOperationEventBytes+1)
	scanner.Split(splitWorkspaceEventLine)
	index := 0
	for scanner.Scan() {
		if err := contextErr(ctx); err != nil {
			return err
		}
		line := scanner.Bytes()
		event, err := decodeEventLine(line)
		if err != nil {
			return normalizeStorageError("parsing workspace event ledger", err)
		}
		if visit != nil {
			if err := visit(index, event); err != nil {
				if errors.Is(err, stopLedgerScan) {
					return nil
				}
				return err
			}
		}
		index++
	}
	if err := scanner.Err(); err != nil {
		if errors.Is(err, errWorkspaceEventLineTooLarge) {
			return internalerrors.Validation(err.Error())
		}
		if errors.Is(err, errWorkspaceEventLineIncomplete) {
			return internalerrors.Validation(err.Error())
		}
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	return nil
}

func splitWorkspaceEventLine(data []byte, atEOF bool) (advance int, token []byte, err error) {
	if index := bytes.IndexByte(data, '\n'); index >= 0 {
		if index+1 > maxOperationEventBytes {
			return 0, nil, errWorkspaceEventLineTooLarge
		}
		return index + 1, data[:index], nil
	}
	if atEOF {
		if len(data) == 0 {
			return 0, nil, nil
		}
		return 0, nil, errWorkspaceEventLineIncomplete
	}
	if len(data) > maxOperationEventBytes {
		return 0, nil, errWorkspaceEventLineTooLarge
	}
	return 0, nil, nil
}

func (s *Store) ledgerMetadata() (ledgerSnapshot, error) {
	info, err := s.root.Lstat(workspaceEventFile)
	if err != nil {
		if os.IsNotExist(err) {
			return ledgerSnapshot{}, nil
		}
		return ledgerSnapshot{}, filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return ledgerSnapshot{}, internalerrors.Filesystem(fmt.Sprintf("%s must be a regular file: %s", workspaceEventFile, filepath.Join(s.path, workspaceEventFile)))
	}
	return ledgerSnapshot{present: true, size: info.Size()}, nil
}

func decodeEventLine(line []byte) (domain.Operation, error) {
	line = bytes.TrimSuffix(line, []byte{'\n'})
	decoder := json.NewDecoder(bytes.NewReader(line))
	decoder.DisallowUnknownFields()
	var event domain.Operation
	if err := decoder.Decode(&event); err != nil {
		return domain.Operation{}, err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		if err == nil {
			return domain.Operation{}, errors.New("workspace event line contains multiple JSON values")
		}
		return domain.Operation{}, err
	}
	if err := event.Validate(); err != nil {
		return domain.Operation{}, err
	}
	return event, nil
}

func marshalEventLine(event domain.Operation) ([]byte, error) {
	if err := event.Validate(); err != nil {
		return nil, internalerrors.Wrap(internalerrors.KindValidation, "invalid operation event", err)
	}
	data, err := json.Marshal(event)
	if err != nil {
		return nil, normalizeStorageError("serializing workspace event", err)
	}
	line := append(data, '\n')
	if len(line) > maxOperationEventBytes {
		return nil, internalerrors.Validation("workspace event line is too large")
	}
	return line, nil
}

func (s *Store) appendLedgerLine(line []byte) error {
	if len(line) > maxOperationEventBytes {
		return internalerrors.Validation("workspace event line is too large")
	}
	file, err := s.root.OpenFile(workspaceEventFile, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o600)
	if err != nil {
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	if err := file.Chmod(0o600); err != nil {
		_ = file.Close()
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	written, writeErr := file.Write(line)
	if writeErr == nil && written != len(line) {
		writeErr = io.ErrShortWrite
	}
	if writeErr == nil {
		writeErr = file.Sync()
	}
	closeErr := file.Close()
	if writeErr == nil {
		writeErr = closeErr
	}
	if writeErr != nil {
		return filesystem(filepath.Join(s.path, workspaceEventFile), writeErr)
	}
	if err := syncRoot(s.root); err != nil {
		return err
	}
	return nil
}

func (s *Store) trackTransactionEvent(transaction *workspaceTransaction, event domain.Operation) error {
	if err := event.Validate(); err != nil {
		return internalerrors.Wrap(internalerrors.KindValidation, "invalid mutation event", err)
	}
	old, err := s.ledgerMetadata()
	if err != nil {
		return err
	}
	line, err := marshalEventLine(event)
	if err != nil {
		return err
	}
	newState := ledgerSnapshot{present: true, size: old.size + int64(len(line))}
	transaction.EventsTracked = true
	transaction.EventLine = append([]byte(nil), line...)
	transaction.OldEventsPresent = old.present
	transaction.NewEventsPresent = newState.present
	transaction.OldEventsSize = old.size
	transaction.NewEventsSize = newState.size
	return nil
}

func transactionLedgerState(transaction workspaceTransaction, old bool) ledgerSnapshot {
	if old {
		return ledgerSnapshot{present: transaction.OldEventsPresent, size: transaction.OldEventsSize}
	}
	return ledgerSnapshot{present: transaction.NewEventsPresent, size: transaction.NewEventsSize}
}

func equalLedgerSnapshot(left, right ledgerSnapshot) bool {
	return left.present == right.present && left.size == right.size
}

func (s *Store) restoreLedger(snapshot ledgerSnapshot) error {
	if !snapshot.present {
		if err := s.root.Remove(workspaceEventFile); err != nil && !os.IsNotExist(err) {
			return filesystem(filepath.Join(s.path, workspaceEventFile), err)
		}
		if err := syncRoot(s.root); err != nil {
			return err
		}
		return nil
	}
	file, err := s.root.OpenFile(workspaceEventFile, os.O_WRONLY, 0o600)
	if err != nil {
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	err = file.Truncate(snapshot.size)
	if err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		return filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	if err := syncRoot(s.root); err != nil {
		return err
	}
	return nil
}

func (s *Store) publishTransactionEvent(transaction workspaceTransaction) error {
	if !transaction.EventsTracked {
		return nil
	}
	old := transactionLedgerState(transaction, true)
	newState := transactionLedgerState(transaction, false)
	current, err := s.ledgerMetadata()
	if err != nil {
		return err
	}
	if current.present && current.size >= newState.size {
		if err := s.verifyTransactionEvent(transaction); err == nil {
			return nil
		}
	}
	if !equalLedgerSnapshot(current, old) {
		return internalerrors.Conflict("workspace event ledger changed during transaction")
	}
	if len(transaction.EventLine) != 0 {
		if err := s.appendLedgerLine(transaction.EventLine); err != nil {
			return err
		}
	}
	current, err = s.ledgerMetadata()
	if err != nil {
		return err
	}
	if !equalLedgerSnapshot(current, newState) {
		return internalerrors.Conflict("workspace event ledger publication did not match transaction")
	}
	return s.verifyTransactionEvent(transaction)
}

func (s *Store) rollbackTransactionEvent(transaction workspaceTransaction) error {
	if !transaction.EventsTracked {
		return nil
	}
	old := transactionLedgerState(transaction, true)
	newState := transactionLedgerState(transaction, false)
	current, err := s.ledgerMetadata()
	if err != nil {
		return err
	}
	if equalLedgerSnapshot(current, old) {
		return nil
	}
	if !equalLedgerSnapshot(current, newState) {
		if current.present && current.size >= old.size && current.size < newState.size {
			prefixLength := current.size - old.size
			matches, err := s.ledgerBytesMatchAt(old.size, transaction.EventLine[:prefixLength])
			if err != nil {
				return err
			}
			if matches {
				return s.restoreLedger(old)
			}
		}
		return internalerrors.Conflict("workspace event ledger changed during rollback")
	}
	if err := s.verifyTransactionEvent(transaction); err != nil {
		return err
	}
	return s.restoreLedger(old)
}

func (s *Store) verifyTransactionEvent(transaction workspaceTransaction) error {
	if len(transaction.EventLine) == 0 {
		return nil
	}
	if _, err := decodeEventLine(transaction.EventLine); err != nil {
		return internalerrors.Validation("workspace transaction has invalid event line")
	}
	return s.verifyLedgerEventAt(transaction.OldEventsSize, transaction.EventLine)
}

func (s *Store) verifyLedgerEventAt(offset int64, expected []byte) error {
	if offset < 0 || len(expected) == 0 {
		return internalerrors.Validation("workspace transaction has invalid event offset")
	}
	matches, err := s.ledgerBytesMatchAt(offset, expected)
	if err != nil {
		return err
	}
	if !matches {
		return internalerrors.Conflict("workspace event ledger is missing transaction event")
	}
	return nil
}

func (s *Store) ledgerBytesMatchAt(offset int64, expected []byte) (bool, error) {
	if offset < 0 {
		return false, internalerrors.Validation("workspace transaction has invalid event offset")
	}
	if len(expected) == 0 {
		return true, nil
	}
	file, err := s.root.Open(workspaceEventFile)
	if err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	defer file.Close()
	if _, err := file.Seek(offset, io.SeekStart); err != nil {
		return false, filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	actual := make([]byte, len(expected))
	if _, err := io.ReadFull(file, actual); err != nil {
		if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
			return false, nil
		}
		return false, filesystem(filepath.Join(s.path, workspaceEventFile), err)
	}
	return bytes.Equal(actual, expected), nil
}

func (s *Store) readState() (domain.State, []byte, application.Revision, error) {
	if err := s.recoverWorkspaceTransaction(); err != nil {
		return domain.State{}, nil, application.Revision{}, err
	}
	data, err := s.stateBytes()
	if err != nil {
		return domain.State{}, nil, application.Revision{}, err
	}
	state, err := domain.UnmarshalState(data)
	if err != nil {
		return domain.State{}, nil, application.Revision{}, normalizeStorageError("parsing "+filepath.Join(s.path, "state.json"), err)
	}
	for _, source := range state.Sources {
		for _, snapshot := range source.Snapshots {
			if _, _, err := s.documentInfo(domain.RawRef(snapshot.Filename)); err != nil {
				return domain.State{}, nil, application.Revision{}, err
			}
		}
		if source.Summary == nil {
			continue
		}
		_, info, err := s.documentInfo(domain.SummaryRef(source.Summary.Filename))
		if err != nil {
			return domain.State{}, nil, application.Revision{}, err
		}
		if info.Size() == 0 {
			return domain.State{}, nil, application.Revision{}, internalerrors.MissingResource(fmt.Sprintf("referenced summary is empty: %s", source.Summary.Filename))
		}
	}
	for _, distillation := range state.DistillationDocuments {
		_, info, err := s.documentInfo(domain.DistillationRef(distillation.Filename))
		if err != nil {
			return domain.State{}, nil, application.Revision{}, err
		}
		if info.Size() == 0 {
			return domain.State{}, nil, application.Revision{}, internalerrors.MissingResource(fmt.Sprintf("referenced distillation document is empty: %s", distillation.Filename))
		}
	}
	revision, err := s.workspaceRevision(data)
	if err != nil {
		return domain.State{}, nil, application.Revision{}, err
	}
	return state, data, revision, nil
}

func validTransactionTemporary(path, directory, prefix string) bool {
	if filepath.IsAbs(path) || filepath.Dir(path) != directory {
		return false
	}
	base := filepath.Base(path)
	return strings.HasPrefix(base, prefix) && strings.HasSuffix(base, ".tmp") && !strings.ContainsAny(base, `/\\`)
}

func newWorkspaceTransaction(kind, name string, oldDocument []byte, hadOldDocument bool, newDocument, oldState, newState []byte) workspaceTransaction {
	id := fmt.Sprintf("%d-%d", os.Getpid(), time.Now().UnixNano())
	documentTemporary := ".bo-raw-" + id + ".tmp"
	if kind == workspaceTransactionKindSummary {
		documentTemporary = filepath.Join("summaries", ".bo-summary-"+id+".tmp")
	} else if kind == workspaceTransactionKindDistillation {
		documentTemporary = filepath.Join("distillations", ".bo-distillation-"+id+".tmp")
	}
	return workspaceTransaction{
		Version:              workspaceTransactionVersion,
		Phase:                workspaceTransactionPhaseReady,
		Kind:                 kind,
		DocumentName:         name,
		DocumentTemporary:    documentTemporary,
		StateTemporary:       ".bo-state-" + id + ".tmp",
		TransactionTemporary: ".bo-transaction-" + id + ".tmp",
		HadOldDocument:       hadOldDocument,
		OldDocument:          append([]byte(nil), oldDocument...),
		NewDocument:          append([]byte(nil), newDocument...),
		OldState:             append([]byte(nil), oldState...),
		NewState:             append([]byte(nil), newState...),
	}
}

func (transaction workspaceTransaction) validate() error {
	if transaction.Version != workspaceTransactionVersion {
		return internalerrors.Validation("unsupported workspace transaction version")
	}
	if transaction.Phase != workspaceTransactionPhaseReady && transaction.Phase != workspaceTransactionPhaseCommit {
		return internalerrors.Validation("invalid workspace transaction phase")
	}
	if transaction.Kind != workspaceTransactionKindSnapshot && transaction.Kind != workspaceTransactionKindSummary && transaction.Kind != workspaceTransactionKindDistillation {
		return internalerrors.Validation("invalid workspace transaction kind")
	}
	if err := domain.ValidateDocumentName(transaction.DocumentName); err != nil {
		return err
	}
	if transaction.Kind == workspaceTransactionKindSnapshot {
		if !validTransactionTemporary(transaction.DocumentTemporary, ".", ".bo-raw-") {
			return internalerrors.Validation("invalid snapshot transaction temporary path")
		}
	} else if transaction.Kind == workspaceTransactionKindSummary {
		if !validTransactionTemporary(transaction.DocumentTemporary, "summaries", ".bo-summary-") {
			return internalerrors.Validation("invalid summary transaction temporary path")
		}
	} else if !validTransactionTemporary(transaction.DocumentTemporary, "distillations", ".bo-distillation-") {
		return internalerrors.Validation("invalid distillation transaction temporary path")
	}
	if !validTransactionTemporary(transaction.StateTemporary, ".", ".bo-state-") ||
		!validTransactionTemporary(transaction.TransactionTemporary, ".", ".bo-transaction-") {
		return internalerrors.Validation("invalid workspace transaction temporary path")
	}
	if len(transaction.OldState) == 0 || len(transaction.NewState) == 0 {
		return internalerrors.Validation("workspace transaction has incomplete state")
	}
	if transaction.EventsTracked {
		old := transactionLedgerState(transaction, true)
		newState := transactionLedgerState(transaction, false)
		if old.size < 0 || newState.size < old.size {
			return internalerrors.Validation("workspace transaction has incomplete event ledger state")
		}
		if len(transaction.EventLine) != 0 {
			if !newState.present || len(transaction.EventLine) > maxOperationEventBytes || transaction.EventLine[len(transaction.EventLine)-1] != '\n' || int64(len(transaction.EventLine))+old.size != newState.size {
				return internalerrors.Validation("workspace transaction has invalid event line")
			}
			if _, err := decodeEventLine(transaction.EventLine); err != nil {
				return internalerrors.Validation("workspace transaction has invalid event line")
			}
		} else if !equalLedgerSnapshot(old, newState) {
			return internalerrors.Validation("workspace transaction has incomplete event line")
		}
	}
	return nil
}

func (s *Store) beginWorkspaceTransaction(transaction workspaceTransaction) error {
	return s.writeWorkspaceTransaction(transaction)
}

func (s *Store) writeWorkspaceTransaction(transaction workspaceTransaction) error {
	data, err := json.Marshal(transaction)
	if err != nil {
		return normalizeStorageError("serializing workspace transaction", err)
	}
	return s.writeAtomic(workspaceTransactionFile, transaction.TransactionTemporary, data)
}

func (s *Store) readWorkspaceTransaction() (*workspaceTransaction, error) {
	data, err := s.root.ReadFile(workspaceTransactionFile)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, filesystem(filepath.Join(s.path, workspaceTransactionFile), err)
	}
	var transaction workspaceTransaction
	if err := json.Unmarshal(data, &transaction); err != nil {
		return nil, normalizeStorageError("parsing workspace transaction", err)
	}
	if err := transaction.validate(); err != nil {
		return nil, err
	}
	return &transaction, nil
}

func (s *Store) recoverWorkspaceTransaction() error {
	transaction, err := s.readWorkspaceTransaction()
	if err != nil || transaction == nil {
		return err
	}
	if transaction.Phase == workspaceTransactionPhaseCommit {
		return s.finishWorkspaceTransaction(*transaction)
	}
	return s.rollbackWorkspaceTransaction(*transaction)
}

func (s *Store) publishWorkspaceCommit(transaction workspaceTransaction) error {
	transaction.Phase = workspaceTransactionPhaseCommit
	if err := s.writeWorkspaceTransaction(transaction); err != nil {
		return errors.Join(err, s.recoverWorkspaceTransaction())
	}
	return nil
}

func (s *Store) finishWorkspaceTransaction(transaction workspaceTransaction) error {
	if err := s.publishTransactionEvent(transaction); err != nil {
		return err
	}
	state, err := s.stateBytes()
	if err != nil {
		return err
	}
	stateIsOld := bytes.Equal(state, transaction.OldState)
	stateIsNew := bytes.Equal(state, transaction.NewState)
	if !stateIsOld && !stateIsNew {
		return internalerrors.Conflict("workspace transaction state changed during recovery")
	}
	contents, exists, err := s.optionalTransactionDocument(transaction)
	if err != nil {
		return err
	}
	documentIsOld := transaction.documentIsOld(contents, exists)
	documentIsNew := transaction.documentIsNew(contents, exists)
	if !documentIsOld && !documentIsNew {
		return internalerrors.Conflict("workspace transaction content changed during recovery")
	}
	if err := s.clearWorkspaceTransactionTemps(transaction); err != nil {
		return err
	}
	if !documentIsNew {
		if err := s.writeAtomic(transaction.documentPath(), transaction.DocumentTemporary, transaction.NewDocument); err != nil {
			return err
		}
	}
	if !stateIsNew {
		if err := s.writeAtomic("state.json", transaction.StateTemporary, transaction.NewState); err != nil {
			return err
		}
	}
	return s.removeWorkspaceTransaction(transaction)
}

func (s *Store) abortWorkspaceTransaction(transaction workspaceTransaction, cause error) error {
	transaction.Phase = workspaceTransactionPhaseReady
	phaseErr := s.writeWorkspaceTransaction(transaction)
	rollbackErr := s.rollbackWorkspaceTransaction(transaction)
	return errors.Join(cause, phaseErr, rollbackErr)
}

func (s *Store) rollbackWorkspaceTransaction(transaction workspaceTransaction) error {
	if err := s.rollbackTransactionEvent(transaction); err != nil {
		return err
	}
	state, err := s.stateBytes()
	if err != nil {
		return err
	}
	stateIsOld := bytes.Equal(state, transaction.OldState)
	stateIsNew := bytes.Equal(state, transaction.NewState)
	if !stateIsOld && !stateIsNew {
		return internalerrors.Conflict("workspace transaction state changed during rollback")
	}
	contents, exists, err := s.optionalTransactionDocument(transaction)
	if err != nil {
		return err
	}
	documentIsOld := transaction.documentIsOld(contents, exists)
	documentIsNew := transaction.documentIsNew(contents, exists)
	if !documentIsOld && !documentIsNew {
		return internalerrors.Conflict("workspace transaction content changed during rollback")
	}
	if err := s.clearWorkspaceTransactionTemps(transaction); err != nil {
		return err
	}
	if !documentIsOld {
		if transaction.HadOldDocument {
			if err := s.writeAtomic(transaction.documentPath(), transaction.DocumentTemporary, transaction.OldDocument); err != nil {
				return err
			}
		} else {
			path := transaction.documentPath()
			if err := s.root.Remove(path); err != nil && !os.IsNotExist(err) {
				return filesystem(filepath.Join(s.path, path), err)
			}
			if err := syncDirectoryError(s.root, filepath.Dir(path)); err != nil {
				return err
			}
		}
	}
	if !stateIsOld {
		if err := s.writeAtomic("state.json", transaction.StateTemporary, transaction.OldState); err != nil {
			return err
		}
	}
	return s.removeWorkspaceTransaction(transaction)
}

func (s *Store) optionalTransactionDocument(transaction workspaceTransaction) ([]byte, bool, error) {
	kind := domain.DocumentKindRaw
	if transaction.Kind == workspaceTransactionKindSummary {
		kind = domain.DocumentKindSummary
	} else if transaction.Kind == workspaceTransactionKindDistillation {
		kind = domain.DocumentKindDistillation
	}
	return s.optionalDocument(domain.DocumentRef{Kind: kind, Name: transaction.DocumentName})
}

func (transaction workspaceTransaction) documentPath() string {
	if transaction.Kind == workspaceTransactionKindSummary {
		return filepath.Join("summaries", transaction.DocumentName)
	}
	if transaction.Kind == workspaceTransactionKindDistillation {
		return filepath.Join("distillations", transaction.DocumentName)
	}
	return transaction.DocumentName
}

func (transaction workspaceTransaction) documentIsOld(contents []byte, exists bool) bool {
	return exists == transaction.HadOldDocument && (!exists || bytes.Equal(contents, transaction.OldDocument))
}

func (transaction workspaceTransaction) documentIsNew(contents []byte, exists bool) bool {
	return exists && bytes.Equal(contents, transaction.NewDocument)
}

func (s *Store) clearWorkspaceTransactionTemps(transaction workspaceTransaction) error {
	for _, path := range []string{transaction.DocumentTemporary, transaction.StateTemporary, transaction.TransactionTemporary} {
		if err := s.root.Remove(path); err != nil && !os.IsNotExist(err) {
			return filesystem(filepath.Join(s.path, path), err)
		}
	}
	return nil
}

func (s *Store) removeWorkspaceTransaction(transaction workspaceTransaction) error {
	if err := s.clearWorkspaceTransactionTemps(transaction); err != nil {
		return err
	}
	if err := s.root.Remove(workspaceTransactionFile); err != nil && !os.IsNotExist(err) {
		return filesystem(filepath.Join(s.path, workspaceTransactionFile), err)
	}
	return syncRoot(s.root)
}

func (s *Store) workspaceRevision(state []byte) (application.Revision, error) {
	var data bytes.Buffer
	writeRevisionPart(&data, state)
	for _, kind := range []domain.DocumentKind{domain.DocumentKindRaw, domain.DocumentKindSummary, domain.DocumentKindDistillation} {
		refs, err := s.listDocuments(kind)
		if err != nil {
			return application.Revision{}, err
		}
		for _, ref := range refs {
			writeRevisionPart(&data, []byte(kind))
			writeRevisionPart(&data, []byte(ref.Name))
			_, info, err := s.documentInfo(ref)
			if err != nil {
				return application.Revision{}, err
			}
			var fingerprint [16]byte
			binary.BigEndian.PutUint64(fingerprint[:8], uint64(info.Size()))
			binary.BigEndian.PutUint64(fingerprint[8:], uint64(info.ModTime().UnixNano()))
			writeRevisionPart(&data, fingerprint[:])
		}
	}
	return application.NewRevision(data.Bytes()), nil
}

func (s *Store) ensureDocumentBaseline(state *domain.State, ref domain.DocumentRef, contents []byte) error {
	_, info, err := s.documentInfo(ref)
	if err != nil {
		return err
	}
	digest := application.NewRevision(contents).String()
	modifiedAt := info.ModTime().UTC().Format(time.RFC3339Nano)
	size := info.Size()
	setBaseline := func(currentDigest *string, currentSize **int64, currentModifiedAt *string) error {
		if *currentDigest != "" && strings.EqualFold(*currentDigest, digest) && *currentSize != nil && **currentSize == size && *currentModifiedAt == modifiedAt {
			return nil
		}
		if *currentDigest != "" && *currentSize != nil && **currentSize == size && *currentModifiedAt == modifiedAt && !strings.EqualFold(*currentDigest, digest) {
			return internalerrors.Conflict("workspace document changed")
		}
		*currentDigest = digest
		*currentSize = &size
		*currentModifiedAt = modifiedAt
		return nil
	}
	switch ref.Kind {
	case domain.DocumentKindRaw:
		for sourceIndex := range state.Sources {
			for snapshotIndex := range state.Sources[sourceIndex].Snapshots {
				snapshot := &state.Sources[sourceIndex].Snapshots[snapshotIndex]
				if snapshot.Filename == ref.Name {
					return setBaseline(&snapshot.ContentDigest, &snapshot.ContentSize, &snapshot.ContentModifiedAt)
				}
			}
		}
	case domain.DocumentKindSummary:
		for sourceIndex := range state.Sources {
			summary := state.Sources[sourceIndex].Summary
			if summary != nil && summary.Filename == ref.Name {
				return setBaseline(&summary.ContentDigest, &summary.ContentSize, &summary.ContentModifiedAt)
			}
		}
	case domain.DocumentKindDistillation:
		for index := range state.DistillationDocuments {
			record := &state.DistillationDocuments[index]
			if record.Filename == ref.Name {
				return setBaseline(&record.ContentDigest, &record.ContentSize, &record.ContentModifiedAt)
			}
		}
	default:
		return internalerrors.Validation("unsupported document kind")
	}
	return internalerrors.MissingResource("document is not in workspace state")
}

func writeRevisionPart(buffer *bytes.Buffer, value []byte) {
	var length [8]byte
	binary.BigEndian.PutUint64(length[:], uint64(len(value)))
	buffer.Write(length[:])
	buffer.Write(value)
}

func (s *Store) applyDistillation(state domain.State, commit application.DistillationCommit) (domain.State, error) {
	next, err := state.ApplyDistillation(commit)
	if err != nil {
		return domain.State{}, err
	}
	for _, input := range commit.DerivedFrom {
		ref := domain.DocumentRef{Kind: input.Kind, Name: input.Filename}
		contents, err := s.readDocument(ref)
		if err != nil {
			return domain.State{}, err
		}
		if !strings.EqualFold(application.NewRevision(contents).String(), input.ContentDigest) {
			return domain.State{}, internalerrors.Conflict(fmt.Sprintf("workspace document changed: %s", input.Filename))
		}
		if err := s.ensureDocumentBaseline(&next, ref, contents); err != nil {
			return domain.State{}, err
		}
	}
	return next, nil
}

func snapshotRecord(state domain.State, sourceKey, filename string) *domain.RawRecord {
	for sourceIndex := range state.Sources {
		source := &state.Sources[sourceIndex]
		if source.SourceKey != sourceKey {
			continue
		}
		for snapshotIndex := range source.Snapshots {
			if source.Snapshots[snapshotIndex].Filename == filename {
				return &source.Snapshots[snapshotIndex]
			}
		}
	}
	return nil
}

func summaryRecord(state domain.State, sourceKey string) *domain.SummaryRecord {
	for _, source := range state.Sources {
		if source.SourceKey == sourceKey {
			return source.Summary
		}
	}
	return nil
}

func (s *Store) writeNewRaw(name, temporary string, data []byte) error {
	file, err := s.root.OpenFile(temporary, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
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
		if _, statErr := s.root.Lstat(name); statErr == nil {
			err = internalerrors.Wrap(internalerrors.KindAlreadyExists, "raw document already exists", internalerrors.ErrAlreadyExists)
		} else if !os.IsNotExist(statErr) {
			err = filesystem(filepath.Join(s.path, name), statErr)
		}
	}
	if err == nil {
		err = s.root.Rename(temporary, name)
	}
	if err == nil {
		err = syncRoot(s.root)
	}
	if err != nil {
		_ = s.root.Remove(temporary)
		if errors.Is(err, internalerrors.ErrAlreadyExists) {
			return err
		}
		return filesystem(filepath.Join(s.path, name), err)
	}
	return nil
}

func (s *Store) optionalDocument(ref domain.DocumentRef) ([]byte, bool, error) {
	path, err := s.documentPath(ref)
	if err != nil {
		return nil, false, err
	}
	if _, err := s.root.Lstat(path); err != nil {
		if os.IsNotExist(err) {
			return nil, false, nil
		}
		return nil, false, filesystem(filepath.Join(s.path, path), err)
	}
	data, err := s.readDocument(ref)
	return data, true, err
}

func (s *Store) ensureSummaryDirectory() error {
	if info, err := s.root.Lstat("summaries"); err == nil {
		if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
			return internalerrors.Filesystem("summaries must be a directory")
		}
		return nil
	} else if !os.IsNotExist(err) {
		return filesystem(filepath.Join(s.path, "summaries"), err)
	}
	if err := s.root.Mkdir("summaries", 0o755); err != nil {
		return filesystem(filepath.Join(s.path, "summaries"), err)
	}
	return syncRoot(s.root)
}

func (s *Store) ensureDistillationDirectory() error {
	if info, err := s.root.Lstat("distillations"); err == nil {
		if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
			return internalerrors.Filesystem("distillations must be a directory")
		}
		return nil
	} else if !os.IsNotExist(err) {
		return filesystem(filepath.Join(s.path, "distillations"), err)
	}
	if err := s.root.Mkdir("distillations", 0o755); err != nil {
		return filesystem(filepath.Join(s.path, "distillations"), err)
	}
	return syncRoot(s.root)
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
	if err := domain.ValidateDocumentName(ref.Name); err != nil {
		return "", err
	}
	switch ref.Kind {
	case domain.DocumentKindRaw:
		return ref.Name, nil
	case domain.DocumentKindSummary:
		return filepath.Join("summaries", ref.Name), nil
	case domain.DocumentKindDistillation:
		return filepath.Join("distillations", ref.Name), nil
	default:
		return "", internalerrors.Validation("unsupported document kind")
	}
}

func (s *Store) documentInfo(ref domain.DocumentRef) (string, os.FileInfo, error) {
	path, err := s.documentPath(ref)
	if err != nil {
		return "", nil, err
	}
	info, err := s.root.Stat(path)
	if err != nil {
		return "", nil, filesystem(filepath.Join(s.path, path), err)
	}
	if !info.Mode().IsRegular() {
		return "", nil, internalerrors.Filesystem(fmt.Sprintf("document is not a regular file: %s", filepath.Join(s.path, path)))
	}
	return path, info, nil
}

func (s *Store) writeAtomic(destination, temporary string, data []byte) error {
	file, err := s.root.OpenFile(temporary, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
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
	return syncDirectoryError(s.root, filepath.Dir(destination))
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

func contextErr(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return internalerrors.Context(ctx.Err())
	default:
		return nil
	}
}

func syncRoot(root *os.Root) error {
	return syncDirectoryError(root, ".")
}

func syncDirectoryError(root *os.Root, path string) error {
	directory, err := root.Open(path)
	if err != nil {
		return filesystem(path, err)
	}
	defer directory.Close()
	if err := directory.Sync(); err != nil {
		return filesystem(path, err)
	}
	return nil
}
