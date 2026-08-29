package errors

import (
	"context"
	stderrors "errors"
	"fmt"
)

// Kind is the stable failure classification used inside bo. The root package
// exposes the public form of the same contract.
type Kind string

const (
	KindRequest           Kind = "request"
	KindValidation        Kind = "validation"
	KindSource            Kind = "source"
	KindFilesystem        Kind = "filesystem"
	KindMissingResource   Kind = "missing_resource"
	KindConflict          Kind = "conflict"
	KindAlreadyExists     Kind = "already_exists"
	KindProviderTransport Kind = "provider_transport"
	KindProviderRejected  Kind = "provider_rejected"
	KindProviderMalformed Kind = "provider_malformed"
	KindCanceled          Kind = "canceled"
	KindDeadline          Kind = "deadline"
)

// Error is the internal form of bo's stable failure contract.
type Error struct {
	Kind      Kind
	Detail    string
	Retryable bool
	Cause     error
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

func New(kind Kind, detail string) *Error { return &Error{Kind: kind, Detail: detail} }

func Wrap(kind Kind, detail string, cause error) *Error {
	return &Error{Kind: kind, Detail: detail, Cause: cause}
}

func Request(detail string) *Error         { return New(KindRequest, detail) }
func Validation(detail string) *Error      { return New(KindValidation, detail) }
func Source(detail string) *Error          { return New(KindSource, detail) }
func Filesystem(detail string) *Error      { return New(KindFilesystem, detail) }
func MissingResource(detail string) *Error { return New(KindMissingResource, detail) }
func Conflict(detail string) *Error        { return New(KindConflict, detail) }
func AlreadyExists(detail string) *Error   { return New(KindAlreadyExists, detail) }

func ProviderTransport(detail string, cause error) *Error {
	return Wrap(KindProviderTransport, detail, cause)
}

func TransientProviderTransport(detail string, cause error) *Error {
	err := ProviderTransport(detail, cause)
	err.Retryable = true
	return err
}

func ProviderRejected(detail string, retryable bool) *Error {
	err := New(KindProviderRejected, detail)
	err.Retryable = retryable
	return err
}

func ProviderMalformed(detail string, cause error) *Error {
	return Wrap(KindProviderMalformed, detail, cause)
}

func Context(err error) *Error {
	if err == nil {
		return nil
	}
	var categorized *Error
	if stderrors.As(err, &categorized) {
		if categorized.Kind == KindCanceled || categorized.Kind == KindDeadline {
			return categorized
		}
		return nil
	}
	if stderrors.Is(err, context.Canceled) {
		return Wrap(KindCanceled, "operation canceled", err)
	}
	if stderrors.Is(err, context.DeadlineExceeded) {
		return Wrap(KindDeadline, "operation deadline exceeded", err)
	}
	return nil
}

var ErrAlreadyExists = stderrors.New("document already exists")

func IsKind(err error, kind Kind) bool {
	var categorized *Error
	return stderrors.As(err, &categorized) && categorized.Kind == kind
}

func IsAlreadyExists(err error) bool {
	return stderrors.Is(err, ErrAlreadyExists) || IsKind(err, KindAlreadyExists)
}
