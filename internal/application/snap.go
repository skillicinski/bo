package application

import (
	"context"
	"fmt"
	"net/http"
	"time"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
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
	return &SnapCommandError{Err: internalerrors.Validation(detail)}
}

func Snap(ctx context.Context, workspace Workspace, directory string, inputs []string, options OperationOptions) ([]SnapOutcome, error) {
	return SnapWithWorkflow(ctx, workspace, defaultSourceWorkflow(), directory, inputs, options)
}

func SnapWithWorkflow(ctx context.Context, workspace Workspace, workflow source.Fetcher, directory string, inputs []string, options OperationOptions) ([]SnapOutcome, error) {
	var err error
	options, err = normalizeOperationOptions(options)
	if err != nil {
		return nil, err
	}
	if workspace == nil {
		return nil, internalerrors.Request("workspace is not configured")
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
	_, revision, err := workspace.ReadState(ctx)
	if err != nil {
		err = normalizeError(err, internalerrors.KindFilesystem, "reading workspace state")
		for _, input := range inputs {
			logSnap(input, "", err)
		}
		return nil, &SnapCommandError{Err: err}
	}
	outcomes := make([]SnapOutcome, 0, len(inputs))
	for index, input := range inputs {
		snapshot, fetchErr := workflow.Fetch(ctx, input)
		if fetchErr != nil {
			fetchErr = normalizeError(fetchErr, internalerrors.KindSource, "fetching source")
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: fetchErr})
			logSnap(input, "", fetchErr)
			continue
		}
		sourceKey := snapshot.SourceKey
		if sourceKey == "" {
			sourceKey = input
		}
		if sourceErr := domain.ValidateSourceKey(sourceKey); sourceErr != nil {
			sourceErr = normalizeError(sourceErr, internalerrors.KindValidation, "validating source key")
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: sourceErr})
			logSnap(input, "", sourceErr)
			continue
		}
		slug, slugErr := KebabCase(snapshot.Title)
		if slugErr != nil {
			slugErr = normalizeError(slugErr, internalerrors.KindValidation, "creating snapshot filename")
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: slugErr})
			logSnap(input, "", slugErr)
			continue
		}
		writtenAt := time.Now().UTC()
		filename, newRevision, writeErr := createRaw(ctx, workspace, revision, sourceKey, slug, writtenAt, snapshot.Markdown)
		if writeErr != nil {
			logSnap(input, "", writeErr)
			for _, skipped := range inputs[index+1:] {
				logSnap(skipped, "", fmt.Errorf("snap batch aborted after %s", input))
			}
			return outcomes, &SnapCommandError{Completed: outcomes, SourceKey: input, Err: writeErr}
		}
		revision = newRevision
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

func createRaw(ctx context.Context, workspace Workspace, revision Revision, sourceKey, slug string, writtenAt time.Time, contents []byte) (string, Revision, error) {
	for attempt := 0; ; attempt++ {
		filename := slug + ".md"
		if attempt == 1 {
			filename = fmt.Sprintf("%s--%d.md", slug, writtenAt.UnixNano())
		} else if attempt > 1 {
			filename = fmt.Sprintf("%s--%d--%d.md", slug, writtenAt.UnixNano(), attempt)
		}
		_, revision, err := workspace.CommitSnapshot(ctx, SnapshotCommit{
			SourceKey: sourceKey, Filename: filename, WrittenAt: writtenAt, Contents: contents,
		}, revision)
		if err == nil {
			return filename, revision, nil
		}
		if !internalerrors.IsAlreadyExists(err) {
			return "", Revision{}, err
		}
	}
}
