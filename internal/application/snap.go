package application

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"time"

	"github.com/skillicinski/bo/internal/domain"
	"github.com/skillicinski/bo/internal/source"
	filesource "github.com/skillicinski/bo/internal/source/file"
	urlsource "github.com/skillicinski/bo/internal/source/url"
)

type SnapOutcome struct {
	SourceKey string
	Filename  string
	Err       error
}

func (o SnapOutcome) Failed() bool { return o.Err != nil }

type SnapCommandError struct {
	Completed []SnapOutcome
	SourceKey string
	Err       error
}

func (e *SnapCommandError) Error() string {
	if e.SourceKey != "" && e.Err != nil {
		return fmt.Sprintf("%s (%s)", e.SourceKey, e.Err)
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

func Snap(ctx context.Context, storage Storage, directory string, inputs []string, options OperationOptions) ([]SnapOutcome, error) {
	return SnapWithWorkflow(ctx, storage, defaultSourceWorkflow(), directory, inputs, options)
}

func SnapWithWorkflow(ctx context.Context, storage Storage, workflow source.Fetcher, directory string, inputs []string, options OperationOptions) ([]SnapOutcome, error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return nil, err
	}
	logSnap := func(sourceKey, filename string, operationErr error) {
		details := map[string]any{"source_key": sourceKey}
		if filename != "" {
			details["filename"] = filename
		}
		for key, value := range operationErrorDetails(operationErr) {
			details[key] = value
		}
		recordOperation(options, directory, CommandSnap, operationErr == nil, details)
	}
	if len(inputs) == 0 {
		return nil, NewSnapInputError("usage: bo snap <dir> <source>...")
	}
	if workflow == nil {
		return nil, NewSnapInputError("source workflow is not configured")
	}
	state, generation, err := storage.ReadState(ctx)
	if err != nil {
		if !IsCategory(err, CategoryFilesystem) {
			err = FilesystemError(err.Error())
		}
		for _, input := range inputs {
			logSnap(input, "", err)
		}
		return nil, &SnapCommandError{Err: err}
	}
	outcomes := make([]SnapOutcome, 0, len(inputs))
	for index, input := range inputs {
		snapshot, fetchErr := workflow.Fetch(ctx, input)
		if fetchErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: fetchErr})
			logSnap(input, "", fetchErr)
			continue
		}
		sourceKey := snapshot.SourceKey
		if sourceKey == "" {
			sourceKey = input
		}
		if sourceErr := domain.ValidateSourceKey(sourceKey); sourceErr != nil {
			sourceErr = InputError(sourceErr.Error())
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: sourceErr})
			logSnap(input, "", sourceErr)
			continue
		}
		slug, slugErr := KebabCase(snapshot.Title)
		if slugErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: slugErr})
			logSnap(input, "", slugErr)
			continue
		}
		writtenAt := time.Now().UTC()
		filename, document, writeErr := createRaw(ctx, storage, slug, writtenAt.UnixNano(), snapshot.Markdown)
		if writeErr != nil {
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: writeErr})
			logSnap(input, "", writeErr)
			continue
		}
		next := state
		next.Sources = append([]domain.SourceRecord{}, state.Sources...)
		found := false
		for index := range next.Sources {
			if next.Sources[index].SourceKey == sourceKey {
				next.Sources[index].Snapshots = append(append([]domain.RawRecord{}, next.Sources[index].Snapshots...), domain.RawRecord{
					Filename: filename, WrittenAt: writtenAt,
				})
				found = true
				break
			}
		}
		if !found {
			next.Sources = append(next.Sources, domain.SourceRecord{SourceKey: sourceKey, Snapshots: []domain.RawRecord{{
				Filename: filename, WrittenAt: writtenAt,
			}}})
		}
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
			for _, skipped := range inputs[index+1:] {
				logSnap(skipped, "", fmt.Errorf("snap batch aborted after %s", input))
			}
			return outcomes, &SnapCommandError{Completed: outcomes, SourceKey: input, Err: failure}
		}
		state, generation = next, newGeneration
		outcomes = append(outcomes, SnapOutcome{SourceKey: sourceKey, Filename: filename})
		logSnap(sourceKey, filename, nil)
	}
	return outcomes, nil
}

func defaultSourceWorkflow() *source.Workflow {
	client := &http.Client{Timeout: 30 * time.Second}
	return source.NewWorkflow(
		[]source.Transport{urlsource.NewTransport(), filesource.NewTransport()},
		map[source.OriginType]source.Plugin{
			source.OriginHTML:     urlsource.NewHTML(client),
			source.OriginYouTube:  urlsource.NewYouTube(client),
			source.OriginMarkdown: filesource.NewMarkdownPlugin(),
		},
	)
}

func createRaw(ctx context.Context, storage Storage, slug string, timestamp int64, contents []byte) (string, domain.DocumentRef, error) {
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
