package application

import (
	"context"
	"errors"
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
	options = normalizeOperationOptions(options)
	if workspace == nil {
		return nil, snapWorkflowFailure(ctx, nil, directory, options, internalerrors.Request("workspace is not configured"))
	}
	if len(inputs) == 0 {
		return nil, snapWorkflowFailure(ctx, workspace, directory, options, NewSnapInputError("usage: bo snap <dir> <source>..."))
	}
	if workflow == nil {
		return nil, snapWorkflowFailure(ctx, workspace, directory, options, NewSnapInputError("source workflow is not configured"))
	}
	_, revision, err := workspace.ReadState(ctx)
	if err != nil {
		err = normalizeError(err, internalerrors.KindFilesystem, "reading workspace state")
		operation := newOperation(CommandSnap, options.Actor)
		eventErr := snapFailureEvent(ctx, workspace, operation, err)
		return nil, &SnapCommandError{Err: errors.Join(err, eventErr)}
	}
	outcomes := make([]SnapOutcome, 0, len(inputs))
	for index, input := range inputs {
		operation := newOperation(CommandSnap, options.Actor)
		if domain.ValidateSourceKey(input) == nil {
			operation.Source = &domain.SourceIdentity{SourceKey: input}
		}
		snapshot, fetchErr := workflow.Fetch(ctx, input)
		if fetchErr != nil {
			fetchErr = normalizeError(fetchErr, internalerrors.KindSource, "fetching source")
			failureErr := snapFailureEvent(ctx, workspace, operation, fetchErr)
			if failureErr != nil {
				return outcomes, &SnapCommandError{Completed: outcomes, SourceKey: input, Err: errors.Join(fetchErr, failureErr)}
			}
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: fetchErr})
			continue
		}
		sourceKey := snapshot.SourceKey
		if sourceKey == "" {
			sourceKey = input
		}
		if sourceErr := domain.ValidateSourceKey(sourceKey); sourceErr != nil {
			sourceErr = normalizeError(sourceErr, internalerrors.KindValidation, "validating source key")
			failureErr := snapFailureEvent(ctx, workspace, operation, sourceErr)
			if failureErr != nil {
				return outcomes, &SnapCommandError{Completed: outcomes, SourceKey: input, Err: errors.Join(sourceErr, failureErr)}
			}
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: sourceErr})
			continue
		}
		operation.Source = &domain.SourceIdentity{SourceKey: sourceKey}
		slug, slugErr := KebabCase(snapshot.Title)
		if slugErr != nil {
			slugErr = normalizeError(slugErr, internalerrors.KindValidation, "creating snapshot filename")
			failureErr := snapFailureEvent(ctx, workspace, operation, slugErr)
			if failureErr != nil {
				return outcomes, &SnapCommandError{Completed: outcomes, SourceKey: input, Err: errors.Join(slugErr, failureErr)}
			}
			outcomes = append(outcomes, SnapOutcome{SourceKey: input, Err: slugErr})
			continue
		}
		writtenAt := time.Now().UTC()
		filename, newRevision, writeErr := createRaw(ctx, workspace, revision, sourceKey, slug, writtenAt, snapshot.Markdown, operation)
		if writeErr != nil {
			batchErr := writeErr
			for _, skipped := range inputs[index+1:] {
				skippedOperation := newOperation(CommandSnap, options.Actor)
				if domain.ValidateSourceKey(skipped) == nil {
					skippedOperation.Source = &domain.SourceIdentity{SourceKey: skipped}
				}
				if failureErr := snapFailureEvent(ctx, workspace, skippedOperation, fmt.Errorf("snap batch aborted after %s", input)); failureErr != nil {
					batchErr = errors.Join(batchErr, failureErr)
					break
				}
			}
			return outcomes, &SnapCommandError{Completed: outcomes, SourceKey: input, Err: batchErr}
		}
		revision = newRevision
		outcomes = append(outcomes, SnapOutcome{SourceKey: sourceKey, Filename: filename})
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

func createRaw(ctx context.Context, workspace Workspace, revision Revision, sourceKey, slug string, writtenAt time.Time, contents []byte, operation Operation) (string, Revision, error) {
	for attempt := 1; ; attempt++ {
		filename := slug + ".md"
		if attempt == 2 {
			filename = fmt.Sprintf("%s--%d.md", slug, writtenAt.UnixNano())
		} else if attempt > 2 {
			filename = fmt.Sprintf("%s--%d--%d.md", slug, writtenAt.UnixNano(), attempt)
		}
		attemptOperation := operation
		attemptOperation.Attempt = attempt
		attemptOperation.Document = &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: filename}
		committed := committedOperation(attemptOperation)
		commit := SnapshotCommit{SourceKey: sourceKey, Filename: filename, WrittenAt: writtenAt, Contents: contents, Event: committed}
		_, newRevision, err := workspace.CommitSnapshot(ctx, commit, revision)
		if err == nil {
			return filename, newRevision, nil
		}
		if eventErr := recordFailedOperation(ctx, workspace, attemptOperation, err); eventErr != nil {
			return "", revision, errors.Join(err, eventErr)
		}
		if !internalerrors.IsAlreadyExists(err) {
			return "", Revision{}, err
		}
	}
}

func snapFailureEvent(ctx context.Context, workspace Workspace, operation Operation, cause error) error {
	return recordFailedOperation(ctx, workspace, operation, cause)
}

func snapWorkflowFailure(ctx context.Context, workspace Workspace, directory string, options OperationOptions, cause error) error {
	if workspace != nil {
		if err := recordFailedOperation(ctx, workspace, newOperation(CommandSnap, options.Actor), cause); err != nil {
			return errors.Join(cause, err)
		}
		return cause
	}
	return cause
}
