package application

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const (
	toolReadCorpus   = "read_corpus"
	toolReadLogs     = "read_logs"
	toolReadDocument = "read_document"
	toolReadSummary  = "read_summary"
	toolWriteSummary = "write_summary"
	toolEditSummary  = "edit_summary"
)

var allSynthesisTools = []string{toolReadCorpus, toolReadLogs, toolReadDocument, toolReadSummary, toolWriteSummary, toolEditSummary}

func normalizeSynthesisTools(names []string) ([]string, error) {
	if len(names) == 0 || len(names) == 1 && names[0] == "all" {
		return append([]string{}, allSynthesisTools...), nil
	}
	known := make(map[string]bool, len(allSynthesisTools))
	for _, name := range allSynthesisTools {
		known[name] = true
	}
	seen := make(map[string]bool, len(names))
	validated := make([]string, 0, len(names))
	for _, name := range names {
		if !known[name] {
			return nil, fmt.Errorf("unknown synthesis tool: %s", name)
		}
		if seen[name] {
			return nil, fmt.Errorf("duplicate synthesis tool: %s", name)
		}
		seen[name] = true
		validated = append(validated, name)
	}
	return validated, nil
}

func synthTools(contextState *agentContext, names []string) []agent.Tool {
	objectParameters := func(properties map[string]any, required []string) map[string]any {
		return map[string]any{"type": "object", "properties": properties, "required": required, "additionalProperties": false}
	}
	execute := func(_ context.Context, call agent.ToolCall) (string, error) {
		return executeToolCall(contextState, call)
	}
	definitions := map[string]agent.Tool{
		toolReadCorpus:   {Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: toolReadCorpus, Description: "Read the authoritative corpus state, including raw snapshots and summaries.", Parameters: objectParameters(map[string]any{}, []string{})}}, Execute: execute},
		toolReadLogs:     {Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: toolReadLogs, Description: "Read paginated operation log entries for the current directory.", Parameters: objectParameters(map[string]any{"offset": map[string]any{"type": "integer", "default": 0, "minimum": 0}, "limit": map[string]any{"type": "integer", "default": 20, "minimum": 1, "maximum": 100}}, []string{})}}, Execute: execute},
		toolReadDocument: {Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: toolReadDocument, Description: "Read the newest raw Markdown document for one source identity. Use its exact filename.", Parameters: objectParameters(map[string]any{"filename": map[string]any{"type": "string"}}, []string{"filename"})}}, Execute: execute},
		toolReadSummary:  {Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: toolReadSummary, Description: "Read the existing Markdown summary for one source identity.", Parameters: objectParameters(map[string]any{"source_key": map[string]any{"type": "string"}}, []string{"source_key"})}}, Execute: execute},
		toolWriteSummary: {Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: toolWriteSummary, Description: "Create a Markdown summary for one source identity using its newest raw snapshot.", Parameters: objectParameters(map[string]any{"source_key": map[string]any{"type": "string"}, "markdown": map[string]any{"type": "string"}}, []string{"source_key", "markdown"})}}, Execute: execute},
		toolEditSummary:  {Definition: agent.ToolDefinition{Type: "function", Function: agent.ToolDeclaration{Name: toolEditSummary, Description: "Replace an existing Markdown summary for one source identity in full.", Parameters: objectParameters(map[string]any{"source_key": map[string]any{"type": "string"}, "markdown": map[string]any{"type": "string"}}, []string{"source_key", "markdown"})}}, Execute: execute},
	}
	tools := make([]agent.Tool, 0, len(names))
	for _, name := range names {
		tools = append(tools, definitions[name])
	}
	return tools
}

type agentSource struct {
	LatestFilename   string
	LatestWrittenAt  time.Time
	LatestStateIndex int
}

type agentContext struct {
	ctx            context.Context
	directory      string
	workspace      Workspace
	options        OperationOptions
	events         WorkspaceEvents
	documents      map[string]domain.DocumentRef
	sources        map[string]agentSource
	state          domain.State
	revision       Revision
	maxOutputBytes int
	completed      map[string]bool
	written        map[string]bool
	mutationOps    map[string]Operation
	eventFailure   error
}

func (context *agentContext) nextMutationOperation(key string) Operation {
	operation, ok := context.mutationOps[key]
	if !ok {
		operation = newOperation(CommandWriteSummary, context.options.Actor)
	} else {
		operation.Attempt++
		operation.Timestamp = time.Now().UTC().Format(time.RFC3339Nano)
	}
	context.mutationOps[key] = operation
	return operation
}

