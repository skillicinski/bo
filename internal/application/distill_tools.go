package application

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const (
	toolReadDistillation  = "read_distillation"
	toolWriteDistillation = "write_distillation"
	toolEditDistillation  = "edit_distillation"
	toolSkipDistill       = "skip_distill"
)

var allDistillTools = []string{toolReadCorpus, toolReadLogs, toolReadDocument, toolReadSummary, toolReadDistillation, toolWriteDistillation, toolEditDistillation, toolSkipDistill}

type distillCatalog struct {
	sources       map[string]agentSource
	documents     map[string]distillDocument
	distillations map[string]domain.DistillationRecord
	state         domain.State
}

type distillDocument struct {
	SourceKey string
	Ref       domain.DocumentRef
}

type distillContext struct {
	ctx                context.Context
	directory          string
	workspace          Workspace
	options            OperationOptions
	catalog            distillCatalog
	state              domain.State
	revision           Revision
	maxOutputBytes     int
	readDocuments      map[string][]byte
	readRefs           map[string]bool
	readDistillations  map[string]bool
	summaryToolEnabled bool
	mutationOps        map[string]Operation
	committed          []Operation
	filenames          []string
	filename           string
	reason             string
	equivalent         bool
	skipped            bool
	logEvents          []Operation
	logWindowLoaded    bool
	eventFailure       error
}

type distillSourceReference struct {
	SourceKey string              `json:"source_key"`
	Kind      domain.DocumentKind `json:"kind"`
	Filename  string              `json:"filename"`
}

type distillSection struct {
	Heading   string                   `json:"heading"`
	Paragraph string                   `json:"paragraph"`
	Bullets   []string                 `json:"bullets"`
	Sources   []distillSourceReference `json:"sources"`
}

type distillWriteArguments struct {
	Topic        string           `json:"topic"`
	Title        string           `json:"title"`
	Introduction string           `json:"introduction"`
	Sections     []distillSection `json:"sections"`
	Filename     string           `json:"filename,omitempty"`
}

func normalizeDistillTools(names []string) ([]string, error) {
	if len(names) == 0 || len(names) == 1 && names[0] == "all" {
		return append([]string{}, allDistillTools...), nil
	}
	known := make(map[string]bool, len(allDistillTools))
	for _, name := range allDistillTools {
		known[name] = true
	}
	seen := make(map[string]bool, len(names))
	validated := make([]string, 0, len(names))
	for _, name := range names {
		if !known[name] {
			return nil, fmt.Errorf("unknown distill tool: %s", name)
		}
		if seen[name] {
			return nil, fmt.Errorf("duplicate distill tool: %s", name)
		}
		seen[name] = true
		validated = append(validated, name)
	}
	if !seen[toolSkipDistill] {
		return nil, fmt.Errorf("distill tool set must include %s", toolSkipDistill)
	}
	return validated, nil
}

