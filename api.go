package bo

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	stderrors "errors"
	"fmt"
	"net/http"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	app "github.com/skillicinski/bo/internal/application"
	internaldomain "github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	deepseek "github.com/skillicinski/bo/internal/provider/deepseek"
	"github.com/skillicinski/bo/internal/source"
	filesource "github.com/skillicinski/bo/internal/source/file"
	urlsource "github.com/skillicinski/bo/internal/source/url"
	loc "github.com/skillicinski/bo/internal/storage/local"
)

// ErrorKind is the stable failure classification returned by bo workflows.
type ErrorKind string

const (
	ErrorKindRequest           ErrorKind = "request"
	ErrorKindValidation        ErrorKind = "validation"
	ErrorKindSource            ErrorKind = "source"
	ErrorKindFilesystem        ErrorKind = "filesystem"
	ErrorKindMissingResource   ErrorKind = "missing_resource"
	ErrorKindConflict          ErrorKind = "conflict"
	ErrorKindAlreadyExists     ErrorKind = "already_exists"
	ErrorKindProviderTransport ErrorKind = "provider_transport"
	ErrorKindProviderRejected  ErrorKind = "provider_rejected"
	ErrorKindProviderMalformed ErrorKind = "provider_malformed"
	ErrorKindCanceled          ErrorKind = "canceled"
	ErrorKindDeadline          ErrorKind = "deadline"
)

// Error is the stable public failure contract for all bo workflows.
type Error struct {
	Kind      ErrorKind `json:"kind"`
	Detail    string    `json:"detail,omitempty"`
	Retryable bool      `json:"retryable"`
	Cause     error     `json:"-"`
}

func (e *Error) Error() string {
	if e == nil {
		return "<nil>"
	}
	if e.Detail == "" {
		return string(e.Kind)
	}
	return fmt.Sprintf("%s: %s", e.Kind, e.Detail)
}

func (e *Error) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Cause
}

func (e *Error) Is(target error) bool {
	return target == ErrAlreadyExists && e != nil && e.Kind == ErrorKindAlreadyExists
}

func NewError(kind ErrorKind, detail string) *Error {
	return &Error{Kind: kind, Detail: detail}
}

func WrapError(kind ErrorKind, detail string, cause error) *Error {
	return &Error{Kind: kind, Detail: detail, Cause: cause}
}

var ErrAlreadyExists = stderrors.New("document already exists")

func IsKind(err error, kind ErrorKind) bool {
	var categorized *Error
	if stderrors.As(err, &categorized) {
		return categorized.Kind == kind
	}
	var internalCategorized *internalerrors.Error
	return stderrors.As(err, &internalCategorized) && ErrorKind(internalCategorized.Kind) == kind
}

func IsAlreadyExists(err error) bool {
	return IsKind(err, ErrorKindAlreadyExists) || stderrors.Is(err, ErrAlreadyExists)
}

type DocumentKind string

const (
	DocumentKindRaw          DocumentKind = "raw"
	DocumentKindSummary      DocumentKind = "summary"
	DocumentKindDistillation DocumentKind = "distillation"
)

type DocumentRef struct {
	Kind DocumentKind
	Name string
}

func RawRef(name string) DocumentRef     { return DocumentRef{Kind: DocumentKindRaw, Name: name} }
func SummaryRef(name string) DocumentRef { return DocumentRef{Kind: DocumentKindSummary, Name: name} }
func DistillationRef(name string) DocumentRef {
	return DocumentRef{Kind: DocumentKindDistillation, Name: name}
}

type Revision struct{ digest [sha256.Size]byte }

func NewRevision(data []byte) Revision { return Revision{digest: sha256.Sum256(data)} }

func (r Revision) Equal(other Revision) bool { return r == other }
func (r Revision) IsZero() bool              { return r == Revision{} }
func (r Revision) String() string            { return hex.EncodeToString(r.digest[:]) }

func (r Revision) MarshalJSON() ([]byte, error) { return json.Marshal(r.String()) }

func (r *Revision) UnmarshalJSON(data []byte) error {
	var value string
	if err := json.Unmarshal(data, &value); err != nil {
		return err
	}
	decoded, err := hex.DecodeString(value)
	if err != nil || len(decoded) != sha256.Size {
		return fmt.Errorf("invalid revision")
	}
	copy(r.digest[:], decoded)
	return nil
}

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

type DistillationInput struct {
	SourceKey     string       `json:"source_key"`
	Kind          DocumentKind `json:"kind"`
	Filename      string       `json:"filename"`
	ContentDigest string       `json:"content_digest"`
}

type DistillationRecord struct {
	Filename          string              `json:"filename"`
	Topic             string              `json:"topic,omitempty"`
	Kind              DocumentKind        `json:"kind"`
	CreatedAt         time.Time           `json:"created_at"`
	UpdatedAt         time.Time           `json:"updated_at"`
	ContentDigest     string              `json:"content_digest,omitempty"`
	ContentSize       *int64              `json:"content_size,omitempty"`
	ContentModifiedAt string              `json:"content_modified_at,omitempty"`
	DerivedFrom       []DistillationInput `json:"derived_from"`
}

type SourceRecord struct {
	SourceKey string         `json:"source_key"`
	Snapshots []RawRecord    `json:"snapshots"`
	Summary   *SummaryRecord `json:"summary,omitempty"`
}