func systemPrompt(context *agentContext, documentNames []string) string {
	identities := make([]string, 0, len(context.sources))
	for sourceKey, source := range context.sources {
		identities = append(identities, sourceKey+" ("+source.LatestFilename+")")
	}
	sort.Strings(identities)
	return fmt.Sprintf("You are bo's document-summary agent for %s. Purpose: turn each source into a concise Markdown summary that retains its key claims. Ontology: raw documents are immutable snapshots; a source identity is an exact URL or raw:filename; a summary is one mutable document derived from the newest raw snapshot; state is the authoritative record of these links. Rules: use only evidence from the source, preserve qualifications and uncertainty, attribute experience, measurements, recommendations, opinions, and forecasts to their author, and never modify raw documents or invent facts. The host owns document access, newest-snapshot selection, provenance, state publication, and completion. Before reading raw documents, inspect recent scoped committed operation entries with read_logs and follow next_offset while has_more is true. A successful write_summary entry for the current raw filename with an existing current summary means that source is complete; do not rewrite it. Use the available bounded tools. Source identities: %s. Latest raw documents: %s.", context.directory, strings.Join(identities, ", "), strings.Join(documentNames, ", "))
}

func DiscoverDocuments(ctx context.Context, workspace Workspace) (map[string]domain.DocumentRef, error) {
	refs, err := workspace.ListDocuments(ctx, domain.DocumentKindRaw)
	if err != nil {
		return nil, normalizeError(err, internalerrors.KindFilesystem, "listing raw documents")
	}
	documents := make(map[string]domain.DocumentRef, len(refs))
	for _, ref := range refs {
		if ref.Kind != domain.DocumentKindRaw {
			return nil, internalerrors.Validation("workspace returned a non-raw document")
		}
		if err := domain.ValidateDocumentName(ref.Name); err != nil {
			return nil, err
		}
		documents[ref.Name] = ref
	}
	return documents, nil
}

func sourceGroups(documents map[string]domain.DocumentRef, state domain.State) map[string]agentSource {
	names := make([]string, 0, len(documents))
	for name := range documents {
		names = append(names, name)
	}
	sort.Strings(names)
	sources := map[string]agentSource{}
	for _, filename := range names {
		sourceKey, writtenAt, stateIndex := "raw:"+filename, time.Time{}, 0
		found := false
		recordIndex := 0
		for _, source := range state.Sources {
			for _, snapshot := range source.Snapshots {
				index := recordIndex
				recordIndex++
				if snapshot.Filename == filename && (!found || snapshot.WrittenAt.After(writtenAt) || snapshot.WrittenAt.Equal(writtenAt) && index > stateIndex) {
					sourceKey, writtenAt, stateIndex = source.SourceKey, snapshot.WrittenAt, index
					found = true
				}
			}
		}
		current, ok := sources[sourceKey]
		if !ok || writtenAt.After(current.LatestWrittenAt) || writtenAt.Equal(current.LatestWrittenAt) && stateIndex > current.LatestStateIndex {
			sources[sourceKey] = agentSource{LatestFilename: filename, LatestWrittenAt: writtenAt, LatestStateIndex: stateIndex}
		}
	}
	return sources
}

