package bo

import (
	"context"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/application"
)

type ErrorCategory = application.ErrorCategory
type ErrorKind = application.ErrorKind

const (
	CategoryInput       = application.CategoryInput
	CategoryRequest     = application.CategoryRequest
	CategoryHTTP        = application.CategoryHTTP
	CategoryContent     = application.CategoryContent
	CategoryFilesystem  = application.CategoryFilesystem
	CategoryUnsupported = application.CategoryUnsupported
	CategoryConflict    = application.CategoryConflict

	ErrorInput       = application.ErrorInput
	ErrorRequest     = application.ErrorRequest
	ErrorHTTP        = application.ErrorHTTP
	ErrorContent     = application.ErrorContent
	ErrorFilesystem  = application.ErrorFilesystem
	ErrorUnsupported = application.ErrorUnsupported
	ErrorConflict    = application.ErrorConflict
)

type Error = application.Error

func NewError(category ErrorCategory, detail string) *Error {
	return application.NewError(category, detail)
}

func InputError(detail string) *Error       { return application.InputError(detail) }
func RequestError(detail string) *Error     { return application.RequestError(detail) }
func ContentError(detail string) *Error     { return application.ContentError(detail) }
func FilesystemError(detail string) *Error  { return application.FilesystemError(detail) }
func UnsupportedError(detail string) *Error { return application.UnsupportedError(detail) }
func ConflictError(detail string) *Error    { return application.ConflictError(detail) }
func HTTPError(status int, requestID string) *Error {
	return application.HTTPError(status, requestID)
}

var ErrAlreadyExists = application.ErrAlreadyExists

type SnapError = application.SnapError

func IsCategory(err error, category ErrorCategory) bool {
	return application.IsCategory(err, category)
}

func IsConflict(err error) bool      { return application.IsConflict(err) }
func IsFilesystem(err error) bool    { return application.IsFilesystem(err) }
func IsAlreadyExists(err error) bool { return application.IsAlreadyExists(err) }

type Generation = application.Generation

func NewGeneration(data []byte) Generation { return application.NewGeneration(data) }

type Storage = application.Storage
type DocumentStorage = Storage
type DocumentStore = Storage

type Operation = application.Operation
type OperationCommand = application.OperationCommand
type OperationLog = application.OperationLog
type OperationPage = application.OperationPage
type OperationOptions = application.OperationOptions

const (
	CommandSeed         = application.CommandSeed
	CommandSnap         = application.CommandSnap
	CommandState        = application.CommandState
	CommandSynth        = application.CommandSynth
	CommandWriteSummary = application.CommandWriteSummary
)

type Page = application.Page
type Source = application.Source
type Fetcher = application.Fetcher

type SnapOutcome = application.SnapOutcome
type SnapCommandError = application.SnapCommandError

func NewSnapInputError(detail string) *SnapCommandError {
	return application.NewSnapInputError(detail)
}

func Snap(ctx context.Context, storage Storage, fetcher Source, directory string, urls []string, options OperationOptions) ([]SnapOutcome, error) {
	return application.Snap(ctx, storage, fetcher, directory, urls, options)
}

func StateOutput(ctx context.Context, storage Storage, directory string, full bool, options OperationOptions) (string, error) {
	return application.StateOutput(ctx, storage, directory, full, options)
}

func KebabCase(value string) (string, error) { return application.KebabCase(value) }

type Workspace = application.Workspace
type WorkspaceCreator = application.WorkspaceCreator
type WorkspaceOpener = application.WorkspaceOpener

func Seed(ctx context.Context, creator WorkspaceCreator, name string, options OperationOptions) (string, error) {
	return application.Seed(ctx, creator, name, options)
}

func Synthesize(ctx context.Context, opener WorkspaceOpener, workspaceName string, provider CompletionProvider, options SynthesisOptions, operationOptions OperationOptions) (SynthesisResult, error) {
	return application.Synthesize(ctx, opener, workspaceName, provider, options, operationOptions)
}

const (
	DefaultMaxTurns                = application.DefaultMaxTurns
	DefaultMaxToolCalls            = application.DefaultMaxToolCalls
	DefaultMaxToolOutputBytes      = application.DefaultMaxToolOutputBytes
	DefaultMaxResponseTokens       = application.DefaultMaxResponseTokens
	DefaultSynthesisTimeoutSeconds = application.DefaultSynthesisTimeoutSeconds
)

type SynthesisOptions = application.SynthesisOptions
type SynthesisResult = application.SynthesisResult
type Metrics = agent.Metrics
type TokenUsage = agent.TokenUsage
type Usage = agent.Usage

func DefaultSynthesisOptions() SynthesisOptions { return application.DefaultSynthesisOptions() }

type ChatMessage = agent.ChatMessage
type ToolCall = agent.ToolCall
type ToolFunction = agent.ToolFunction
type ToolDefinition = agent.ToolDefinition
type ToolDeclaration = agent.ToolDeclaration
type CompletionRequest = agent.CompletionRequest
type CompletionResponse = agent.CompletionResponse
type CompletionProvider = agent.CompletionProvider
type Provider = agent.Provider