type State struct {
	Sources               []SourceRecord       `json:"sources"`
	DistillationDocuments []DistillationRecord `json:"distillation_documents,omitempty"`
}

func (s State) SnapshotCount() int {
	count := 0
	for _, source := range s.Sources {
		count += len(source.Snapshots)
	}
	return count
}

// Workspace is a caller-scoped workspace. bo does not select or mutate the
// tenant, authentication, routing, or storage configuration for a workspace.
type Workspace interface {
	Name() string
	ListDocuments(context.Context, DocumentKind) ([]DocumentRef, error)
	ReadDocument(context.Context, DocumentRef) ([]byte, error)
	ReadState(context.Context) (State, Revision, error)
	WorkspaceEvents
	CommitSnapshot(context.Context, SnapshotCommit, Revision) (State, Revision, error)
	CommitSummary(context.Context, SummaryCommit, Revision) (State, Revision, error)
	CommitDistillation(context.Context, DistillationCommit, Revision) (State, Revision, error)
	Close() error
}

// WorkspaceEvents is the durable event contract for one workspace.
type WorkspaceEvents interface {
	ReadEvents(context.Context, int, int) (OperationPage, error)
	// ReadRecentEvents returns at most limit of the most recent events
	// in chronological order, oldest first.
	ReadRecentEvents(context.Context, int) ([]Operation, error)
	CommitEvent(context.Context, Operation) error
}

type SnapshotCommit struct {
	SourceKey string
	Filename  string
	WrittenAt time.Time
	Contents  []byte
	Event     Operation
}

type SummaryCommit struct {
	SourceKey    string
	Filename     string
	DerivedFrom  string
	RawWrittenAt time.Time
	CreatedAt    time.Time
	UpdatedAt    time.Time
	Contents     []byte
	Event        Operation
}

type DistillationCommit struct {
	Kind     DocumentKind
	Filename string
	Topic    string
	// Update replaces an existing distillation record with the same filename.
	Update      bool
	CreatedAt   time.Time
	UpdatedAt   time.Time
	DerivedFrom []DistillationInput
	Contents    []byte
	Event       Operation
}

type WorkspaceCreator interface {
	Create(context.Context, string, Operation) (string, error)
}

type OperationCommand string

const (
	CommandSeed              OperationCommand = "seed"
	CommandSnap              OperationCommand = "snap"
	CommandState             OperationCommand = "state"
	CommandSynth             OperationCommand = "synth"
	CommandDistill           OperationCommand = "distill"
	CommandWriteSummary      OperationCommand = "write_summary"
	CommandWriteDistillation OperationCommand = "write_distillation"
)

type SynthMode string

const (
	SynthModeDefault   SynthMode = ""
	SynthModeSummarize SynthMode = "summarize"
	SynthModeDistill   SynthMode = "distill"
)

type OperationOutcome string

const (
	OutcomeCommitted OperationOutcome = "committed"
	OutcomeFailed    OperationOutcome = "failed"
)

type SourceIdentity struct {
	SourceKey string `json:"source_key"`
}

type DocumentIdentity struct {
	Kind     DocumentKind `json:"kind"`
	Filename string       `json:"filename"`
}

type OperationProvenance struct {
	DerivedFrom  *DocumentIdentity `json:"derived_from,omitempty"`
	RawWrittenAt *time.Time        `json:"raw_written_at,omitempty"`
}

type OperationError struct {
	Kind      string `json:"kind"`
	Retryable bool   `json:"retryable"`
}

type TokenUsage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

type OperationMetrics struct {
	Turns               int           `json:"turns"`
	ToolCalls           int           `json:"tool_calls"`
	Duration            time.Duration `json:"duration"`
	SummariesWritten    int           `json:"summaries_written"`
	SummariesSkipped    int           `json:"summaries_skipped"`
	DistillationWritten int           `json:"distillation_written,omitempty"`
	DistillationSkipped int           `json:"distillation_skipped,omitempty"`
	Usage               *TokenUsage   `json:"usage,omitempty"`
}

type Operation struct {
	OperationID string               `json:"operation_id"`
	Attempt     int                  `json:"attempt"`
	Timestamp   string               `json:"timestamp"`
	Actor       string               `json:"actor"`
	Command     OperationCommand     `json:"command"`
	Outcome     OperationOutcome     `json:"outcome"`
	Source      *SourceIdentity      `json:"source,omitempty"`
	Document    *DocumentIdentity    `json:"document,omitempty"`
	Provenance  *OperationProvenance `json:"provenance,omitempty"`
	Error       *OperationError      `json:"error,omitempty"`
	Metrics     *OperationMetrics    `json:"metrics,omitempty"`
}

type OperationPage struct {
	Directory  string      `json:"directory"`
	Entries    []Operation `json:"entries"`
	Offset     int         `json:"offset"`
	Limit      int         `json:"limit"`
	NextOffset int         `json:"next_offset"`
	HasMore    bool        `json:"has_more"`
}

type OperationOptions struct {
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
	Workspace    Workspace
	Sources      []string
	SourceConfig *SnapSourceConfig
	Operations   OperationOptions
}

