package application

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
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
	LatestWrittenAt  uint64
	LatestStateIndex int
}

type agentContext struct {
	ctx            context.Context
	target         string
	directory      string
	actor          string
	storage        Storage
	operationLog   OperationLog
	documents      map[string]string
	sources        map[string]agentSource
	state          domain.State
	generation     Generation
	maxOutputBytes int
	completed      map[string]bool
	written        map[string]bool
}

func systemPrompt(context *agentContext, documentNames []string) string {
	identities := make([]string, 0, len(context.sources))
	for sourceKey, source := range context.sources {
		identities = append(identities, sourceKey+" ("+source.LatestFilename+")")
	}
	sort.Strings(identities)
	return fmt.Sprintf("You are bo's document-summary agent for %s. Purpose: turn each source into a concise Markdown summary that retains its key claims. Ontology: raw documents are immutable snapshots; a source identity is an exact URL or raw:filename; a summary is one mutable document derived from the newest raw snapshot; state is the authoritative record of these links. Rules: use only evidence from the source, preserve qualifications and uncertainty, attribute experience, measurements, recommendations, opinions, and forecasts to their author, and never modify raw documents or invent facts. The host owns paths, newest-snapshot selection, provenance, state publication, and completion. Before reading raw documents, inspect recent scoped operation entries with read_logs and follow next_offset while has_more is true. A successful write_summary entry for the current raw filename with an existing current summary means that source is complete; do not rewrite it. Use the available bounded tools. Source identities: %s. Latest raw documents: %s.", context.target, strings.Join(identities, ", "), strings.Join(documentNames, ", "))
}

func DiscoverDocuments(root, target string) (map[string]string, error) {
	entries, err := os.ReadDir(target)
	if err != nil {
		return nil, fmt.Errorf("reading %s failed: %w", target, err)
	}
	documents := map[string]string{}
	for _, entry := range entries {
		name := entry.Name()
		if !strings.EqualFold(filepath.Ext(name), ".md") {
			continue
		}
		path := filepath.Join(target, name)
		resolved, err := filepath.EvalSymlinks(path)
		if err != nil {
			return nil, fmt.Errorf("resolving %s failed: %w", path, err)
		}
		if err := ensureInside(resolved, root); err != nil {
			return nil, err
		}
		if err := ensureInside(resolved, target); err != nil {
			return nil, err
		}
		info, err := os.Stat(resolved)
		if err != nil {
			return nil, fmt.Errorf("reading %s failed: %w", resolved, err)
		}
		if info.Mode().IsRegular() {
			documents[name] = resolved
		}
	}
	return documents, nil
}

func sourceGroups(documents map[string]string, state domain.State) map[string]agentSource {
	names := make([]string, 0, len(documents))
	for name := range documents {
		names = append(names, name)
	}
	sort.Strings(names)
	sources := map[string]agentSource{}
	for _, filename := range names {
		sourceKey, writtenAt, stateIndex := "raw:"+filename, uint64(0), 0
		found := false
		for index, record := range state.Raw {
			if record.Filename == filename && (!found || record.WrittenAt > writtenAt || record.WrittenAt == writtenAt && index > stateIndex) {
				sourceKey, writtenAt, stateIndex = record.URL, record.WrittenAt, index
				found = true
			}
		}
		current, ok := sources[sourceKey]
		if !ok || writtenAt > current.LatestWrittenAt || writtenAt == current.LatestWrittenAt && stateIndex > current.LatestStateIndex {
			sources[sourceKey] = agentSource{LatestFilename: filename, LatestWrittenAt: writtenAt, LatestStateIndex: stateIndex}
		}
	}
	return sources
}

