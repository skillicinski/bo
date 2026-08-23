package bo

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	stderrors "errors"
	"fmt"
	"net/http"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	app "github.com/skillicinski/bo/internal/application"
	internaldomain "github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	deepseek "github.com/skillicinski/bo/internal/provider/deepseek"
	loc "github.com/skillicinski/bo/internal/storage/local"
)

type ErrorCategory string

const (
	CategoryInput       ErrorCategory = "input"
	CategoryRequest     ErrorCategory = "request"
	CategoryHTTP        ErrorCategory = "http"
	CategoryContent     ErrorCategory = "content"
	CategoryFilesystem  ErrorCategory = "filesystem"
	CategoryUnsupported ErrorCategory = "unsupported"
	CategoryConflict    ErrorCategory = "conflict"
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

var ErrAlreadyExists = stderrors.New("document already exists")

func IsCategory(err error, category ErrorCategory) bool {
	var categorized *Error
	if stderrors.As(err, &categorized) {
		return categorized.Category == category
	}
	var internalCategorized *internalerrors.Error
	return stderrors.As(err, &internalCategorized) && ErrorCategory(internalCategorized.Category) == category
}

func IsConflict(err error) bool   { return IsCategory(err, CategoryConflict) }
func IsFilesystem(err error) bool { return IsCategory(err, CategoryFilesystem) }
func IsAlreadyExists(err error) bool {
	return stderrors.Is(err, ErrAlreadyExists) || stderrors.Is(err, internalerrors.ErrAlreadyExists)
}

type DocumentKind string

const (
	DocumentKindRaw     DocumentKind = "raw"
	DocumentKindSummary DocumentKind = "summary"
)

type DocumentRef struct {
	Kind DocumentKind
	Name string
}

func RawRef(name string) DocumentRef     { return DocumentRef{Kind: DocumentKindRaw, Name: name} }
func SummaryRef(name string) DocumentRef { return DocumentRef{Kind: DocumentKindSummary, Name: name} }

type Generation struct{ digest [sha256.Size]byte }

func NewGeneration(data []byte) Generation { return Generation{digest: sha256.Sum256(data)} }

func (g Generation) Equal(other Generation) bool { return g == other }
func (g Generation) IsZero() bool                { return g == Generation{} }
func (g Generation) String() string              { return hex.EncodeToString(g.digest[:]) }

type RawRecord struct {
	Filename  string    `json:"filename"`
	WrittenAt time.Time `json:"written_at"`
}

type SummaryRecord struct {
	Filename    string    `json:"filename"`
	DerivedFrom string    `json:"derived_from"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
}

type SourceRecord struct {
	SourceKey string         `json:"source_key"`
	Snapshots []RawRecord    `json:"snapshots"`
	Summary   *SummaryRecord `json:"summary,omitempty"`
}

type State struct {
	Sources []SourceRecord `json:"sources"`
}

func (s State) SnapshotCount() int {
	count := 0
	for _, source := range s.Sources {
		count += len(source.Snapshots)
	}
	return count
}

type Storage interface {
	CreateRaw(context.Context, string, []byte) (DocumentRef, error)
	ReadDocument(context.Context, DocumentRef) ([]byte, error)
	ReplaceSummary(context.Context, DocumentRef, []byte) error
	DeleteDocument(context.Context, DocumentRef) error
	ReadState(context.Context) (State, Generation, error)
	PublishState(context.Context, State, Generation) (Generation, error)
}

// Workspace is a caller-scoped workspace. bo does not select or mutate the
// tenant, authentication, routing, or storage configuration for a workspace.
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

type OperationCommand string

const (
	CommandSeed         OperationCommand = "seed"
	CommandSnap         OperationCommand = "snap"
	CommandState        OperationCommand = "state"
	CommandSynth        OperationCommand = "synth"
	CommandWriteSummary OperationCommand = "write_summary"
)

type Operation struct {
	Timestamp string           `json:"timestamp"`
	Actor     string           `json:"actor"`
	Directory string           `json:"directory"`
	Command   OperationCommand `json:"command"`
	Success   bool             `json:"success"`
	Details   map[string]any   `json:"details"`
}

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
	// Log is required. Workflows reject requests without durable operation logging.
	Log   OperationLog
	Actor string
}

type SeedRequest struct {
	Creator    WorkspaceCreator
	Name       string
	Operations OperationOptions
}

type SeedResult struct {
	Name string `json:"name"`
}

type SnapRequest struct {
	Workspace  Workspace
	Sources    []string
	Operations OperationOptions
}

type SnapOutcome struct {
	SourceKey string
	Filename  string
	Err       error
}

func (o SnapOutcome) Failed() bool { return o.Err != nil }

type SnapResult struct {
	Outcomes     []SnapOutcome `json:"outcomes"`
	Aborted      bool          `json:"aborted"`
	FailedSource string        `json:"failed_source,omitempty"`
}

type StateRequest struct {
	Workspace  Workspace
	Operations OperationOptions
}

type StateResult struct {
	State State `json:"state"`
}

const (
	DefaultMaxTurns                = app.DefaultMaxTurns
	DefaultMaxToolCalls            = app.DefaultMaxToolCalls
	DefaultMaxToolOutputBytes      = app.DefaultMaxToolOutputBytes
	DefaultMaxResponseTokens       = app.DefaultMaxResponseTokens
	DefaultSynthesisTimeoutSeconds = app.DefaultSynthesisTimeoutSeconds
)

type SynthesisOptions struct {
	MaxTurns           int
	MaxToolCalls       int
	MaxToolOutputBytes int
	MaxResponseTokens  int
	TimeoutSeconds     int
}

func DefaultSynthesisOptions() SynthesisOptions {
	return SynthesisOptions{
		MaxTurns: DefaultMaxTurns, MaxToolCalls: DefaultMaxToolCalls,
		MaxToolOutputBytes: DefaultMaxToolOutputBytes, MaxResponseTokens: DefaultMaxResponseTokens,
		TimeoutSeconds: DefaultSynthesisTimeoutSeconds,
	}
}

// Provider is an opaque, caller-scoped LLM provider. Construct one with a
// supported provider constructor; its completion protocol is internal.
type Provider struct {
	completion agent.CompletionProvider
}

type DeepSeekConfig struct {
	APIKey     string
	Endpoint   string
	Model      string
	HTTPClient *http.Client
}

func NewDeepSeekProvider(config DeepSeekConfig) Provider {
	client := deepseek.New(config.APIKey, config.Endpoint)
	client.Model = config.Model
	if config.HTTPClient != nil {
		client.HTTPClient = config.HTTPClient
	}
	return Provider{completion: client}
}

type Usage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

type Metrics struct {
	Turns     int           `json:"turns"`
	ToolCalls int           `json:"tool_calls"`
	Usage     *Usage        `json:"usage,omitempty"`
	Duration  time.Duration `json:"duration"`
}

type SynthRequest struct {
	Workspace  Workspace
	Provider   Provider
	Options    SynthesisOptions
	Operations OperationOptions
}

type SynthResult struct {
	SummariesWritten int     `json:"summaries_written"`
	SummariesSkipped int     `json:"summaries_skipped"`
	Metrics          Metrics `json:"metrics"`
}

type LocalManager struct {
	manager *loc.Manager
}

func NewLocalManager(home string) *LocalManager {
	return &LocalManager{manager: loc.NewManager(home)}
}

func (m *LocalManager) Create(ctx context.Context, name string) (string, error) {
	if m == nil || m.manager == nil {
		return "", RequestError("local workspace manager is not configured")
	}
	created, err := m.manager.Create(ctx, name)
	return created, publicError(err)
}

func (m *LocalManager) Open(ctx context.Context, name string) (Workspace, error) {
	if m == nil || m.manager == nil {
		return nil, RequestError("local workspace manager is not configured")
	}
	workspace, err := m.manager.Open(ctx, name)
	if err != nil {
		return nil, publicError(err)
	}
	if workspace == nil {
		return nil, RequestError("local workspace manager returned no workspace")
	}
	return &localWorkspace{workspace: workspace, storage: &localStorage{storage: workspace.Storage()}}, nil
}

func NewOperationLog(home string) OperationLog {
	return &localOperationLog{log: loc.NewOperationLog(home)}
}

func Seed(ctx context.Context, request SeedRequest) (SeedResult, error) {
	created, err := app.Seed(ctx, request.Creator, request.Name, internalOperationOptions(request.Operations))
	return SeedResult{Name: created}, publicError(err)
}

func Snap(ctx context.Context, request SnapRequest) (SnapResult, error) {
	result := SnapResult{}
	if request.Workspace == nil {
		return result, RequestError("workspace is not configured")
	}
	storage := request.Workspace.Storage()
	if storage == nil {
		return result, RequestError("workspace storage is not configured")
	}
	outcomes, err := app.Snap(ctx, &publicStorage{storage: storage}, request.Workspace.Name(), request.Sources, internalOperationOptions(request.Operations))
	result.Outcomes = publicSnapOutcomes(outcomes)
	var commandErr *app.SnapCommandError
	if stderrors.As(err, &commandErr) {
		result.Outcomes = publicSnapOutcomes(commandErr.Completed)
		result.Aborted = true
		result.FailedSource = commandErr.SourceKey
		return result, publicError(commandErr.Err)
	}
	return result, publicError(err)
}

func ReadState(ctx context.Context, request StateRequest) (StateResult, error) {
	if request.Workspace == nil {
		return StateResult{}, RequestError("workspace is not configured")
	}
	storage := request.Workspace.Storage()
	if storage == nil {
		return StateResult{}, RequestError("workspace storage is not configured")
	}
	state, err := app.ReadState(ctx, &publicStorage{storage: storage}, request.Workspace.Name(), internalOperationOptions(request.Operations))
	if err != nil {
		return StateResult{}, publicError(err)
	}
	converted, err := publicState(state)
	if err != nil {
		return StateResult{}, publicError(err)
	}
	return StateResult{State: converted}, nil
}

func Synth(ctx context.Context, request SynthRequest) (SynthResult, error) {
	if request.Workspace == nil {
		return SynthResult{}, RequestError("workspace is not configured")
	}
	if request.Workspace.Storage() == nil {
		return SynthResult{}, RequestError("workspace storage is not configured")
	}
	workspace := &scopedWorkspace{workspace: request.Workspace, storage: &publicStorage{storage: request.Workspace.Storage()}}
	result, err := app.Synthesize(ctx, fixedWorkspaceOpener{workspace: workspace}, request.Workspace.Name(), request.Provider.completion, app.SynthesisOptions{
		MaxTurns: request.Options.MaxTurns, MaxToolCalls: request.Options.MaxToolCalls,
		MaxToolOutputBytes: request.Options.MaxToolOutputBytes, MaxResponseTokens: request.Options.MaxResponseTokens,
		TimeoutSeconds: request.Options.TimeoutSeconds,
	}, internalOperationOptions(request.Operations))
	return publicSynthResult(result), publicError(err)
}

type publicStorage struct{ storage Storage }

func (s *publicStorage) CreateRaw(ctx context.Context, name string, contents []byte) (internaldomain.DocumentRef, error) {
	ref, err := s.storage.CreateRaw(ctx, name, contents)
	return internalDocumentRef(ref), internalError(err)
}

func (s *publicStorage) ReadDocument(ctx context.Context, ref internaldomain.DocumentRef) ([]byte, error) {
	data, err := s.storage.ReadDocument(ctx, publicDocumentRef(ref))
	return data, internalError(err)
}

func (s *publicStorage) ListMarkdownDocuments(ctx context.Context, kind internaldomain.DocumentKind) ([]internaldomain.DocumentRef, error) {
	lister, ok := s.storage.(interface {
		ListMarkdownDocuments(context.Context, DocumentKind) ([]DocumentRef, error)
	})
	if !ok {
		return []internaldomain.DocumentRef{}, nil
	}
	refs, err := lister.ListMarkdownDocuments(ctx, DocumentKind(kind))
	if err != nil {
		return nil, internalError(err)
	}
	result := make([]internaldomain.DocumentRef, len(refs))
	for index, ref := range refs {
		result[index] = internalDocumentRef(ref)
	}
	return result, nil
}

func (s *publicStorage) ReplaceSummary(ctx context.Context, ref internaldomain.DocumentRef, contents []byte) error {
	return internalError(s.storage.ReplaceSummary(ctx, publicDocumentRef(ref), contents))
}

func (s *publicStorage) DeleteDocument(ctx context.Context, ref internaldomain.DocumentRef) error {
	return internalError(s.storage.DeleteDocument(ctx, publicDocumentRef(ref)))
}

func (s *publicStorage) ReadState(ctx context.Context) (internaldomain.State, app.Generation, error) {
	state, generation, err := s.storage.ReadState(ctx)
	if err != nil {
		return internaldomain.State{}, app.Generation{}, internalError(err)
	}
	converted, err := internalState(state)
	if err != nil {
		return internaldomain.State{}, app.Generation{}, internalError(err)
	}
	internalGeneration, err := app.GenerationFromString(generation.String())
	if err != nil {
		return internaldomain.State{}, app.Generation{}, internalError(err)
	}
	return converted, internalGeneration, nil
}

func (s *publicStorage) PublishState(ctx context.Context, state internaldomain.State, expected app.Generation) (app.Generation, error) {
	converted, err := publicState(state)
	if err != nil {
		return app.Generation{}, internalError(err)
	}
	expectedGeneration, err := app.GenerationFromString(expected.String())
	if err != nil {
		return app.Generation{}, internalError(err)
	}
	newGeneration, err := s.storage.PublishState(ctx, converted, publicGeneration(expectedGeneration))
	if err != nil {
		return app.Generation{}, internalError(err)
	}
	result, err := app.GenerationFromString(newGeneration.String())
	if err != nil {
		return app.Generation{}, internalError(err)
	}
	return result, nil
}

type localStorage struct{ storage app.Storage }

func (s *localStorage) CreateRaw(ctx context.Context, name string, contents []byte) (DocumentRef, error) {
	ref, err := s.storage.CreateRaw(ctx, name, contents)
	return publicDocumentRef(ref), publicError(err)
}

func (s *localStorage) ReadDocument(ctx context.Context, ref DocumentRef) ([]byte, error) {
	data, err := s.storage.ReadDocument(ctx, internalDocumentRef(ref))
	return data, publicError(err)
}

func (s *localStorage) ListMarkdownDocuments(ctx context.Context, kind DocumentKind) ([]DocumentRef, error) {
	refs, err := s.storage.ListMarkdownDocuments(ctx, internaldomain.DocumentKind(kind))
	if err != nil {
		return nil, publicError(err)
	}
	result := make([]DocumentRef, len(refs))
	for index, ref := range refs {
		result[index] = publicDocumentRef(ref)
	}
	return result, nil
}

func (s *localStorage) ReplaceSummary(ctx context.Context, ref DocumentRef, contents []byte) error {
	return publicError(s.storage.ReplaceSummary(ctx, internalDocumentRef(ref), contents))
}

func (s *localStorage) DeleteDocument(ctx context.Context, ref DocumentRef) error {
	return publicError(s.storage.DeleteDocument(ctx, internalDocumentRef(ref)))
}

func (s *localStorage) ReadState(ctx context.Context) (State, Generation, error) {
	state, generation, err := s.storage.ReadState(ctx)
	if err != nil {
		return State{}, Generation{}, publicError(err)
	}
	converted, err := publicState(state)
	if err != nil {
		return State{}, Generation{}, publicError(err)
	}
	return converted, publicGeneration(generation), nil
}

func (s *localStorage) PublishState(ctx context.Context, state State, expected Generation) (Generation, error) {
	converted, err := internalState(state)
	if err != nil {
		return Generation{}, publicError(err)
	}
	result, err := s.storage.PublishState(ctx, converted, internalGeneration(expected))
	return publicGeneration(result), publicError(err)
}

type localWorkspace struct {
	workspace app.Workspace
	storage   Storage
}

func (w *localWorkspace) Name() string       { return w.workspace.Name() }
func (w *localWorkspace) RootPath() string   { return w.workspace.RootPath() }
func (w *localWorkspace) TargetPath() string { return w.workspace.TargetPath() }
func (w *localWorkspace) Storage() Storage   { return w.storage }
func (w *localWorkspace) Close() error       { return publicError(w.workspace.Close()) }

type scopedWorkspace struct {
	workspace Workspace
	storage   app.Storage
}

func (w *scopedWorkspace) Name() string         { return w.workspace.Name() }
func (w *scopedWorkspace) RootPath() string     { return w.workspace.RootPath() }
func (w *scopedWorkspace) TargetPath() string   { return w.workspace.TargetPath() }
func (w *scopedWorkspace) Storage() app.Storage { return w.storage }
func (w *scopedWorkspace) Close() error         { return nil }

type fixedWorkspaceOpener struct{ workspace app.Workspace }

func (o fixedWorkspaceOpener) Open(context.Context, string) (app.Workspace, error) {
	if o.workspace == nil {
		return nil, app.RequestError("workspace is not configured")
	}
	return o.workspace, nil
}

type operationLogBridge struct{ log OperationLog }

func (l operationLogBridge) Append(ctx context.Context, operation internaldomain.Operation) error {
	return internalError(l.log.Append(ctx, publicOperation(operation)))
}

func (l operationLogBridge) Read(ctx context.Context, directory string, offset, limit int) (app.OperationPage, error) {
	page, err := l.log.Read(ctx, directory, offset, limit)
	if err != nil {
		return app.OperationPage{}, internalError(err)
	}
	entries := make([]internaldomain.Operation, len(page.Entries))
	for index, operation := range page.Entries {
		entries[index] = internalOperation(operation)
	}
	return app.OperationPage{Directory: page.Directory, Entries: entries, Offset: page.Offset, Limit: page.Limit, NextOffset: page.NextOffset, HasMore: page.HasMore}, nil
}

type localOperationLog struct{ log *loc.OperationLog }

func (l *localOperationLog) Append(ctx context.Context, operation Operation) error {
	return publicError(l.log.Append(ctx, internalOperation(operation)))
}

func (l *localOperationLog) Read(ctx context.Context, directory string, offset, limit int) (OperationPage, error) {
	page, err := l.log.Read(ctx, directory, offset, limit)
	if err != nil {
		return OperationPage{}, publicError(err)
	}
	entries := make([]Operation, len(page.Entries))
	for index, operation := range page.Entries {
		entries[index] = publicOperation(operation)
	}
	return OperationPage{Directory: page.Directory, Entries: entries, Offset: page.Offset, Limit: page.Limit, NextOffset: page.NextOffset, HasMore: page.HasMore}, nil
}

func internalOperationOptions(options OperationOptions) app.OperationOptions {
	var log app.OperationLog
	if options.Log != nil {
		log = operationLogBridge{log: options.Log}
	}
	return app.OperationOptions{Log: log, Actor: options.Actor}
}

func internalGeneration(generation Generation) app.Generation {
	result, _ := app.GenerationFromString(generation.String())
	return result
}

func publicGeneration(generation app.Generation) Generation {
	data, _ := hex.DecodeString(generation.String())
	var digest [sha256.Size]byte
	copy(digest[:], data)
	return Generation{digest: digest}
}

func internalDocumentRef(ref DocumentRef) internaldomain.DocumentRef {
	return internaldomain.DocumentRef{Kind: internaldomain.DocumentKind(ref.Kind), Name: ref.Name}
}

func publicDocumentRef(ref internaldomain.DocumentRef) DocumentRef {
	return DocumentRef{Kind: DocumentKind(ref.Kind), Name: ref.Name}
}

func internalState(state State) (internaldomain.State, error) {
	result := internaldomain.State{Sources: make([]internaldomain.SourceRecord, len(state.Sources))}
	for index, source := range state.Sources {
		result.Sources[index] = internaldomain.SourceRecord{SourceKey: source.SourceKey, Snapshots: make([]internaldomain.RawRecord, len(source.Snapshots))}
		for snapshotIndex, snapshot := range source.Snapshots {
			result.Sources[index].Snapshots[snapshotIndex] = internaldomain.RawRecord{Filename: snapshot.Filename, WrittenAt: snapshot.WrittenAt}
		}
		if source.Summary != nil {
			result.Sources[index].Summary = &internaldomain.SummaryRecord{Filename: source.Summary.Filename, DerivedFrom: source.Summary.DerivedFrom, CreatedAt: source.Summary.CreatedAt, UpdatedAt: source.Summary.UpdatedAt}
		}
	}
	return result, result.Validate()
}

func publicState(state internaldomain.State) (State, error) {
	if err := state.Validate(); err != nil {
		return State{}, err
	}
	result := State{Sources: make([]SourceRecord, len(state.Sources))}
	for index, source := range state.Sources {
		result.Sources[index] = SourceRecord{SourceKey: source.SourceKey, Snapshots: make([]RawRecord, len(source.Snapshots))}
		for snapshotIndex, snapshot := range source.Snapshots {
			result.Sources[index].Snapshots[snapshotIndex] = RawRecord{Filename: snapshot.Filename, WrittenAt: snapshot.WrittenAt}
		}
		if source.Summary != nil {
			result.Sources[index].Summary = &SummaryRecord{Filename: source.Summary.Filename, DerivedFrom: source.Summary.DerivedFrom, CreatedAt: source.Summary.CreatedAt, UpdatedAt: source.Summary.UpdatedAt}
		}
	}
	return result, nil
}

func internalOperation(operation Operation) internaldomain.Operation {
	return internaldomain.Operation{Timestamp: operation.Timestamp, Actor: operation.Actor, Directory: operation.Directory, Command: internaldomain.OperationCommand(operation.Command), Success: operation.Success, Details: operation.Details}
}

func publicOperation(operation internaldomain.Operation) Operation {
	return Operation{Timestamp: operation.Timestamp, Actor: operation.Actor, Directory: operation.Directory, Command: OperationCommand(operation.Command), Success: operation.Success, Details: operation.Details}
}

func publicSnapOutcomes(outcomes []app.SnapOutcome) []SnapOutcome {
	result := make([]SnapOutcome, len(outcomes))
	for index, outcome := range outcomes {
		result[index] = SnapOutcome{SourceKey: outcome.SourceKey, Filename: outcome.Filename, Err: publicError(outcome.Err)}
	}
	return result
}

func publicSynthResult(result app.SynthesisResult) SynthResult {
	converted := SynthResult{SummariesWritten: result.SummariesWritten, SummariesSkipped: result.SummariesSkipped, Metrics: Metrics{Turns: result.Metrics.Turns, ToolCalls: result.Metrics.ToolCalls, Duration: result.Metrics.Duration}}
	if result.Metrics.Usage != nil {
		converted.Metrics.Usage = &Usage{PromptTokens: result.Metrics.Usage.PromptTokens, CompletionTokens: result.Metrics.Usage.CompletionTokens, TotalTokens: result.Metrics.Usage.TotalTokens}
	}
	return converted
}

func publicError(err error) error {
	if err == nil {
		return nil
	}
	var categorized *internalerrors.Error
	if !stderrors.As(err, &categorized) {
		return err
	}
	cause := categorized.Cause
	if stderrors.Is(cause, internalerrors.ErrAlreadyExists) {
		cause = ErrAlreadyExists
	}
	return &Error{Category: ErrorCategory(categorized.Category), Detail: categorized.Detail, Status: categorized.Status, RequestID: categorized.RequestID, Cause: cause}
}

func internalError(err error) error {
	if err == nil {
		return nil
	}
	if stderrors.Is(err, ErrAlreadyExists) {
		return app.ErrAlreadyExists
	}
	var categorized *Error
	if !stderrors.As(err, &categorized) {
		return err
	}
	cause := categorized.Cause
	if stderrors.Is(cause, ErrAlreadyExists) {
		cause = app.ErrAlreadyExists
	}
	converted := app.NewError(app.ErrorCategory(categorized.Category), categorized.Detail)
	converted.Status = categorized.Status
	converted.RequestID = categorized.RequestID
	converted.Cause = cause
	return converted
}
