package bo

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
)

type ErrorCategory string

type ErrorKind = ErrorCategory

const (
	CategoryInput       ErrorCategory = "input"
	CategoryRequest     ErrorCategory = "request"
	CategoryHTTP        ErrorCategory = "http"
	CategoryContent     ErrorCategory = "content"
	CategoryFilesystem  ErrorCategory = "filesystem"
	CategoryUnsupported ErrorCategory = "unsupported"
	CategoryConflict    ErrorCategory = "conflict"
)

const (
	ErrorInput       = CategoryInput
	ErrorRequest     = CategoryRequest
	ErrorHTTP        = CategoryHTTP
	ErrorContent     = CategoryContent
	ErrorFilesystem  = CategoryFilesystem
	ErrorUnsupported = CategoryUnsupported
	ErrorConflict    = CategoryConflict
)

// Error is a user-facing error with a stable category.
type Error struct {
	Category  ErrorCategory
	Detail    string
	Status    int
	RequestID string
	Cause     error
}

func (e *Error) Error() string {
	if e.Category == CategoryHTTP && e.Status != 0 {
		if e.RequestID != "" {
			return fmt.Sprintf("http: HTTP %d (request_id: %s)", e.Status, e.RequestID)
		}
		return fmt.Sprintf("http: HTTP %d", e.Status)
	}
	return fmt.Sprintf("%s: %s", e.Category, e.Detail)
}

func (e *Error) Unwrap() error { return e.Cause }

func NewError(category ErrorCategory, detail string) *Error {
	return &Error{Category: category, Detail: detail}
}

func InputError(detail string) *Error       { return NewError(CategoryInput, detail) }
func RequestError(detail string) *Error     { return NewError(CategoryRequest, detail) }
func ContentError(detail string) *Error     { return NewError(CategoryContent, detail) }
func FilesystemError(detail string) *Error  { return NewError(CategoryFilesystem, detail) }
func UnsupportedError(detail string) *Error { return NewError(CategoryUnsupported, detail) }
func ConflictError(detail string) *Error    { return NewError(CategoryConflict, detail) }

func HTTPError(status int, requestID string) *Error {
	return &Error{Category: CategoryHTTP, Status: status, RequestID: requestID}
}

var ErrAlreadyExists = errors.New("document already exists")

type SnapError = Error

func IsCategory(err error, category ErrorCategory) bool {
	var categorized *Error
	return errors.As(err, &categorized) && categorized.Category == category
}

func IsConflict(err error) bool      { return IsCategory(err, CategoryConflict) }
func IsFilesystem(err error) bool    { return IsCategory(err, CategoryFilesystem) }
func IsAlreadyExists(err error) bool { return errors.Is(err, ErrAlreadyExists) }

type State struct {
	Raw       []RawRecord     `json:"raw"`
	Summaries []SummaryRecord `json:"summaries"`
}

func (s State) MarshalJSON() ([]byte, error) {
	raw := s.Raw
	if raw == nil {
		raw = []RawRecord{}
	}
	summaries := s.Summaries
	if summaries == nil {
		summaries = []SummaryRecord{}
	}
	type state State
	return json.Marshal(state{Raw: raw, Summaries: summaries})
}

type RawRecord struct {
	Filename  string `json:"filename"`
	URL       string `json:"url"`
	WrittenAt uint64 `json:"written_at"`
}

type SummaryRecord struct {
	Filename    string `json:"filename"`
	SourceKey   string `json:"source_key"`
	DerivedFrom string `json:"derived_from"`
	CreatedAt   uint64 `json:"created_at"`
	UpdatedAt   uint64 `json:"updated_at"`
}

func MarshalState(state State) ([]byte, error) {
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(data, '\n'), nil
}

func UnmarshalState(data []byte) (State, error) {
	var state State
	if err := json.Unmarshal(data, &state); err != nil {
		return State{}, err
	}
	if state.Raw == nil {
		state.Raw = []RawRecord{}
	}
	if state.Summaries == nil {
		state.Summaries = []SummaryRecord{}
	}
	return state, nil
}

// Generation is an opaque storage version. Callers can only compare or pass it
// back to a storage implementation.
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

type Page struct {
	Title     string
	Markdown  string
	SourceURL string
}

type Source interface {
	Fetch(context.Context, string) (Page, error)
}

type Fetcher = Source

type SnapOutcome struct {
	SourceURL string
	Filename  string
	Err       error
}

func (o SnapOutcome) Failed() bool { return o.Err != nil }

type SnapCommandError struct {
	Completed []SnapOutcome
	SourceURL string
	Err       error
}

func (e *SnapCommandError) Error() string {
	if e.SourceURL != "" && e.Err != nil {
		return fmt.Sprintf("%s (%s)", e.SourceURL, e.Err)
	}
	if e.Err != nil {
		return e.Err.Error()
	}
	return "snap failed"
}

func (e *SnapCommandError) Unwrap() error { return e.Err }

func NewSnapInputError(detail string) *SnapCommandError {
	return &SnapCommandError{Err: InputError(detail)}
}

type ChatMessage struct {
	Role             string     `json:"role"`
	Content          any        `json:"content"`
	Name             string     `json:"name,omitempty"`
	ToolCalls        []ToolCall `json:"tool_calls,omitempty"`
	ToolCallID       string     `json:"tool_call_id,omitempty"`
	ReasoningContent any        `json:"reasoning_content,omitempty"`
}

type ToolCall struct {
	ID       string       `json:"id"`
	Type     string       `json:"type"`
	Function ToolFunction `json:"function"`
}

type ToolFunction struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

type ToolDefinition struct {
	Type     string          `json:"type"`
	Function ToolDeclaration `json:"function"`
}

type ToolDeclaration struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Parameters  map[string]any `json:"parameters"`
}

type CompletionRequest struct {
	Model      string            `json:"model"`
	Messages   []ChatMessage     `json:"messages"`
	Tools      []ToolDefinition  `json:"tools"`
	ToolChoice string            `json:"tool_choice"`
	Stream     bool              `json:"stream"`
	MaxTokens  int               `json:"max_tokens"`
	Thinking   map[string]string `json:"thinking"`
}

type CompletionResponse struct {
	Message      ChatMessage
	FinishReason string
}

type CompletionProvider interface {
	Complete(context.Context, CompletionRequest) (CompletionResponse, error)
}

type Provider = CompletionProvider