func distillTools(contextState *distillContext, names []string) []agent.Tool {
	objectParameters := func(properties map[string]any, required []string) map[string]any {
		return map[string]any{"type": "object", "properties": properties, "required": required, "additionalProperties": false}
	}
	execute := func(_ context.Context, call agent.ToolCall) (string, error) {
		return executeDistillTool(contextState, call)
	}
	definitions := map[string]agent.Tool{
		toolReadCorpus:       {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolReadCorpus, Description: "Read the current raw snapshots and current summaries available for cross-source evidence.", Parameters: objectParameters(map[string]any{}, []string{})}}, Execute: execute},
		toolReadLogs:         {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolReadLogs, Description: "Read paginated operation log entries for the current directory, newest first.", Parameters: objectParameters(map[string]any{"offset": map[string]any{"type": "integer", "default": 0, "minimum": 0}, "limit": map[string]any{"type": "integer", "default": 20, "minimum": 1, "maximum": 100}}, []string{})}}, Execute: execute},
		toolReadDocument:     {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolReadDocument, Description: "Read one newest raw Markdown snapshot by its exact filename.", Parameters: objectParameters(map[string]any{"filename": map[string]any{"type": "string"}}, []string{"filename"})}}, Execute: execute},
		toolReadSummary:      {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolReadSummary, Description: "Read the current summary for one source identity when it derives from that source's newest raw snapshot.", Parameters: objectParameters(map[string]any{"source_key": map[string]any{"type": "string"}}, []string{"source_key"})}}, Execute: execute},
		toolReadDistillation: {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolReadDistillation, Description: "Read an existing distillation candidate for topic matching. It is not evidence.", Parameters: objectParameters(map[string]any{"filename": map[string]any{"type": "string"}}, []string{"filename"})}}, Execute: execute},
		toolWriteDistillation: {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolWriteDistillation, Description: "Create one cross-source Markdown distill document with a title, introduction, structured sections, bullets, and source references.", Parameters: objectParameters(map[string]any{
			"topic":        map[string]any{"type": "string"},
			"filename":     map[string]any{"type": "string"},
			"title":        map[string]any{"type": "string"},
			"introduction": map[string]any{"type": "string"},
			"sections": map[string]any{
				"type":     "array",
				"minItems": 1,
				"items": map[string]any{
					"type": "object",
					"properties": map[string]any{
						"heading":   map[string]any{"type": "string"},
						"paragraph": map[string]any{"type": "string"},
						"bullets":   map[string]any{"type": "array", "minItems": 2, "items": map[string]any{"type": "string"}},
						"sources": map[string]any{
							"type":     "array",
							"minItems": 1,
							"items": map[string]any{
								"type": "object",
								"properties": map[string]any{
									"source_key": map[string]any{"type": "string"},
									"kind":       map[string]any{"type": "string", "enum": []string{"raw", "summary"}},
									"filename":   map[string]any{"type": "string"},
								},
								"required":             []string{"source_key", "kind", "filename"},
								"additionalProperties": false,
							},
						},
					},
					"required":             []string{"heading", "paragraph", "bullets", "sources"},
					"additionalProperties": false,
				},
			},
		}, []string{"topic", "title", "introduction", "sections"})}}, Execute: execute},
		toolSkipDistill: {Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: toolSkipDistill, Description: "Report that no useful theme is supported by at least two distinct source identities.", Parameters: objectParameters(map[string]any{"reason": map[string]any{"type": "string"}}, []string{"reason"})}}, Execute: execute},
	}
	edit := definitions[toolWriteDistillation]
	edit.Definition.Function.Name = toolEditDistillation
	edit.Definition.Function.Description = "Edit an existing distillation document after reading it and merging newly relevant evidence."
	definitions[toolEditDistillation] = edit
	tools := make([]agent.Tool, 0, len(names))
	for _, name := range names {
		tools = append(tools, definitions[name])
	}
	return tools
}

func buildDistillCatalog(ctx context.Context, workspace Workspace, state domain.State) (distillCatalog, error) {
	documents, err := DiscoverDocuments(ctx, workspace)
	if err != nil {
		return distillCatalog{}, err
	}
	groups := sourceGroups(documents, state)
	catalog := distillCatalog{sources: map[string]agentSource{}, documents: map[string]distillDocument{}, distillations: map[string]domain.DistillationRecord{}}
	filtered := domain.State{Sources: []domain.SourceRecord{}}
	for _, record := range state.DistillationDocuments {
		catalog.distillations[record.Filename] = record
	}
	for _, source := range state.Sources {
		latest, ok := groups[source.SourceKey]
		if !ok {
			continue
		}
		var latestRecord domain.RawRecord
		for _, snapshot := range source.Snapshots {
			if snapshot.Filename == latest.LatestFilename {
				latestRecord = snapshot
				break
			}
		}
		if latestRecord.Filename == "" {
			continue
		}
		catalog.sources[source.SourceKey] = latest
		catalog.documents[distillDocumentKey(domain.DocumentKindRaw, latest.LatestFilename)] = distillDocument{SourceKey: source.SourceKey, Ref: domain.RawRef(latest.LatestFilename)}
		filteredSource := domain.SourceRecord{SourceKey: source.SourceKey, Snapshots: []domain.RawRecord{latestRecord}}
		if source.Summary != nil && source.Summary.DerivedFrom == latest.LatestFilename {
			summary := *source.Summary
			filteredSource.Summary = &summary
			catalog.documents[distillDocumentKey(domain.DocumentKindSummary, summary.Filename)] = distillDocument{SourceKey: source.SourceKey, Ref: domain.SummaryRef(summary.Filename)}
		}
		filtered.Sources = append(filtered.Sources, filteredSource)
	}
	sort.Slice(filtered.Sources, func(i, j int) bool { return filtered.Sources[i].SourceKey < filtered.Sources[j].SourceKey })
	catalog.state = filtered
	return catalog, nil
}

