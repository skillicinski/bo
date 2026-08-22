package application

import internalerrors "github.com/skillicinski/bo/internal/errors"

type ErrorCategory = internalerrors.Category

type ErrorKind = ErrorCategory

const (
	CategoryInput       = internalerrors.CategoryInput
	CategoryRequest     = internalerrors.CategoryRequest
	CategoryHTTP        = internalerrors.CategoryHTTP
	CategoryContent     = internalerrors.CategoryContent
	CategoryFilesystem  = internalerrors.CategoryFilesystem
	CategoryUnsupported = internalerrors.CategoryUnsupported
	CategoryConflict    = internalerrors.CategoryConflict

	ErrorInput       = CategoryInput
	ErrorRequest     = CategoryRequest
	ErrorHTTP        = CategoryHTTP
	ErrorContent     = CategoryContent
	ErrorFilesystem  = CategoryFilesystem
	ErrorUnsupported = CategoryUnsupported
	ErrorConflict    = CategoryConflict
)

type Error = internalerrors.Error

func NewError(category ErrorCategory, detail string) *Error {
	return internalerrors.New(category, detail)
}

func InputError(detail string) *Error       { return internalerrors.Input(detail) }
func RequestError(detail string) *Error     { return internalerrors.Request(detail) }
func ContentError(detail string) *Error     { return internalerrors.Content(detail) }
func FilesystemError(detail string) *Error  { return internalerrors.Filesystem(detail) }
func UnsupportedError(detail string) *Error { return internalerrors.Unsupported(detail) }
func ConflictError(detail string) *Error    { return internalerrors.Conflict(detail) }

func HTTPError(status int, requestID string) *Error {
	return internalerrors.HTTP(status, requestID)
}

var ErrAlreadyExists = internalerrors.ErrAlreadyExists

type SnapError = Error

func IsCategory(err error, category ErrorCategory) bool {
	return internalerrors.IsCategory(err, category)
}

func IsConflict(err error) bool      { return internalerrors.IsConflict(err) }
func IsFilesystem(err error) bool    { return internalerrors.IsFilesystem(err) }
func IsAlreadyExists(err error) bool { return internalerrors.IsAlreadyExists(err) }
