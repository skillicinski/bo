package bo

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
)

// Generation is an opaque storage version. Callers can only compare it or
// pass it back to a storage implementation.
type Generation struct{ digest [sha256.Size]byte }

func NewGeneration(data []byte) Generation { return Generation{digest: sha256.Sum256(data)} }

func (g Generation) Equal(other Generation) bool { return g == other }
func (g Generation) IsZero() bool                { return g == Generation{} }
func (g Generation) String() string              { return hex.EncodeToString(g.digest[:]) }

type DocumentKind string

const (
	DocumentKindRaw     DocumentKind = "raw"
	DocumentKindSummary DocumentKind = "summary"
	RawDocument                      = DocumentKindRaw
	SummaryDocument                  = DocumentKindSummary
)

type DocumentRef struct {
	Kind DocumentKind
	Name string
}

func RawRef(name string) DocumentRef     { return DocumentRef{Kind: DocumentKindRaw, Name: name} }
func SummaryRef(name string) DocumentRef { return DocumentRef{Kind: DocumentKindSummary, Name: name} }

type Storage interface {
	CreateRaw(context.Context, string, []byte) (DocumentRef, error)
	ReadDocument(context.Context, DocumentRef) ([]byte, error)
	ListMarkdownDocuments(context.Context, DocumentKind) ([]DocumentRef, error)
	ReplaceSummary(context.Context, DocumentRef, []byte) error
	DeleteDocument(context.Context, DocumentRef) error
	ReadState(context.Context) (State, Generation, error)
	PublishState(context.Context, State, Generation) (Generation, error)
}

type DocumentStorage = Storage
type DocumentStore = Storage