func distillDocumentKey(kind domain.DocumentKind, filename string) string {
	return string(kind) + "\x00" + filename
}

func distillSystemPrompt(contextState *distillContext, readLogsEnabled bool) string {
	keys := make([]string, 0, len(contextState.catalog.sources))
	for sourceKey := range contextState.catalog.sources {
		keys = append(keys, sourceKey)
	}
	sort.Strings(keys)
	logInstruction := ""
	if readLogsEnabled {
		logInstruction = " Before reading evidence, inspect recent scoped operation entries, including failed attempts, with read_logs."
	}
	distillations := make([]string, 0, len(contextState.catalog.distillations))
	for filename, record := range contextState.catalog.distillations {
		label := filename
		if record.Topic != "" {
			label = record.Topic + " (" + filename + ")"
		}
		distillations = append(distillations, label)
	}
	sort.Strings(distillations)
	existingInstruction := " No existing distillation candidates are available."
	if len(distillations) > 0 {
		existingInstruction = fmt.Sprintf(" Existing distillation candidates are available by topic and filename: %s. Read a candidate before editing it; candidates are not evidence.", strings.Join(distillations, ", "))
	}
	summaries := make([]string, 0, len(contextState.catalog.state.Sources))
	for _, source := range contextState.catalog.state.Sources {
		if source.Summary != nil {
			summaries = append(summaries, source.SourceKey+" ("+source.Summary.Filename+")")
		}
	}
	summaryInstruction := " Current summaries: none."
	if len(summaries) > 0 {
		summaryInstruction = fmt.Sprintf(" Current summaries: %s.", strings.Join(summaries, ", "))
	}
	surveyInstruction := ""
	if contextState.summaryToolEnabled && len(summaries) >= 2 && len(contextState.catalog.distillations) == 0 {
		surveyInstruction = " There are no existing distillation documents. Before deciding to skip, read current summaries for at least two distinct source identities with read_summary, use them to select a promising cross-source theme, and do not skip after reading only the log, corpus index, or one source."
	}
	taskInstruction := "Process every useful unprocessed theme supported by at least two distinct source identities. Use write_distillation once for each new topic or edit_distillation once for each existing topic with changed evidence after reading it. If current evidence exactly matches an existing topic, skip that topic and continue. Call skip_distill exactly once only when no unprocessed theme remains."
	return fmt.Sprintf("You are bo's cross-source distill agent for %s. %s Use only the current raw snapshots and summaries exposed by the host as evidence; stale summaries and distillation documents are not evidence. Preserve qualifications, uncertainty, authorship, measurements, recommendations, opinions, and forecasts. The host owns document access, current-snapshot selection, provenance, filename allocation, and publication.%s%s%s Available source identities: %s.", contextState.directory, taskInstruction, logInstruction, existingInstruction, summaryInstruction+surveyInstruction, strings.Join(keys, ", "))
}