// SnapSourceConfig controls the source adapters used by Snap. A nil config
// keeps the CLI defaults: local Markdown files are enabled and bo uses a
// 30-second HTTP client. When HTTPClient is set, its transport owns DNS,
// redirect, and private-network policy; bo does not apply another policy.
type SnapSourceConfig struct {
	// AllowLocalFiles enables local Markdown reads. It defaults to false when a
	// config is provided, and nil SnapSourceConfig enables local files.
	AllowLocalFiles bool
	// HTTPClient supplies the transport and redirect policy for URL sources.
	HTTPClient *http.Client
}

type SnapOutcome struct {
	SourceKey string
	Filename  string
	Err       error
}

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
	State    State    `json:"state"`
	Revision Revision `json:"revision"`
}

type SynthesisOptions struct {
	MaxTurns           int
	MaxToolCalls       int
	MaxToolOutputBytes int
	MaxResponseTokens  int
	// RuntimeTimeoutSeconds limits each agent runtime. The caller context controls the complete workflow.
	RuntimeTimeoutSeconds int
}

func DefaultSynthesisOptions() SynthesisOptions {
	defaults := app.DefaultSynthesisOptions()
	return SynthesisOptions{
		MaxTurns: defaults.MaxTurns, MaxToolCalls: defaults.MaxToolCalls,
		MaxToolOutputBytes: defaults.MaxToolOutputBytes, MaxResponseTokens: defaults.MaxResponseTokens,
		RuntimeTimeoutSeconds: defaults.RuntimeTimeoutSeconds,
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

type Metrics struct {
	Turns     int           `json:"turns"`
	ToolCalls int           `json:"tool_calls"`
	Usage     *TokenUsage   `json:"usage,omitempty"`
	Duration  time.Duration `json:"duration"`
}

type SynthRequest struct {
	Workspace  Workspace
	Provider   Provider
	Mode       SynthMode
	Options    SynthesisOptions
	Operations OperationOptions
}

type OperationReport struct {
	Operation OperationCommand   `json:"operation"`
	Documents []DocumentIdentity `json:"documents"`
}

type SynthResult struct {
	SummariesWritten    int               `json:"summaries_written"`
	SummariesSkipped    int               `json:"summaries_skipped"`
	DistillationWritten int               `json:"distillation_written"`
	DistillationSkipped int               `json:"distillation_skipped"`
	Report              []OperationReport `json:"report,omitempty"`
	Metrics             Metrics           `json:"metrics"`
}

type LocalManager struct {
	manager *loc.Manager
}

func NewLocalManager(home string) *LocalManager {
	return &LocalManager{manager: loc.NewManager(home)}
}

func (m *LocalManager) Create(ctx context.Context, name string, event Operation) (string, error) {
	if m == nil || m.manager == nil {
		return "", NewError(ErrorKindRequest, "local workspace manager is not configured")
	}
	created, err := m.manager.Create(ctx, name, internalOperation(event))
	return created, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "creating workspace"))
}

func (m *LocalManager) Open(ctx context.Context, name string) (Workspace, error) {
	if m == nil || m.manager == nil {
		return nil, NewError(ErrorKindRequest, "local workspace manager is not configured")
	}
	workspace, err := m.manager.Open(ctx, name)
	if err != nil {
		return nil, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "opening workspace"))
	}
	if workspace == nil {
		return nil, NewError(ErrorKindRequest, "local workspace manager returned no workspace")
	}
	return &localWorkspace{workspace: workspace}, nil
}

func Seed(ctx context.Context, request SeedRequest) (SeedResult, error) {
	created, err := app.Seed(ctx, internalWorkspaceCreator(request.Creator), request.Name, internalOperationOptions(request.Operations))
	return SeedResult{Name: created}, publicError(err)
}

type workspaceCreatorBridge struct{ creator WorkspaceCreator }

func (b workspaceCreatorBridge) Create(ctx context.Context, name string, event app.Operation) (string, error) {
	created, err := b.creator.Create(ctx, name, publicOperation(event))
	return created, internalError(err)
}

func internalWorkspaceCreator(creator WorkspaceCreator) app.WorkspaceCreator {
	if creator == nil {
		return nil
	}
	return workspaceCreatorBridge{creator: creator}
}

