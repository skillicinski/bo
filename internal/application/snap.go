package application

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/skillicinski/bo/internal/domain"
)

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

func Snap(ctx context.Context, storage Storage, fetcher Source, directory string, urls []string, options OperationOptions) ([]SnapOutcome, error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return nil, err
	}
	logSnap := func(url, filename string, operationErr error) {
		details := map[string]any{"url": url}
		if filename != "" {
			details["filename"] = filename
		}
		for key, value := range operationErrorDetails(operationErr) {
			details[key] = value
		}
		recordOperation(options, directory, CommandSnap, operationErr == nil, details)
	}
	if len(urls) == 0 {
		return nil, NewSnapInputError("usage: bo snap <dir> <url>...")
	}
	state, generation, err := storage.ReadState(ctx)
	if err != nil {
		if !IsCategory(err, CategoryFilesystem) {
			err = FilesystemError(err.Error())
		}
		for _, input := range urls {
			logSnap(input, "", err)
		}
		return nil, &SnapCommandError{Err: err}
	}
	outcomes := make([]SnapOutcome, 0, len(urls))
	for index, input := range urls {
		page, fetchErr := fetcher.Fetch(ctx, input)
		if fetchErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceURL: input, Err: fetchErr})
			logSnap(input, "", fetchErr)
			continue
		}
		sourceURL := page.SourceURL
		if sourceURL == "" {
			sourceURL = input
		}
		slug, slugErr := KebabCase(page.Title)
		if slugErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceURL: input, Err: slugErr})
			logSnap(input, "", slugErr)
			continue
		}
		writtenAt := uint64(time.Now().UnixMilli())
		filename, document, writeErr := createRaw(ctx, storage, slug, writtenAt, []byte(page.Markdown))
		if writeErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceURL: input, Err: writeErr})
			logSnap(input, "", writeErr)
			continue
		}
		next := state
		next.Raw = append(append([]domain.RawRecord{}, state.Raw...), domain.RawRecord{
			Filename: filename, URL: sourceURL, WrittenAt: writtenAt,
		})
		newGeneration, publishErr := storage.PublishState(ctx, next, generation)
		if publishErr != nil {
			rollbackErr := storage.DeleteDocument(ctx, document)
			detail := fmt.Sprintf("updating state failed: %s; snapshot written then deleted", errorDetail(publishErr))
			if rollbackErr != nil {
				detail = fmt.Sprintf("updating state failed: %s; snapshot cleanup failed: %s", errorDetail(publishErr), errorDetail(rollbackErr))
			}
			failure := FilesystemError(detail)
			if IsConflict(publishErr) {
				failure = ConflictError(detail)
			}
			logSnap(input, "", failure)
			for _, skipped := range urls[index+1:] {
				logSnap(skipped, "", fmt.Errorf("snap batch aborted after %s", input))
			}
			return outcomes, &SnapCommandError{Completed: outcomes, SourceURL: input, Err: failure}
		}
		state, generation = next, newGeneration
		outcomes = append(outcomes, SnapOutcome{SourceURL: input, Filename: filename})
		logSnap(input, filename, nil)
	}
	return outcomes, nil
}

func createRaw(ctx context.Context, storage Storage, slug string, timestamp uint64, contents []byte) (string, domain.DocumentRef, error) {
	for attempt := 0; ; attempt++ {
		filename := slug + ".md"
		if attempt == 1 {
			filename = fmt.Sprintf("%s--%d.md", slug, timestamp)
		} else if attempt > 1 {
			filename = fmt.Sprintf("%s--%d--%d.md", slug, timestamp, attempt)
		}
		ref, err := storage.CreateRaw(ctx, filename, contents)
		if err == nil {
			return filename, ref, nil
		}
		if !IsAlreadyExists(err) {
			return "", domain.DocumentRef{}, err
		}
	}
}

func errorDetail(err error) string {
	var categorized *Error
	if errors.As(err, &categorized) {
		return categorized.Detail
	}
	return err.Error()
}