func executeDistillTool(contextState *distillContext, call agent.ToolCall) (output string, returnErr error) {
	name := call.Function.Name
	var mutation *Operation
	mutationKey := ""
	commitFailureRecorded := false
	if name == toolWriteDistillation || name == toolEditDistillation {
		mutationKey = distillMutationKey(call)
		event := contextState.nextMutationOperation(mutationKey)
		mutation = &event
		defer func() {
			if returnErr == nil || commitFailureRecorded {
				return
			}
			if eventErr := recordFailedOperation(contextState.ctx, contextState.workspace, *mutation, returnErr); eventErr != nil {
				contextState.eventFailure = errors.Join(contextState.eventFailure, eventErr)
			}
		}()
	}
	arguments, err := objectArguments(call.Function.Arguments)
	if err != nil {
		return "", fmt.Errorf("%s arguments are malformed JSON: %v", name, err)
	}
	switch name {
	case toolReadCorpus:
		if len(arguments) != 0 {
			return "", fmt.Errorf("read_corpus arguments must be empty")
		}
		data, err := json.MarshalIndent(contextState.catalog.state, "", "  ")
		if err != nil {
			return "", fmt.Errorf("serializing state failed: %v", err)
		}
		return agent.BoundedOutput(string(data), contextState.maxOutputBytes), nil
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
		if err != nil || limit < 1 || limit > MaxOperationPageLimit {
			return "", fmt.Errorf("read_logs.limit must be an integer from 1 to %d", MaxOperationPageLimit)
		}
		return readDistillLogs(contextState, offset, limit)
	case toolReadDocument:
		if len(arguments) != 1 {
			return "", fmt.Errorf("read_document arguments must contain only filename")
		}
		filename, err := stringArgument(arguments, "filename")
		if err != nil {
			return "", fmt.Errorf("read_document.filename must be a string")
		}
		return readDistillDocument(contextState, filename)
	case toolReadSummary:
		if len(arguments) != 1 {
			return "", fmt.Errorf("read_summary arguments must contain only source_key")
		}
		sourceKey, err := stringArgument(arguments, "source_key")
		if err != nil {
			return "", fmt.Errorf("read_summary.source_key must be a string")
		}
		return readDistillSummary(contextState, sourceKey)
	case toolReadDistillation:
		if len(arguments) != 1 {
			return "", fmt.Errorf("read_distillation arguments must contain only filename")
		}
		filename, err := stringArgument(arguments, "filename")
		if err != nil {
			return "", fmt.Errorf("read_distillation.filename must be a string")
		}
		return readDistillation(contextState, filename)
	case toolWriteDistillation, toolEditDistillation:
		if contextState.skipped {
			return "", fmt.Errorf("distill already has a terminal result")
		}
		var write distillWriteArguments
		if err := decodeStrictArguments(call.Function.Arguments, &write); err != nil {
			return "", fmt.Errorf("%s arguments are malformed: %v", name, err)
		}
		if name == toolEditDistillation && strings.TrimSpace(write.Filename) == "" {
			return "", fmt.Errorf("edit_distillation.filename must be non-empty")
		}
		if err := validateDistillWrite(contextState, write); err != nil {
			return "", err
		}
		if mutation == nil {
			return "", fmt.Errorf("write_distillation mutation is not configured")
		}
		filename, err, recorded := writeDistillation(contextState, write, *mutation, mutationKey)
		commitFailureRecorded = recorded
		if err != nil {
			return "", err
		}
		if contextState.equivalent {
			contextState.equivalent = false
			return name + " skipped: an equivalent distillation already exists; continue with another topic", nil
		}
		contextState.filename = filename
		contextState.filenames = append(contextState.filenames, filename)
		return name + " succeeded: " + filename, nil
	case toolSkipDistill:
		if len(arguments) != 1 {
			return "", fmt.Errorf("skip_distill arguments must contain only reason")
		}
		reason, err := stringArgument(arguments, "reason")
		if err != nil || strings.TrimSpace(reason) == "" {
			return "", fmt.Errorf("skip_distill.reason must be a non-empty string")
		}
		if contextState.skipped {
			return "", fmt.Errorf("distill already has a terminal result")
		}
		if len(contextState.catalog.distillations) == 0 && len(contextState.filenames) == 0 {
			if contextState.summaryToolEnabled && contextState.availableSourceCount(domain.DocumentKindSummary) >= 2 {
				if contextState.readSourceCount(domain.DocumentKindSummary) < 2 {
					return "", fmt.Errorf("skip_distill requires current summaries from at least two distinct source identities before the first distillation; read_summary for another source before skipping")
				}
			} else if contextState.readSourceCount("") < 2 {
				return "", fmt.Errorf("skip_distill requires evidence from at least two distinct source identities before the first distillation; read current summaries or raw documents first")
			}
		}
		contextState.skipped = true
		contextState.reason = strings.TrimSpace(reason)
		return "skip_distill succeeded", nil
	default:
		return "", fmt.Errorf("unsupported tool: %s", name)
	}
}