func Snap(ctx context.Context, request SnapRequest) (SnapResult, error) {
	result := SnapResult{}
	if request.Workspace == nil {
		return result, NewError(ErrorKindRequest, "workspace is not configured")
	}
	outcomes, err := app.Snap(ctx, &publicWorkspace{workspace: request.Workspace}, sourceWorkflow(request.SourceConfig), request.Sources, internalOperationOptions(request.Operations))
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

func sourceWorkflow(config *SnapSourceConfig) *source.Workflow {
	client := &http.Client{Timeout: 30 * time.Second}
	allowLocalFiles := config == nil
	if config != nil {
		allowLocalFiles = config.AllowLocalFiles
		if config.HTTPClient != nil {
			client = config.HTTPClient
		}
	}
	transports := []source.Transport{urlsource.NewTransport()}
	plugins := map[source.OriginType]source.Plugin{
		source.OriginHTML:    urlsource.NewHTML(client),
		source.OriginYouTube: urlsource.NewYouTube(client),
	}
	if allowLocalFiles {
		transports = append(transports, filesource.NewTransport())
		plugins[source.OriginMarkdown] = filesource.NewMarkdownPlugin()
	}
	return source.NewWorkflow(transports, plugins)
}

func ReadState(ctx context.Context, request StateRequest) (StateResult, error) {
	if request.Workspace == nil {
		return StateResult{}, NewError(ErrorKindRequest, "workspace is not configured")
	}
	state, revision, err := app.ReadState(ctx, &publicWorkspace{workspace: request.Workspace}, internalOperationOptions(request.Operations))
	if err != nil {
		return StateResult{}, publicError(err)
	}
	converted, err := publicState(state)
	if err != nil {
		return StateResult{}, publicError(err)
	}
	return StateResult{State: converted, Revision: publicRevision(revision)}, nil
}

func Synth(ctx context.Context, request SynthRequest) (SynthResult, error) {
	if request.Workspace == nil {
		return SynthResult{}, NewError(ErrorKindRequest, "workspace is not configured")
	}
	workspace := &publicWorkspace{workspace: request.Workspace}
	result, err := app.Synth(ctx, workspace, request.Provider.completion, app.SynthesisOptions{
		MaxTurns: request.Options.MaxTurns, MaxToolCalls: request.Options.MaxToolCalls,
		MaxToolOutputBytes: request.Options.MaxToolOutputBytes, MaxResponseTokens: request.Options.MaxResponseTokens,
		RuntimeTimeoutSeconds: request.Options.RuntimeTimeoutSeconds,
	}, app.SynthMode(request.Mode), internalOperationOptions(request.Operations))
	return publicSynthResult(result), publicError(err)
}

type localWorkspace struct {
	workspace *loc.Store
}

func (w *localWorkspace) Name() string { return w.workspace.Name() }

func (w *localWorkspace) ListDocuments(ctx context.Context, kind DocumentKind) ([]DocumentRef, error) {
	refs, err := w.workspace.ListDocuments(ctx, internaldomain.DocumentKind(kind))
	if err != nil {
		return nil, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "listing documents"))
	}
	return publicDocumentRefs(refs), nil
}

func (w *localWorkspace) ReadDocument(ctx context.Context, ref DocumentRef) ([]byte, error) {
	data, err := w.workspace.ReadDocument(ctx, internalDocumentRef(ref))
	return data, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "reading document"))
}

func (w *localWorkspace) ReadState(ctx context.Context) (State, Revision, error) {
	state, revision, err := w.workspace.ReadState(ctx)
	if err != nil {
		return State{}, Revision{}, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "reading workspace state"))
	}
	converted, err := publicState(state)
	if err != nil {
		return State{}, Revision{}, publicError(err)
	}
	return converted, publicRevision(revision), nil
}

func (w *localWorkspace) ReadEvents(ctx context.Context, offset, limit int) (OperationPage, error) {
	page, err := w.workspace.ReadEvents(ctx, offset, limit)
	if err != nil {
		return OperationPage{}, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "reading operation events"))
	}
	return publicOperationPage(page), nil
}

func (w *localWorkspace) ReadRecentEvents(ctx context.Context, limit int) ([]Operation, error) {
	events, err := w.workspace.ReadRecentEvents(ctx, limit)
	if err != nil {
		return nil, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "reading recent operation events"))
	}
	result := make([]Operation, len(events))
	for index, event := range events {
		result[index] = publicOperation(event)
	}
	return result, nil
}

func (w *localWorkspace) CommitEvent(ctx context.Context, event Operation) error {
	err := w.workspace.CommitEvent(ctx, internalOperation(event))
	if err != nil {
		return publicError(internalErrorAs(err, internalerrors.KindFilesystem, "committing operation event"))
	}
	return nil
}

func (w *localWorkspace) CommitSnapshot(ctx context.Context, commit SnapshotCommit, expected Revision) (State, Revision, error) {
	state, revision, err := w.workspace.CommitSnapshot(ctx, internalSnapshotCommit(commit), internalRevision(expected))
	if err != nil {
		return State{}, Revision{}, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "committing snapshot"))
	}
	converted, err := publicState(state)
	if err != nil {
		return State{}, Revision{}, publicError(err)
	}
	return converted, publicRevision(revision), nil
}

func (w *localWorkspace) CommitSummary(ctx context.Context, commit SummaryCommit, expected Revision) (State, Revision, error) {
	state, revision, err := w.workspace.CommitSummary(ctx, internalSummaryCommit(commit), internalRevision(expected))
	if err != nil {
		return State{}, Revision{}, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "committing summary"))
	}
	converted, err := publicState(state)
	if err != nil {
		return State{}, Revision{}, publicError(err)
	}
	return converted, publicRevision(revision), nil
}

func (w *localWorkspace) CommitDistillation(ctx context.Context, commit DistillationCommit, expected Revision) (State, Revision, error) {
	state, revision, err := w.workspace.CommitDistillation(ctx, internalDistillationCommit(commit), internalRevision(expected))
	if err != nil {
		return State{}, Revision{}, publicError(internalErrorAs(err, internalerrors.KindFilesystem, "committing distillation document"))
	}
	converted, err := publicState(state)
	if err != nil {
		return State{}, Revision{}, publicError(err)
	}
	return converted, publicRevision(revision), nil
}

func (w *localWorkspace) Close() error {
	return publicError(internalErrorAs(w.workspace.Close(), internalerrors.KindFilesystem, "closing workspace"))
}

