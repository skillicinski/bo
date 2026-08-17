package bo

import (
	"context"
	"errors"
	"fmt"
	"time"
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

func Snap(ctx context.Context, storage Storage, fetcher Source, urls []string) ([]SnapOutcome, error) {
	if len(urls) == 0 {
		return nil, NewSnapInputError("usage: bo snap <dir> <url>...")
	}
	state, generation, err := storage.ReadState(ctx)
	if err != nil {
		if !IsCategory(err, CategoryFilesystem) {
			err = FilesystemError(err.Error())
		}
		return nil, &SnapCommandError{Err: err}
	}
	outcomes := make([]SnapOutcome, 0, len(urls))
	for _, input := range urls {
		page, fetchErr := fetcher.Fetch(ctx, input)
		if fetchErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceURL: input, Err: fetchErr})
			continue
		}
		sourceURL := page.SourceURL
		if sourceURL == "" {
			sourceURL = input
		}
		slug, slugErr := KebabCase(page.Title)
		if slugErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceURL: input, Err: slugErr})
			continue
		}
		writtenAt := uint64(time.Now().UnixMilli())
		filename, document, writeErr := createRaw(ctx, storage, slug, writtenAt, []byte(page.Markdown))
		if writeErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceURL: input, Err: writeErr})
			continue
		}
		next := state
		next.Raw = append(append([]RawRecord{}, state.Raw...), RawRecord{
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
			return outcomes, &SnapCommandError{Completed: outcomes, SourceURL: input, Err: failure}
		}
		state, generation = next, newGeneration
		outcomes = append(outcomes, SnapOutcome{SourceURL: input, Filename: filename})
	}
	return outcomes, nil
}

func createRaw(ctx context.Context, storage Storage, slug string, timestamp uint64, contents []byte) (string, DocumentRef, error) {
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
			return "", DocumentRef{}, err
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