func (contextState *distillContext) availableSourceCount(kind domain.DocumentKind) int {
	sources := map[string]bool{}
	for _, document := range contextState.catalog.documents {
		if kind == "" || document.Ref.Kind == kind {
			sources[document.SourceKey] = true
		}
	}
	return len(sources)
}

func (contextState *distillContext) readSourceCount(kind domain.DocumentKind) int {
	sources := map[string]bool{}
	for key := range contextState.readRefs {
		document, ok := contextState.catalog.documents[key]
		if ok && (kind == "" || document.Ref.Kind == kind) {
			sources[document.SourceKey] = true
		}
	}
	return len(sources)
}

func (contextState *distillContext) nextMutationOperation(key string) Operation {
	operation, ok := contextState.mutationOps[key]
	if !ok {
		operation = newOperation(CommandWriteDistillation, contextState.options.Actor)
	} else {
		operation.Attempt++
		operation.Timestamp = time.Now().UTC().Format(time.RFC3339Nano)
	}
	contextState.mutationOps[key] = operation
	return operation
}

func distillMutationKey(call agent.ToolCall) string {
	var arguments struct {
		Topic    string `json:"topic"`
		Title    string `json:"title"`
		Filename string `json:"filename"`
	}
	if err := json.Unmarshal([]byte(call.Function.Arguments), &arguments); err == nil {
		key := strings.TrimSpace(arguments.Topic)
		if key == "" {
			key = strings.TrimSpace(arguments.Title)
		}
		if key == "" {
			key = strings.TrimSpace(arguments.Filename)
		}
		if key != "" {
			if canonical, err := normalizeDistillTopic(key); err == nil {
				return canonical
			}
			return key
		}
	}
	if call.ID != "" {
		return call.ID
	}
	return toolWriteDistillation
}

func objectArguments(raw string) (map[string]json.RawMessage, error) {
	var arguments map[string]json.RawMessage
	if err := json.Unmarshal([]byte(raw), &arguments); err != nil {
		return nil, err
	}
	if arguments == nil {
		return nil, errors.New("arguments must be a JSON object")
	}
	return arguments, nil
}

func decodeStrictArguments(raw string, target any) error {
	decoder := json.NewDecoder(strings.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		if err != nil {
			return err
		}
		return errors.New("arguments contain multiple JSON values")
	}
	return nil
}

func readDistillLogs(contextState *distillContext, offset, limit int) (string, error) {
	view := &agentContext{directory: contextState.directory, maxOutputBytes: contextState.maxOutputBytes, logEvents: contextState.logEvents, logWindowLoaded: contextState.logWindowLoaded}
	return readLogs(view, offset, limit)
}

func readDistillDocument(contextState *distillContext, filename string) (string, error) {
	filename = strings.TrimPrefix(filename, "raw/")
	if filename == "" || strings.ContainsAny(filename, `/\\`) || domain.ValidateDocumentName(filename) != nil {
		return "", fmt.Errorf("read_document.filename must be a raw Markdown filename")
	}
	key := distillDocumentKey(domain.DocumentKindRaw, filename)
	document, ok := contextState.catalog.documents[key]
	if !ok {
		return "", fmt.Errorf("document is not an available newest raw snapshot: %s", filename)
	}
	return readDistillDocumentRef(contextState, key, document)
}

func readDistillSummary(contextState *distillContext, sourceKey string) (string, error) {
	if _, ok := contextState.catalog.sources[sourceKey]; !ok {
		return "", fmt.Errorf("unknown source: %s", sourceKey)
	}
	for key, document := range contextState.catalog.documents {
		if document.SourceKey == sourceKey && document.Ref.Kind == domain.DocumentKindSummary {
			return readDistillDocumentRef(contextState, key, document)
		}
	}
	return "", fmt.Errorf("no current summary exists for source: %s", sourceKey)
}