type publicWorkspace struct {
	workspace Workspace
}

func (w *publicWorkspace) Name() string { return w.workspace.Name() }

func (w *publicWorkspace) ListDocuments(ctx context.Context, kind internaldomain.DocumentKind) ([]internaldomain.DocumentRef, error) {
	refs, err := w.workspace.ListDocuments(ctx, DocumentKind(kind))
	if err != nil {
		return nil, internalErrorAs(err, internalerrors.KindFilesystem, "listing documents")
	}
	result := make([]internaldomain.DocumentRef, len(refs))
	for index, ref := range refs {
		result[index] = internalDocumentRef(ref)
	}
	return result, nil
}

func (w *publicWorkspace) ReadDocument(ctx context.Context, ref internaldomain.DocumentRef) ([]byte, error) {
	data, err := w.workspace.ReadDocument(ctx, publicDocumentRef(ref))
	return data, internalErrorAs(err, internalerrors.KindFilesystem, "reading document")
}

func (w *publicWorkspace) ReadState(ctx context.Context) (internaldomain.State, app.Revision, error) {
	state, revision, err := w.workspace.ReadState(ctx)
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalErrorAs(err, internalerrors.KindFilesystem, "reading workspace state")
	}
	converted, err := internalState(state)
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	internalRevision, err := app.RevisionFromString(revision.String())
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	return converted, internalRevision, nil
}

func (w *publicWorkspace) ReadEvents(ctx context.Context, offset, limit int) (app.OperationPage, error) {
	if err := app.ValidateOperationPageRequest(offset, limit); err != nil {
		return app.OperationPage{}, err
	}
	page, err := w.workspace.ReadEvents(ctx, offset, limit)
	if err != nil {
		return app.OperationPage{}, internalErrorAs(err, internalerrors.KindFilesystem, "reading operation events")
	}
	entries := make([]internaldomain.Operation, len(page.Entries))
	for index, event := range page.Entries {
		entries[index] = internalOperation(event)
	}
	converted := app.OperationPage{Directory: page.Directory, Entries: entries, Offset: page.Offset, Limit: page.Limit, NextOffset: page.NextOffset, HasMore: page.HasMore}
	if err := app.ValidateOperationPage(converted, offset, limit); err != nil {
		return app.OperationPage{}, err
	}
	return converted, nil
}

func (w *publicWorkspace) ReadRecentEvents(ctx context.Context, limit int) ([]app.Operation, error) {
	events, err := w.workspace.ReadRecentEvents(ctx, limit)
	if err != nil {
		return nil, internalErrorAs(err, internalerrors.KindFilesystem, "reading recent operation events")
	}
	result := make([]app.Operation, len(events))
	for index, event := range events {
		result[index] = internalOperation(event)
	}
	return result, nil
}

func (w *publicWorkspace) CommitEvent(ctx context.Context, event app.Operation) error {
	err := w.workspace.CommitEvent(ctx, publicOperation(event))
	if err != nil {
		return internalErrorAs(err, internalerrors.KindFilesystem, "committing operation event")
	}
	return nil
}

func (w *publicWorkspace) CommitSnapshot(ctx context.Context, commit app.SnapshotCommit, expected app.Revision) (internaldomain.State, app.Revision, error) {
	state, revision, err := w.workspace.CommitSnapshot(ctx, publicSnapshotCommit(commit), publicRevision(expected))
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalErrorAs(err, internalerrors.KindFilesystem, "committing snapshot")
	}
	converted, err := internalState(state)
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	internalRevision, err := app.RevisionFromString(revision.String())
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	return converted, internalRevision, nil
}

func (w *publicWorkspace) CommitSummary(ctx context.Context, commit app.SummaryCommit, expected app.Revision) (internaldomain.State, app.Revision, error) {
	state, revision, err := w.workspace.CommitSummary(ctx, publicSummaryCommit(commit), publicRevision(expected))
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalErrorAs(err, internalerrors.KindFilesystem, "committing summary")
	}
	converted, err := internalState(state)
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	internalRevision, err := app.RevisionFromString(revision.String())
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	return converted, internalRevision, nil
}

func (w *publicWorkspace) CommitDistillation(ctx context.Context, commit app.DistillationCommit, expected app.Revision) (internaldomain.State, app.Revision, error) {
	state, revision, err := w.workspace.CommitDistillation(ctx, publicDistillationCommit(commit), publicRevision(expected))
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalErrorAs(err, internalerrors.KindFilesystem, "committing distillation document")
	}
	converted, err := internalState(state)
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	internalRevision, err := app.RevisionFromString(revision.String())
	if err != nil {
		return internaldomain.State{}, app.Revision{}, internalError(err)
	}
	return converted, internalRevision, nil
}

func internalOperationOptions(options OperationOptions) app.OperationOptions {
	return app.OperationOptions{Actor: options.Actor}
}

func internalRevision(revision Revision) app.Revision {
	result, _ := app.RevisionFromString(revision.String())
	return result
}

func publicRevision(revision app.Revision) Revision {
	data, _ := hex.DecodeString(revision.String())
	var digest [sha256.Size]byte
	copy(digest[:], data)
	return Revision{digest: digest}
}

