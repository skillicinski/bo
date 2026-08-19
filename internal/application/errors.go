package application

import (
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