func readDistillation(contextState *distillContext, filename string) (string, error) {
	filename = strings.TrimPrefix(filename, "distillations/")
	if filename == "" || strings.ContainsAny(filename, `/\\`) || domain.ValidateDocumentName(filename) != nil {
		return "", fmt.Errorf("read_distillation.filename must be a distillation Markdown filename")
	}
	if _, ok := contextState.catalog.distillations[filename]; !ok {
		return "", fmt.Errorf("unknown distillation document: %s", filename)
	}
	data, err := contextState.workspace.ReadDocument(contextState.ctx, domain.DistillationRef(filename))
	if err != nil {
		return "", normalizeError(err, internalerrors.KindFilesystem, "reading distillation")
	}
	if len(data) > contextState.maxOutputBytes {
		return "", fmt.Errorf("distillation document is too large to edit safely (max %d bytes)", contextState.maxOutputBytes)
	}
	contextState.readDistillations[filename] = true
	return string(data), nil
}

func readDistillDocumentRef(contextState *distillContext, key string, document distillDocument) (string, error) {
	data, err := contextState.workspace.ReadDocument(contextState.ctx, document.Ref)
	if err != nil {
		return "", normalizeError(err, internalerrors.KindFilesystem, "reading distill document")
	}
	contextState.readRefs[key] = true
	contextState.readDocuments[key] = append([]byte(nil), data...)
	return agent.BoundedOutput(string(data), contextState.maxOutputBytes), nil
}

func validateDistillWrite(contextState *distillContext, write distillWriteArguments) error {
	if strings.TrimSpace(write.Title) == "" {
		return fmt.Errorf("write_distillation.title must be non-empty")
	}
	if strings.TrimSpace(write.Topic) == "" {
		return fmt.Errorf("write_distillation.topic must be non-empty")
	}
	if _, err := normalizeDistillTopic(write.Topic); err != nil {
		return fmt.Errorf("write_distillation.topic is invalid: %v", err)
	}
	if strings.TrimSpace(write.Introduction) == "" {
		return fmt.Errorf("write_distillation.introduction must be non-empty")
	}
	if len(write.Sections) == 0 {
		return fmt.Errorf("write_distillation.sections must not be empty")
	}
	distinctSources := map[string]bool{}
	for index, section := range write.Sections {
		if strings.TrimSpace(section.Heading) == "" {
			return fmt.Errorf("write_distillation.sections[%d].heading must be non-empty", index)
		}
		if strings.TrimSpace(section.Paragraph) == "" {
			return fmt.Errorf("write_distillation.sections[%d].paragraph must be non-empty", index)
		}
		if len(section.Bullets) < 2 {
			return fmt.Errorf("write_distillation.sections[%d].bullets must contain at least two items", index)
		}
		for bulletIndex, bullet := range section.Bullets {
			if strings.TrimSpace(bullet) == "" {
				return fmt.Errorf("write_distillation.sections[%d].bullets[%d] must be non-empty", index, bulletIndex)
			}
		}
		if len(section.Sources) == 0 {
			return fmt.Errorf("write_distillation.sections[%d].sources must not be empty", index)
		}
		for referenceIndex, reference := range section.Sources {
			if reference.Kind != domain.DocumentKindRaw && reference.Kind != domain.DocumentKindSummary {
				return fmt.Errorf("write_distillation.sections[%d].sources[%d].kind is invalid", index, referenceIndex)
			}
			if err := domain.ValidateSourceKey(reference.SourceKey); err != nil {
				return fmt.Errorf("write_distillation.sections[%d].sources[%d].source_key is invalid", index, referenceIndex)
			}
			if err := domain.ValidateDocumentName(reference.Filename); err != nil {
				return fmt.Errorf("write_distillation.sections[%d].sources[%d].filename is invalid", index, referenceIndex)
			}
			key := distillDocumentKey(reference.Kind, reference.Filename)
			document, ok := contextState.catalog.documents[key]
			if !ok || document.SourceKey != reference.SourceKey {
				return fmt.Errorf("write_distillation.sections[%d].sources[%d] is not an available document for its source", index, referenceIndex)
			}
			if !contextState.readRefs[key] {
				return fmt.Errorf("write_distillation.sections[%d].sources[%d] was not read", index, referenceIndex)
			}
			distinctSources[reference.SourceKey] = true
		}
	}
	if len(distinctSources) < 2 {
		return fmt.Errorf("write_distillation.sources must contain at least two distinct source identities")
	}
	return nil
}

