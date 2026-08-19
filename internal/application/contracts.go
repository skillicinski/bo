package application

import (
	"context"
	"crypto/sha256"
	"encoding/hex"

	"github.com/skillicinski/bo/internal/domain"
)

type Page struct {
	Title     string
	Markdown  string
	SourceURL string
}

type Source interface {
	Fetch(context.Context, string) (Page, error)
}

type Fetcher = Source

// Generation is an opaque storage version. Callers can only compare it or
// pass it back to a storage implementation.
type Generation struct{ digest [sha256.Size]byte }

func NewGeneration(data []byte) Generation { return Generation{digest: sha256.Sum256(data)} }

func (g Generation) Equal(other Generation) bool { return g == other }
func (g Generation) IsZero() bool                { return g == Generation{} }
func (g Generation) String() string              { return hex.EncodeToString(g.digest[:]) }

type Storage interface {
	CreateRaw(context.Context, string, []byte) (domain.DocumentRef, error)
	ReadDocument(context.Context, domain.DocumentRef) ([]byte, error)
	ListMarkdownDocuments(context.Context, domain.DocumentKind) ([]domain.DocumentRef, error)
	ReplaceSummary(context.Context, domain.DocumentRef, []byte) error
	DeleteDocument(context.Context, domain.DocumentRef) error
	ReadState(context.Context) (domain.State, Generation, error)
	PublishState(context.Context, domain.State, Generation) (Generation, error)
}

type DocumentStorage = Storage
type DocumentStore = Storage

// Workspace is the current local synthesis boundary.
// ponytail: synthesis currently requires workspace paths; replace them with document access methods when a remote workspace adapter is needed.
type Workspace interface {
	Name() string
	RootPath() string
	TargetPath() string
	Storage() Storage
	Close() error
}

type WorkspaceCreator interface {
	Create(context.Context, string) (string, error)
}

type WorkspaceOpener interface {
	Open(context.Context, string) (Workspace, error)
}
