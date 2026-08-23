package application

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"time"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

// Revision is an opaque workspace version. Callers can only compare it or
// pass it back to a workspace implementation.
type Revision struct{ digest [sha256.Size]byte }

func NewRevision(data []byte) Revision { return Revision{digest: sha256.Sum256(data)} }

func RevisionFromString(value string) (Revision, error) {
	data, err := hex.DecodeString(value)
	if err != nil || len(data) != sha256.Size {
		return Revision{}, internalerrors.Validation("invalid revision")
	}
	var digest [sha256.Size]byte
	copy(digest[:], data)
	return Revision{digest: digest}, nil
}

func (r Revision) Equal(other Revision) bool { return r == other }
func (r Revision) IsZero() bool              { return r == Revision{} }
func (r Revision) String() string            { return hex.EncodeToString(r.digest[:]) }

type SnapshotCommit struct {
	SourceKey string
	Filename  string
	WrittenAt time.Time
	Contents  []byte
}

type SummaryCommit struct {
	SourceKey    string
	Filename     string
	DerivedFrom  string
	RawWrittenAt time.Time
	CreatedAt    time.Time
	UpdatedAt    time.Time
	Contents     []byte
}

// Workspace is the persistence boundary for one workspace.
type Workspace interface {
	Name() string
	ListDocuments(context.Context, domain.DocumentKind) ([]domain.DocumentRef, error)
	ReadDocument(context.Context, domain.DocumentRef) ([]byte, error)
	ReadState(context.Context) (domain.State, Revision, error)
	CommitSnapshot(context.Context, SnapshotCommit, Revision) (domain.State, Revision, error)
	CommitSummary(context.Context, SummaryCommit, Revision) (domain.State, Revision, error)
}

type Operation = domain.Operation
type OperationCommand = domain.OperationCommand

const (
	CommandSeed         = domain.CommandSeed
	CommandSnap         = domain.CommandSnap
	CommandState        = domain.CommandState
	CommandSynth        = domain.CommandSynth
	CommandWriteSummary = domain.CommandWriteSummary
)

type OperationPage struct {
	Directory  string      `json:"directory"`
	Entries    []Operation `json:"entries"`
	Offset     int         `json:"offset"`
	Limit      int         `json:"limit"`
	NextOffset int         `json:"next_offset"`
	HasMore    bool        `json:"has_more"`
}

type OperationLog interface {
	Append(context.Context, Operation) error
	Read(context.Context, string, int, int) (OperationPage, error)
}

type OperationOptions struct {
	Log   OperationLog
	Actor string
}

type WorkspaceCreator interface {
	Create(context.Context, string) (string, error)
}
