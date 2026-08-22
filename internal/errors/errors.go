package errors

import (
	stderrors "errors"
	"fmt"
)

type Category string

const (
	CategoryInput       Category = "input"
	CategoryRequest     Category = "request"
	CategoryHTTP        Category = "http"
	CategoryContent     Category = "content"
	CategoryFilesystem  Category = "filesystem"
	CategoryUnsupported Category = "unsupported"
	CategoryConflict    Category = "conflict"
)

// Error is a user-facing error with a stable category.
type Error struct {
	Category  Category
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

func New(category Category, detail string) *Error { return &Error{Category: category, Detail: detail} }

func Input(detail string) *Error       { return New(CategoryInput, detail) }
func Request(detail string) *Error     { return New(CategoryRequest, detail) }
func Content(detail string) *Error     { return New(CategoryContent, detail) }
func Filesystem(detail string) *Error  { return New(CategoryFilesystem, detail) }
func Unsupported(detail string) *Error { return New(CategoryUnsupported, detail) }
func Conflict(detail string) *Error    { return New(CategoryConflict, detail) }

func HTTP(status int, requestID string) *Error {
	return &Error{Category: CategoryHTTP, Status: status, RequestID: requestID}
}

var ErrAlreadyExists = stderrors.New("document already exists")

func IsCategory(err error, category Category) bool {
	var categorized *Error
	return stderrors.As(err, &categorized) && categorized.Category == category
}

func IsConflict(err error) bool      { return IsCategory(err, CategoryConflict) }
func IsFilesystem(err error) bool    { return IsCategory(err, CategoryFilesystem) }
func IsAlreadyExists(err error) bool { return stderrors.Is(err, ErrAlreadyExists) }