func executeToolCall(context *agentContext, call agent.ToolCall) (output string, returnErr error) {
	name := call.Function.Name
	var mutation *Operation
	if name == toolWriteSummary || name == toolEditSummary {
		event := newOperation(CommandWriteSummary, context.options.Actor)
		mutation = &event
		defer func() {
			if returnErr == nil {
				return
			}
			eventErr := recordFailedOperation(context.ctx, context.workspace, *mutation, returnErr)
			if eventErr != nil {
				context.eventFailure = errors.Join(context.eventFailure, eventErr)
			}
		}()
	}
	var arguments map[string]json.RawMessage
	if err := json.Unmarshal([]byte(call.Function.Arguments), &arguments); err != nil {
		return "", fmt.Errorf("%s arguments are malformed JSON: %v", name, err)
	}
	if arguments == nil {
		return "", fmt.Errorf("%s arguments must be a JSON object", name)
	}
	switch name {
	case toolReadCorpus:
		if len(arguments) != 0 {
			return "", fmt.Errorf("read_corpus arguments must be empty")
		}
		data, err := json.MarshalIndent(context.state, "", "  ")
		if err != nil {
			return "", fmt.Errorf("serializing state failed: %v", err)
		}
		return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
	case toolReadLogs:
		if len(arguments) > 2 {
			return "", fmt.Errorf("read_logs arguments must contain only offset and limit")
		}
		for key := range arguments {
			if key != "offset" && key != "limit" {
				return "", fmt.Errorf("read_logs arguments must contain only offset and limit")
			}
		}
		offset, err := intArgument(arguments, "offset", 0)
		if err != nil || offset < 0 {
			return "", fmt.Errorf("read_logs.offset must be a non-negative integer")
		}
		limit, err := intArgument(arguments, "limit", 20)
		if err != nil || limit < 1 || limit > 100 {
			return "", fmt.Errorf("read_logs.limit must be an integer from 1 to 100")
		}
		return readLogs(context, offset, limit)
	case toolReadDocument:
		if len(arguments) != 1 {
			return "", fmt.Errorf("read_document arguments must contain only filename")
		}
		filename, err := stringArgument(arguments, "filename")
		if err != nil {
			return "", fmt.Errorf("read_document.filename must be a string")
		}
		return readDocument(context, filename)
	case toolReadSummary:
		if len(arguments) != 1 {
			return "", fmt.Errorf("read_summary arguments must contain only source_key")
		}
		sourceKey, err := stringArgument(arguments, "source_key")
		if err != nil {
			return "", fmt.Errorf("read_summary.source_key must be a string")
		}
		return readSummary(context, sourceKey)
	case toolWriteSummary, toolEditSummary:
		if len(arguments) != 2 {
			return "", fmt.Errorf("%s arguments must contain only source_key and markdown", name)
		}
		sourceKey, err := stringArgument(arguments, "source_key")
		if err != nil {
			return "", fmt.Errorf("%s.source_key must be a string", name)
		}
		if mutation != nil {
			*mutation = context.nextMutationOperation(sourceKey)
			if domain.ValidateSourceKey(sourceKey) == nil {
				mutation.Source = &domain.SourceIdentity{SourceKey: sourceKey}
			}
		}
		markdown, err := stringArgument(arguments, "markdown")
		if err != nil {
			return "", fmt.Errorf("%s.markdown must be a string", name)
		}
		if _, ok := context.sources[sourceKey]; !ok {
			return "", fmt.Errorf("unknown source: %s", sourceKey)
		}
		if mutation != nil {
			mutation.Document = &domain.DocumentIdentity{Kind: domain.DocumentKindSummary, Filename: context.sources[sourceKey].LatestFilename}
			writtenAt := context.sources[sourceKey].LatestWrittenAt
			mutation.Provenance = &domain.OperationProvenance{DerivedFrom: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: context.sources[sourceKey].LatestFilename}}
			if !writtenAt.IsZero() {
				mutation.Provenance.RawWrittenAt = &writtenAt
			}
		}
		existing := summaryRecord(context.state, sourceKey)
		if name == toolWriteSummary && existing != nil {
			return "", fmt.Errorf("summary already exists for source: %s", sourceKey)
		}
		if name == toolEditSummary && existing == nil {
			return "", fmt.Errorf("no summary exists for source: %s", sourceKey)
		}
		if err := writeSummary(context, sourceKey, markdown, existing, *mutation); err != nil {
			return "", err
		}
		context.completed[sourceKey] = true
		context.written[sourceKey] = true
		return name + " succeeded: " + sourceKey, nil
	default:
		return "", fmt.Errorf("unsupported tool: %s", name)
	}
}

func intArgument(arguments map[string]json.RawMessage, name string, defaultValue int) (int, error) {
	raw, ok := arguments[name]
	if !ok {
		return defaultValue, nil
	}
	var value int
	if err := json.Unmarshal(raw, &value); err != nil {
		return 0, err
	}
	return value, nil
}

func readLogs(context *agentContext, offset, limit int) (string, error) {
	if context.events == nil {
		return "", internalerrors.Request("workspace event contract is not configured")
	}
	page, err := context.events.ReadEvents(context.ctx, offset, limit)
	if err != nil {
		return "", normalizeError(err, internalerrors.KindFilesystem, "reading operation log")
	}
	page.Directory = context.directory
	page.Offset = offset
	page.Limit = limit
	if page.Entries == nil {
		page.Entries = []Operation{}
	}
	if page.NextOffset < offset || page.NextOffset == 0 && len(page.Entries) > 0 {
		page.NextOffset = offset + len(page.Entries)
	}
	committed := make([]Operation, 0, len(page.Entries))
	for _, operation := range page.Entries {
		if operation.Outcome != domain.OutcomeCommitted {
			continue
		}
		committed = append(committed, operation)
		markCompletedFromOperation(context, operation)
	}
	page.Entries = committed
	data, err := json.Marshal(page)
	if err != nil {
		return "", fmt.Errorf("serializing operation log failed: %v", err)
	}
	return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
}