func executeToolCall(context *agentContext, call agent.ToolCall) (output string, returnErr error) {
	name := call.Function.Name
	writeDetails := map[string]any{}
	if name == toolWriteSummary {
		defer func() {
			for key, value := range operationErrorDetails(returnErr) {
				writeDetails[key] = value
			}
			recordOperation(OperationOptions{Log: context.operationLog, Actor: context.actor}, context.directory, CommandWriteSummary, returnErr == nil, writeDetails)
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
		if name == toolWriteSummary {
			writeDetails["source_key"] = sourceKey
		}
		markdown, err := stringArgument(arguments, "markdown")
		if err != nil {
			return "", fmt.Errorf("%s.markdown must be a string", name)
		}
		if _, ok := context.sources[sourceKey]; !ok {
			return "", fmt.Errorf("unknown source: %s", sourceKey)
		}
		if name == toolWriteSummary {
			writeDetails["derived_from"] = context.sources[sourceKey].LatestFilename
			writeDetails["filename"] = context.sources[sourceKey].LatestFilename
		}
		existing := summaryRecord(context.state, sourceKey)
		if name == toolWriteSummary && existing != nil {
			return "", fmt.Errorf("summary already exists for source: %s", sourceKey)
		}
		if name == toolEditSummary && existing == nil {
			return "", fmt.Errorf("no summary exists for source: %s", sourceKey)
		}
		if err := writeSummary(context, sourceKey, markdown, existing); err != nil {
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
	page, err := context.operationLog.Read(context.ctx, context.directory, offset, limit)
	if err != nil {
		return "", err
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
	for _, operation := range page.Entries {
		markCompletedFromOperation(context, operation)
	}
	data, err := json.Marshal(page)
	if err != nil {
		return "", fmt.Errorf("serializing operation log failed: %v", err)
	}
	return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
}

func markCompletedFromOperation(context *agentContext, operation Operation) {
	if !operation.Success || operation.Command != CommandWriteSummary {
		return
	}
	sourceKey, _ := operation.Details["source_key"].(string)
	filename, _ := operation.Details["derived_from"].(string)
	if filename == "" {
		filename, _ = operation.Details["filename"].(string)
	}
	if filename == "" {
		filename, _ = operation.Details["raw_filename"].(string)
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
	if filename == "" || filepath.Base(filename) != filename || strings.ContainsAny(filename, `/\\`) {
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
	path, ok := context.documents[filename]
	if !ok {
		return "", fmt.Errorf("unknown raw document: %s", filename)
	}
	return readBounded(path, context.maxOutputBytes)
}

func readBounded(path string, limit int) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", fmt.Errorf("reading %s failed: %v", path, err)
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, int64(limit)+4))
	if err != nil {
		return "", fmt.Errorf("reading %s failed: %v", path, err)
	}
	return agent.BoundedOutput(string(data), limit), nil
}

func readSummary(context *agentContext, sourceKey string) (string, error) {
	if _, ok := context.sources[sourceKey]; !ok {
		return "", fmt.Errorf("unknown source: %s", sourceKey)
	}
	record := summaryRecord(context.state, sourceKey)
	if record == nil {
		return "", fmt.Errorf("no summary exists for source: %s", sourceKey)
	}
	data, err := context.storage.ReadDocument(context.ctx, domain.SummaryRef(record.Filename))
	if err != nil {
		return "", err
	}
	return agent.BoundedOutput(string(data), context.maxOutputBytes), nil
}

func summaryRecord(state domain.State, sourceKey string) *domain.SummaryRecord {
	for index := range state.Summaries {
		if state.Summaries[index].SourceKey == sourceKey {
			return &state.Summaries[index]
		}
	}
	return nil
}

func writeSummary(context *agentContext, sourceKey, markdown string, existing *domain.SummaryRecord) error {
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
	createdAt := uint64(0)
	if existing != nil {
		filename = existing.Filename
		createdAt = existing.CreatedAt
	}
	if err := context.storage.ReplaceSummary(context.ctx, domain.SummaryRef(filename), []byte(markdown)); err != nil {
		return err
	}
	now, err := nowNanos()
	if err != nil {
		return err
	}
	updatedAt := now
	if existing != nil {
		updatedAt = existing.UpdatedAt + 1
		if now > updatedAt {
			updatedAt = now
		}
	}
	record := domain.SummaryRecord{Filename: filename, SourceKey: sourceKey, DerivedFrom: source.LatestFilename, CreatedAt: createdAt, UpdatedAt: updatedAt}
	if existing == nil {
		record.CreatedAt = now
	}
	next := context.state
	next.Summaries = append([]domain.SummaryRecord{}, context.state.Summaries...)
	replaced := false
	for index := range next.Summaries {
		if next.Summaries[index].SourceKey == sourceKey {
			next.Summaries[index] = record
			replaced = true
			break
		}
	}
	if !replaced {
		next.Summaries = append(next.Summaries, record)
	}
	generation, err := context.storage.PublishState(context.ctx, next, context.generation)
	if err != nil {
		return err
	}
	context.state, context.generation = next, generation
	return nil
}

func ensureInside(path, root string) error {
	relative, err := filepath.Rel(root, path)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) || filepath.IsAbs(relative) {
		return fmt.Errorf("path escapes %s: %s", root, path)
	}
	return nil
}

func nowNanos() (uint64, error) {
	now := time.Now().UnixNano()
	if now < 0 {
		return 0, fmt.Errorf("clock returned a time before Unix epoch")
	}
	return uint64(now), nil
}