func internalDocumentRef(ref DocumentRef) internaldomain.DocumentRef {
	return internaldomain.DocumentRef{Kind: internaldomain.DocumentKind(ref.Kind), Name: ref.Name}
}

func publicDocumentRef(ref internaldomain.DocumentRef) DocumentRef {
	return DocumentRef{Kind: DocumentKind(ref.Kind), Name: ref.Name}
}

func publicDocumentRefs(refs []internaldomain.DocumentRef) []DocumentRef {
	result := make([]DocumentRef, len(refs))
	for index, ref := range refs {
		result[index] = publicDocumentRef(ref)
	}
	return result
}

func internalSnapshotCommit(commit SnapshotCommit) app.SnapshotCommit {
	return app.SnapshotCommit{SourceKey: commit.SourceKey, Filename: commit.Filename, WrittenAt: commit.WrittenAt, Contents: commit.Contents, Event: internalOperation(commit.Event)}
}

func publicSnapshotCommit(commit app.SnapshotCommit) SnapshotCommit {
	return SnapshotCommit{SourceKey: commit.SourceKey, Filename: commit.Filename, WrittenAt: commit.WrittenAt, Contents: commit.Contents, Event: publicOperation(commit.Event)}
}

func internalSummaryCommit(commit SummaryCommit) app.SummaryCommit {
	return app.SummaryCommit{SourceKey: commit.SourceKey, Filename: commit.Filename, DerivedFrom: commit.DerivedFrom, RawWrittenAt: commit.RawWrittenAt, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt, Contents: commit.Contents, Event: internalOperation(commit.Event)}
}

func internalDistillationCommit(commit DistillationCommit) app.DistillationCommit {
	inputs := make([]internaldomain.DistillationInput, len(commit.DerivedFrom))
	for index, input := range commit.DerivedFrom {
		inputs[index] = internaldomain.DistillationInput{SourceKey: input.SourceKey, Kind: internaldomain.DocumentKind(input.Kind), Filename: input.Filename, ContentDigest: input.ContentDigest}
	}
	return app.DistillationCommit{Kind: internaldomain.DocumentKind(commit.Kind), Filename: commit.Filename, Topic: commit.Topic, Update: commit.Update, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt, DerivedFrom: inputs, Contents: commit.Contents, Event: internalOperation(commit.Event)}
}

func publicSummaryCommit(commit app.SummaryCommit) SummaryCommit {
	return SummaryCommit{SourceKey: commit.SourceKey, Filename: commit.Filename, DerivedFrom: commit.DerivedFrom, RawWrittenAt: commit.RawWrittenAt, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt, Contents: commit.Contents, Event: publicOperation(commit.Event)}
}

func publicDistillationCommit(commit app.DistillationCommit) DistillationCommit {
	inputs := make([]DistillationInput, len(commit.DerivedFrom))
	for index, input := range commit.DerivedFrom {
		inputs[index] = DistillationInput{SourceKey: input.SourceKey, Kind: DocumentKind(input.Kind), Filename: input.Filename, ContentDigest: input.ContentDigest}
	}
	return DistillationCommit{Kind: DocumentKind(commit.Kind), Filename: commit.Filename, Topic: commit.Topic, Update: commit.Update, CreatedAt: commit.CreatedAt, UpdatedAt: commit.UpdatedAt, DerivedFrom: inputs, Contents: commit.Contents, Event: publicOperation(commit.Event)}
}

func internalState(state State) (internaldomain.State, error) {
	result := internaldomain.State{Sources: make([]internaldomain.SourceRecord, len(state.Sources)), DistillationDocuments: make([]internaldomain.DistillationRecord, len(state.DistillationDocuments))}
	for index, source := range state.Sources {
		result.Sources[index] = internaldomain.SourceRecord{SourceKey: source.SourceKey, Snapshots: make([]internaldomain.RawRecord, len(source.Snapshots))}
		for snapshotIndex, snapshot := range source.Snapshots {
			result.Sources[index].Snapshots[snapshotIndex] = internaldomain.RawRecord{Filename: snapshot.Filename, WrittenAt: snapshot.WrittenAt}
		}
		if source.Summary != nil {
			result.Sources[index].Summary = &internaldomain.SummaryRecord{Filename: source.Summary.Filename, DerivedFrom: source.Summary.DerivedFrom, CreatedAt: source.Summary.CreatedAt, UpdatedAt: source.Summary.UpdatedAt}
		}
	}
	for index, record := range state.DistillationDocuments {
		result.DistillationDocuments[index] = internalDistillationRecord(record)
	}
	return result, result.Validate()
}

func publicState(state internaldomain.State) (State, error) {
	if err := state.Validate(); err != nil {
		return State{}, err
	}
	result := State{Sources: make([]SourceRecord, len(state.Sources)), DistillationDocuments: make([]DistillationRecord, len(state.DistillationDocuments))}
	for index, source := range state.Sources {
		result.Sources[index] = SourceRecord{SourceKey: source.SourceKey, Snapshots: make([]RawRecord, len(source.Snapshots))}
		for snapshotIndex, snapshot := range source.Snapshots {
			result.Sources[index].Snapshots[snapshotIndex] = RawRecord{Filename: snapshot.Filename, WrittenAt: snapshot.WrittenAt}
		}
		if source.Summary != nil {
			result.Sources[index].Summary = &SummaryRecord{Filename: source.Summary.Filename, DerivedFrom: source.Summary.DerivedFrom, CreatedAt: source.Summary.CreatedAt, UpdatedAt: source.Summary.UpdatedAt}
		}
	}
	for index, record := range state.DistillationDocuments {
		result.DistillationDocuments[index] = publicDistillationRecord(record)
	}
	return result, nil
}