func normalizeDistillTopic(topic string) (string, error) {
	topic = strings.TrimSpace(topic)
	return domain.CanonicalTopic(topic)
}

func writeDistillation(contextState *distillContext, write distillWriteArguments, operation Operation, mutationKey string) (string, error, bool) {
	contextState.equivalent = false
	topic, err := normalizeDistillTopic(write.Topic)
	if err != nil {
		return "", err, false
	}
	markdown := renderDistill(write)
	if len(markdown) > contextState.maxOutputBytes {
		return "", fmt.Errorf("distill exceeds max tool output bytes (%d)", contextState.maxOutputBytes), false
	}
	slug, err := KebabCase(write.Title)
	if err != nil {
		return "", err, false
	}
	target := strings.TrimSpace(write.Filename)
	var existing *domain.DistillationRecord
	if target != "" {
		if err := domain.ValidateDocumentName(target); err != nil {
			return "", fmt.Errorf("distillation filename is invalid: %v", err), false
		}
		record, ok := contextState.catalog.distillations[target]
		if !ok {
			return "", fmt.Errorf("no existing distillation document: %s", target), false
		}
		if !contextState.readDistillations[target] {
			return "", fmt.Errorf("existing distillation document was not read: %s", target), false
		}
		existing = &record
		if existing.Topic != "" && existing.Topic != topic {
			return "", fmt.Errorf("distillation topic %q does not match existing topic %q", topic, existing.Topic), false
		}
	}
	inputs := distillInputs(contextState, write)
	if equivalentDistillation(contextState.catalog.distillations, topic, inputs) {
		contextState.equivalent = true
		contextState.reason = "an equivalent distillation already exists"
		return "", nil, true
	}
	if existing == nil {
		for filename, record := range contextState.catalog.distillations {
			if record.Topic == topic {
				return "", fmt.Errorf("distillation topic already exists in %s; use edit_distillation", filename), false
			}
		}
	}
	createdAt := time.Now().UTC()
	if existing != nil {
		createdAt = existing.CreatedAt
	}
	for attempt := 1; attempt <= maxRawCommitAttempts; attempt++ {
		filename, update := target, existing != nil
		if existing == nil {
			filename = slug + ".md"
			if attempt == 2 {
				filename = fmt.Sprintf("%s--%d.md", slug, createdAt.UnixNano())
			} else if attempt > 2 {
				filename = fmt.Sprintf("%s--%d--%d.md", slug, createdAt.UnixNano(), attempt)
			}
		}
		attemptOperation := operation
		attemptOperation.Attempt = operation.Attempt + attempt - 1
		attemptOperation.Document = &domain.DocumentIdentity{Kind: domain.DocumentKindDistillation, Filename: filename}
		contextState.mutationOps[mutationKey] = attemptOperation
		if err := contextState.ctx.Err(); err != nil {
			contextErr := internalerrors.Context(err)
			if eventErr := recordFailedOperation(contextState.ctx, contextState.workspace, attemptOperation, contextErr); eventErr != nil {
				contextState.eventFailure = errors.Join(contextState.eventFailure, eventErr)
				return "", errors.Join(contextErr, eventErr), true
			}
			return "", contextErr, true
		}
		committed := committedOperation(attemptOperation)
		state, revision, err := contextState.workspace.CommitDistillation(contextState.ctx, DistillationCommit{
			Kind: domain.DocumentKindDistillation, Filename: filename, Topic: topic, Update: update, CreatedAt: createdAt, UpdatedAt: time.Now().UTC(),
			DerivedFrom: inputs, Contents: []byte(markdown), Event: committed,
		}, contextState.revision)
		if err == nil {
			contextState.state, contextState.revision = state, revision
			contextState.catalog.distillations = make(map[string]domain.DistillationRecord, len(state.DistillationDocuments))
			for _, record := range state.DistillationDocuments {
				contextState.catalog.distillations[record.Filename] = record
			}
			contextState.committed = append(contextState.committed, committed)
			return filename, nil, true
		}
		if eventErr := recordFailedOperation(contextState.ctx, contextState.workspace, attemptOperation, err); eventErr != nil {
			contextState.eventFailure = errors.Join(contextState.eventFailure, eventErr)
			return "", errors.Join(err, eventErr), true
		}
		if !internalerrors.IsAlreadyExists(err) {
			return "", err, true
		}
	}
	return "", internalerrors.Wrap(internalerrors.KindAlreadyExists, "distillation document filename attempts exhausted", internalerrors.ErrAlreadyExists), true
}