func markCompletedFromOperation(context *agentContext, operation Operation) {
	if operation.Outcome != domain.OutcomeCommitted || operation.Command != CommandWriteSummary {
		return
	}
	if operation.Source == nil {
		return
	}
	sourceKey := operation.Source.SourceKey
	filename := ""
	if operation.Provenance != nil && operation.Provenance.DerivedFrom != nil {
		filename = operation.Provenance.DerivedFrom.Filename
	}
	if filename == "" && operation.Document != nil {
		filename = operation.Document.Filename
	}
	source, ok := context.sources[sourceKey]
	if !ok || filename != source.LatestFilename {
		return
	}
	record := summaryRecord(context.state, sourceKey)
	if record != nil && record.DerivedFrom == source.LatestFilename {
		context.completed[sourceKey] = true
	}
}

func stringArgument(arguments map[string]json.RawMessage, name string) (string, error) {
	var value string
	if raw, ok := arguments[name]; !ok || json.Unmarshal(raw, &value) != nil {
		return "", fmt.Errorf("not a string")
	}
	return value, nil
}

func readDocument(context *agentContext, filename string) (string, error) {
	filename = strings.TrimPrefix(filename, "raw/")
	if filename == "" || strings.ContainsAny(filename, `/\\`) || domain.ValidateDocumentName(filename) != nil {
		return "", fmt.Errorf("read_document.filename must be a raw Markdown filename")
	}
	latest := false
	for _, source := range context.sources {
		if source.LatestFilename == filename {
			latest = true
			break
		}
	}
	if !latest {
		return "", fmt.Errorf("document is not a newest raw snapshot: %s", filename)
	}
	ref, ok := context.documents[filename]
	if !ok {
		return "", fmt.Errorf("unknown raw document: %s", filename)
	}
	data, err := context.workspace.ReadDocument(context.ctx, ref)
	if err != nil {
		return "", normalizeError(err, internalerrors.KindFilesystem, "reading raw document")
	}
	return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
}

func readSummary(context *agentContext, sourceKey string) (string, error) {
	if _, ok := context.sources[sourceKey]; !ok {
		return "", fmt.Errorf("unknown source: %s", sourceKey)
	}
	record := summaryRecord(context.state, sourceKey)
	if record == nil {
		return "", fmt.Errorf("no summary exists for source: %s", sourceKey)
	}
	data, err := context.workspace.ReadDocument(context.ctx, domain.SummaryRef(record.Filename))
	if err != nil {
		return "", normalizeError(err, internalerrors.KindFilesystem, "reading summary")
	}
	return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
}

func summaryRecord(state domain.State, sourceKey string) *domain.SummaryRecord {
	for index := range state.Sources {
		if state.Sources[index].SourceKey == sourceKey {
			return state.Sources[index].Summary
		}
	}
	return nil
}

func writeSummary(context *agentContext, sourceKey, markdown string, existing *domain.SummaryRecord, operation Operation) error {
	source, ok := context.sources[sourceKey]
	if !ok {
		return fmt.Errorf("unknown source: %s", sourceKey)
	}
	if strings.TrimSpace(markdown) == "" {
		return fmt.Errorf("summary Markdown must be non-empty")
	}
	if len(markdown) > context.maxOutputBytes {
		return fmt.Errorf("summary exceeds max tool output bytes (%d)", context.maxOutputBytes)
	}
	filename := source.LatestFilename
	createdAt := time.Time{}
	if existing != nil {
		filename = existing.Filename
		createdAt = existing.CreatedAt
	}
	now := time.Now().UTC()
	writtenAt := source.LatestWrittenAt
	if writtenAt.IsZero() {
		writtenAt = now
	}
	updatedAt := now
	if existing != nil && !now.After(existing.UpdatedAt) {
		updatedAt = existing.UpdatedAt.Add(time.Nanosecond)
	}
	record := domain.SummaryRecord{Filename: filename, DerivedFrom: source.LatestFilename, CreatedAt: createdAt, UpdatedAt: updatedAt}
	if existing == nil {
		record.CreatedAt = now
	}
	operation.Document = &domain.DocumentIdentity{Kind: domain.DocumentKindSummary, Filename: filename}
	operation.Provenance = &domain.OperationProvenance{DerivedFrom: &domain.DocumentIdentity{Kind: domain.DocumentKindRaw, Filename: source.LatestFilename}}
	if !writtenAt.IsZero() {
		operation.Provenance.RawWrittenAt = &writtenAt
	}
	committed := committedOperation(operation)
	state, revision, err := context.workspace.CommitSummary(context.ctx, SummaryCommit{
		SourceKey: sourceKey, Filename: filename, DerivedFrom: source.LatestFilename,
		RawWrittenAt: writtenAt, CreatedAt: record.CreatedAt, UpdatedAt: record.UpdatedAt,
		Contents: []byte(markdown), Event: committed,
	}, context.revision)
	if err != nil {
		return normalizeError(err, internalerrors.KindFilesystem, "writing summary")
	}
	context.state, context.revision = state, revision
	return nil
}