func internalDistillationRecord(record DistillationRecord) internaldomain.DistillationRecord {
	inputs := make([]internaldomain.DistillationInput, len(record.DerivedFrom))
	for index, input := range record.DerivedFrom {
		inputs[index] = internaldomain.DistillationInput{SourceKey: input.SourceKey, Kind: internaldomain.DocumentKind(input.Kind), Filename: input.Filename, ContentDigest: input.ContentDigest}
	}
	var size *int64
	if record.ContentSize != nil {
		value := *record.ContentSize
		size = &value
	}
	return internaldomain.DistillationRecord{Filename: record.Filename, Topic: record.Topic, Kind: internaldomain.DocumentKind(record.Kind), CreatedAt: record.CreatedAt, UpdatedAt: record.UpdatedAt, ContentDigest: record.ContentDigest, ContentSize: size, ContentModifiedAt: record.ContentModifiedAt, DerivedFrom: inputs}
}

func publicDistillationRecord(record internaldomain.DistillationRecord) DistillationRecord {
	inputs := make([]DistillationInput, len(record.DerivedFrom))
	for index, input := range record.DerivedFrom {
		inputs[index] = DistillationInput{SourceKey: input.SourceKey, Kind: DocumentKind(input.Kind), Filename: input.Filename, ContentDigest: input.ContentDigest}
	}
	var size *int64
	if record.ContentSize != nil {
		value := *record.ContentSize
		size = &value
	}
	return DistillationRecord{Filename: record.Filename, Topic: record.Topic, Kind: DocumentKind(record.Kind), CreatedAt: record.CreatedAt, UpdatedAt: record.UpdatedAt, ContentDigest: record.ContentDigest, ContentSize: size, ContentModifiedAt: record.ContentModifiedAt, DerivedFrom: inputs}
}

func internalOperation(operation Operation) internaldomain.Operation {
	result := internaldomain.Operation{
		OperationID: operation.OperationID, Attempt: operation.Attempt, Timestamp: operation.Timestamp,
		Actor: operation.Actor, Command: internaldomain.OperationCommand(operation.Command),
		Outcome: internaldomain.OperationOutcome(operation.Outcome),
	}
	if operation.Source != nil {
		result.Source = &internaldomain.SourceIdentity{SourceKey: operation.Source.SourceKey}
	}
	if operation.Document != nil {
		result.Document = &internaldomain.DocumentIdentity{Kind: internaldomain.DocumentKind(operation.Document.Kind), Filename: operation.Document.Filename}
	}
	if operation.Provenance != nil {
		result.Provenance = &internaldomain.OperationProvenance{RawWrittenAt: operation.Provenance.RawWrittenAt}
		if operation.Provenance.DerivedFrom != nil {
			result.Provenance.DerivedFrom = &internaldomain.DocumentIdentity{Kind: internaldomain.DocumentKind(operation.Provenance.DerivedFrom.Kind), Filename: operation.Provenance.DerivedFrom.Filename}
		}
	}
	if operation.Error != nil {
		result.Error = &internaldomain.OperationError{Kind: operation.Error.Kind, Retryable: operation.Error.Retryable}
	}
	if operation.Metrics != nil {
		result.Metrics = &internaldomain.OperationMetrics{
			Turns: operation.Metrics.Turns, ToolCalls: operation.Metrics.ToolCalls, Duration: operation.Metrics.Duration,
			SummariesWritten: operation.Metrics.SummariesWritten, SummariesSkipped: operation.Metrics.SummariesSkipped,
			DistillationWritten: operation.Metrics.DistillationWritten, DistillationSkipped: operation.Metrics.DistillationSkipped,
		}
		if operation.Metrics.Usage != nil {
			result.Metrics.Usage = &internaldomain.TokenUsage{PromptTokens: operation.Metrics.Usage.PromptTokens, CompletionTokens: operation.Metrics.Usage.CompletionTokens, TotalTokens: operation.Metrics.Usage.TotalTokens}
		}
	}
	return result
}