func equivalentDistillation(records map[string]domain.DistillationRecord, topic string, inputs []domain.DistillationInput) bool {
	want := make(map[string]bool, len(inputs))
	for _, input := range inputs {
		want[distillationInputKey(input)] = true
	}
	for _, record := range records {
		if record.Topic != topic {
			continue
		}
		if len(record.DerivedFrom) != len(inputs) {
			continue
		}
		match := true
		for _, input := range record.DerivedFrom {
			if !want[distillationInputKey(input)] {
				match = false
				break
			}
		}
		if match {
			return true
		}
	}
	return false
}

func distillationInputKey(input domain.DistillationInput) string {
	return input.SourceKey + "\x00" + string(input.Kind) + "\x00" + input.Filename + "\x00" + input.ContentDigest
}

func distillInputs(contextState *distillContext, write distillWriteArguments) []domain.DistillationInput {
	inputs := make([]domain.DistillationInput, 0)
	seen := map[string]bool{}
	for _, section := range write.Sections {
		for _, reference := range section.Sources {
			key := distillDocumentKey(reference.Kind, reference.Filename)
			if seen[key] {
				continue
			}
			seen[key] = true
			digest := sha256.Sum256(contextState.readDocuments[key])
			inputs = append(inputs, domain.DistillationInput{
				SourceKey: reference.SourceKey, Kind: reference.Kind, Filename: reference.Filename,
				ContentDigest: hex.EncodeToString(digest[:]),
			})
		}
	}
	return inputs
}

func renderDistill(write distillWriteArguments) string {
	var builder strings.Builder
	title := strings.TrimSpace(write.Title)
	builder.WriteString("# ")
	builder.WriteString(title)
	builder.WriteString("\n\n")
	builder.WriteString(strings.TrimSpace(write.Introduction))
	builder.WriteString("\n\n")
	allReferences := make([]distillSourceReference, 0)
	seen := map[string]bool{}
	for _, section := range write.Sections {
		builder.WriteString("## ")
		builder.WriteString(strings.TrimSpace(section.Heading))
		builder.WriteString("\n\n")
		builder.WriteString(strings.TrimSpace(section.Paragraph))
		builder.WriteString("\n\n")
		for _, bullet := range section.Bullets {
			builder.WriteString("- ")
			builder.WriteString(strings.TrimSpace(bullet))
			builder.WriteByte('\n')
		}
		builder.WriteByte('\n')
		builder.WriteString("Sources: ")
		for index, reference := range section.Sources {
			if index > 0 {
				builder.WriteString(", ")
			}
			builder.WriteString(distillLink(reference))
			key := distillDocumentKey(reference.Kind, reference.Filename) + "\x00" + reference.SourceKey
			if !seen[key] {
				seen[key] = true
				allReferences = append(allReferences, reference)
			}
		}
		builder.WriteString("\n\n")
	}
	builder.WriteString("## Sources\n\n")
	for _, reference := range allReferences {
		builder.WriteString("- ")
		builder.WriteString(distillLink(reference))
		builder.WriteString(" — ")
		builder.WriteString(reference.SourceKey)
		builder.WriteByte('\n')
	}
	return builder.String()
}

func distillLink(reference distillSourceReference) string {
	path := "../" + reference.Filename
	if reference.Kind == domain.DocumentKindSummary {
		path = "../summaries/" + reference.Filename
	}
	return "[" + reference.Filename + "](" + path + ")"
}