func publicOperation(operation internaldomain.Operation) Operation {
	result := Operation{
		OperationID: operation.OperationID, Attempt: operation.Attempt, Timestamp: operation.Timestamp,
		Actor: operation.Actor, Command: OperationCommand(operation.Command),
		Outcome: OperationOutcome(operation.Outcome),
	}
	if operation.Source != nil {
		result.Source = &SourceIdentity{SourceKey: operation.Source.SourceKey}
	}
	if operation.Document != nil {
		result.Document = &DocumentIdentity{Kind: DocumentKind(operation.Document.Kind), Filename: operation.Document.Filename}
	}
	if operation.Provenance != nil {
		result.Provenance = &OperationProvenance{RawWrittenAt: operation.Provenance.RawWrittenAt}
		if operation.Provenance.DerivedFrom != nil {
			result.Provenance.DerivedFrom = &DocumentIdentity{Kind: DocumentKind(operation.Provenance.DerivedFrom.Kind), Filename: operation.Provenance.DerivedFrom.Filename}
		}
	}
	if operation.Error != nil {
		result.Error = &OperationError{Kind: operation.Error.Kind, Retryable: operation.Error.Retryable}
	}
	if operation.Metrics != nil {
		result.Metrics = &OperationMetrics{
			Turns: operation.Metrics.Turns, ToolCalls: operation.Metrics.ToolCalls, Duration: operation.Metrics.Duration,
			SummariesWritten: operation.Metrics.SummariesWritten, SummariesSkipped: operation.Metrics.SummariesSkipped,
			DistillationWritten: operation.Metrics.DistillationWritten, DistillationSkipped: operation.Metrics.DistillationSkipped,
		}
		if operation.Metrics.Usage != nil {
			result.Metrics.Usage = &TokenUsage{PromptTokens: operation.Metrics.Usage.PromptTokens, CompletionTokens: operation.Metrics.Usage.CompletionTokens, TotalTokens: operation.Metrics.Usage.TotalTokens}
		}
	}
	return result
}

func publicOperationPage(page app.OperationPage) OperationPage {
	entries := make([]Operation, len(page.Entries))
	for index, event := range page.Entries {
		entries[index] = publicOperation(event)
	}
	return OperationPage{Directory: page.Directory, Entries: entries, Offset: page.Offset, Limit: page.Limit, NextOffset: page.NextOffset, HasMore: page.HasMore}
}

func publicSnapOutcomes(outcomes []app.SnapOutcome) []SnapOutcome {
	result := make([]SnapOutcome, len(outcomes))
	for index, outcome := range outcomes {
		result[index] = SnapOutcome{SourceKey: outcome.SourceKey, Filename: outcome.Filename, Err: publicError(outcome.Err)}
	}
	return result
}

func publicSynthResult(result app.SynthResult) SynthResult {
	converted := SynthResult{
		SummariesWritten: result.SummariesWritten, SummariesSkipped: result.SummariesSkipped,
		DistillationWritten: result.DistillationWritten, DistillationSkipped: result.DistillationSkipped,
		Report:  publicOperationReport(result.Committed),
		Metrics: Metrics{Turns: result.Metrics.Turns, ToolCalls: result.Metrics.ToolCalls, Duration: result.Metrics.Duration},
	}
	if result.Metrics.Usage != nil {
		converted.Metrics.Usage = &TokenUsage{PromptTokens: result.Metrics.Usage.PromptTokens, CompletionTokens: result.Metrics.Usage.CompletionTokens, TotalTokens: result.Metrics.Usage.TotalTokens}
	}
	return converted
}

func publicOperationReport(operations []app.Operation) []OperationReport {
	reports := make([]OperationReport, 0, 2)
	for _, operation := range operations {
		if operation.Outcome != app.OutcomeCommitted || operation.Document == nil {
			continue
		}
		index := -1
		for reportIndex := range reports {
			if reports[reportIndex].Operation == OperationCommand(operation.Command) {
				index = reportIndex
				break
			}
		}
		if index == -1 {
			reports = append(reports, OperationReport{Operation: OperationCommand(operation.Command)})
			index = len(reports) - 1
		}
		document := DocumentIdentity{Kind: DocumentKind(operation.Document.Kind), Filename: operation.Document.Filename}
		duplicate := false
		for _, existing := range reports[index].Documents {
			if existing == document {
				duplicate = true
				break
			}
		}
		if !duplicate {
			reports[index].Documents = append(reports[index].Documents, document)
		}
	}
	return reports
}

func publicError(err error) error {
	if err == nil {
		return nil
	}
	var categorized *internalerrors.Error
	if stderrors.As(err, &categorized) {
		return &Error{
			Kind:      ErrorKind(categorized.Kind),
			Detail:    categorized.Detail,
			Retryable: categorized.Retryable,
			Cause:     err,
		}
	}
	if contextErr := internalerrors.Context(err); contextErr != nil {
		return &Error{
			Kind:      ErrorKind(contextErr.Kind),
			Detail:    contextErr.Detail,
			Retryable: contextErr.Retryable,
			Cause:     err,
		}
	}
	if stderrors.Is(err, internalerrors.ErrAlreadyExists) {
		return &Error{
			Kind:   ErrorKindAlreadyExists,
			Detail: "document already exists",
			Cause:  err,
		}
	}
	return err
}

func internalError(err error) error {
	if err == nil {
		return nil
	}
	var categorized *Error
	if stderrors.As(err, &categorized) {
		return &internalerrors.Error{
			Kind:      internalerrors.Kind(categorized.Kind),
			Detail:    categorized.Detail,
			Retryable: categorized.Retryable,
			Cause:     err,
		}
	}
	if stderrors.Is(err, ErrAlreadyExists) {
		return internalerrors.Wrap(internalerrors.KindAlreadyExists, "document already exists", err)
	}
	return err
}

func internalErrorAs(err error, kind internalerrors.Kind, detail string) error {
	converted := internalError(err)
	if converted == nil {
		return nil
	}
	var categorized *internalerrors.Error
	if stderrors.As(converted, &categorized) {
		return converted
	}
	if contextErr := internalerrors.Context(converted); contextErr != nil {
		return contextErr
	}
	return internalerrors.Wrap(kind, detail, converted)
}
