package application_test

import (
	"context"
	"fmt"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/skillicinski/bo/internal/agent"
	"github.com/skillicinski/bo/internal/application"
	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

type batchingProvider struct {
	sources       map[string]string
	includeLogs   bool
	includeCorpus bool
	includeDoc    bool
	provideUsage  bool
	failBatch     int
	blockBatch    int
	failure       error
	batch         int
	current       []string
	logsRead      bool
	corpusRead    bool
	read          map[string]bool
	written       map[string]bool
	batches       [][]string
	requests      []agent.CompletionRequest
	deadlines     []time.Time
}

type countingWorkspace struct {
	application.Workspace
	eventReads   int
	recentReads  int
	stateReads   int
	eventReadErr error
}

func (w *countingWorkspace) ReadEvents(ctx context.Context, offset, limit int) (application.OperationPage, error) {
	w.eventReads++
	if w.eventReadErr != nil {
		return application.OperationPage{}, w.eventReadErr
	}
	return w.Workspace.ReadEvents(ctx, offset, limit)
}

func (w *countingWorkspace) ReadRecentEvents(ctx context.Context, limit int) ([]application.Operation, error) {
	w.recentReads++
	return w.Workspace.ReadRecentEvents(ctx, limit)
}

func (w *countingWorkspace) ReadState(ctx context.Context) (domain.State, application.Revision, error) {
	w.stateReads++
	return w.Workspace.ReadState(ctx)
}

func (p *batchingProvider) Complete(ctx context.Context, request agent.CompletionRequest) (agent.CompletionResponse, error) {
	request.Messages = append([]agent.ChatMessage{}, request.Messages...)
	p.requests = append(p.requests, request)
	if deadline, ok := ctx.Deadline(); ok {
		p.deadlines = append(p.deadlines, deadline)
	}
	if len(request.Messages) == 2 {
		p.batch++
		p.current = p.requestSources(request)
		p.batches = append(p.batches, append([]string{}, p.current...))
		p.logsRead = false
		p.corpusRead = false
		p.read = map[string]bool{}
		p.written = map[string]bool{}
		if p.failBatch == p.batch {
			return agent.CompletionResponse{}, p.failure
		}
		if p.blockBatch == p.batch {
			<-ctx.Done()
			return agent.CompletionResponse{}, ctx.Err()
		}
	}
	if p.includeLogs && !p.logsRead {
		p.logsRead = true
		return p.toolResponse("read_logs", "{}"), nil
	}
	for _, sourceKey := range p.current {
		if p.includeCorpus && !p.corpusRead {
			p.corpusRead = true
			return p.toolResponse("read_corpus", "{}"), nil
		}
		if p.includeDoc && !p.read[sourceKey] {
			p.read[sourceKey] = true
			return p.toolResponse("read_document", fmt.Sprintf(`{"filename":"%s"}`, p.sources[sourceKey])), nil
		}
		if !p.written[sourceKey] {
			p.written[sourceKey] = true
			return p.toolResponse("write_summary", fmt.Sprintf(`{"source_key":"%s","markdown":"summary for %s\n"}`, sourceKey, sourceKey)), nil
		}
	}
	return agent.CompletionResponse{}, fmt.Errorf("provider has no pending source")
}

func (p *batchingProvider) requestSources(request agent.CompletionRequest) []string {
	content := ""
	if value, ok := request.Messages[1].Content.(string); ok {
		content = value
	}
	keys := make([]string, 0, len(p.sources))
	for sourceKey := range p.sources {
		if strings.Contains(content, sourceKey) {
			keys = append(keys, sourceKey)
		}
	}
	sort.Strings(keys)
	return keys
}

func (p *batchingProvider) toolResponse(name, arguments string) agent.CompletionResponse {
	response := agent.CompletionResponse{Message: agent.ChatMessage{
		Role: "assistant",
		ToolCalls: []agent.ToolCall{{
			ID:       fmt.Sprintf("call-%d-%d", p.batch, len(p.requests)),
			Function: agent.ToolFunction{Name: name, Arguments: arguments},
		}},
	}}
	if p.provideUsage {
		response.Usage = &agent.TokenUsage{PromptTokens: 1, CompletionTokens: 2, TotalTokens: 3}
	}
	return response
}

func testSources(t *testing.T, workspace application.Workspace, count int) map[string]string {
	t.Helper()
	sources := make(map[string]string, count)
	for index := 0; index < count; index++ {
		sourceKey := fmt.Sprintf("https://example.test/%02d", index)
		filename := fmt.Sprintf("source-%02d.md", index)
		commitRaw(t, workspace, sourceKey, filename, time.Unix(int64(index+1), 0).UTC(), []byte("raw "+sourceKey+"\n"))
		sources[sourceKey] = filename
	}
	return sources
}

func TestSynthesisBatchesInOrderAndIsolatesContext(t *testing.T) {
	store, target := seededStore(t)
	sources := testSources(t, store, 3)
	provider := &batchingProvider{sources: sources, includeLogs: true, includeCorpus: true, includeDoc: true, provideUsage: true}
	config := application.SynthesisOptions{MaxTurns: 4, MaxToolCalls: 5, MaxToolOutputBytes: 4096, MaxResponseTokens: 32, TimeoutSeconds: 5}

	result, err := application.SynthesizeWithTools(context.Background(), store, provider, config, []string{"read_logs", "read_corpus", "read_document", "write_summary"}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != len(sources) || result.SummariesSkipped != 0 {
		t.Fatalf("synthesis = %#v, error = %v", result, err)
	}
	if len(provider.batches) != len(sources) || result.Metrics.Turns != len(sources)*4 || result.Metrics.ToolCalls != len(sources)*4 || result.Metrics.Duration <= 0 {
		t.Fatalf("batches = %#v, metrics = %#v", provider.batches, result.Metrics)
	}
	if result.Metrics.Usage == nil || result.Metrics.Usage.PromptTokens != len(sources)*4 || result.Metrics.Usage.CompletionTokens != len(sources)*8 || result.Metrics.Usage.TotalTokens != len(sources)*12 {
		t.Fatalf("usage = %#v", result.Metrics.Usage)
	}
	want := make([]string, 0, len(sources))
	for sourceKey := range sources {
		want = append(want, sourceKey)
	}
	sort.Strings(want)
	for index, batch := range provider.batches {
		if len(batch) != 1 || batch[0] != want[index] {
			t.Fatalf("batch %d = %#v, want %q", index, batch, want[index])
		}
		request := provider.requests[index*4]
		if len(request.Messages) != 2 || strings.Contains(fmt.Sprint(request.Messages), want[(index+len(want)-1)%len(want)]) && index > 0 {
			t.Fatalf("batch %d carried prior messages: %#v", index, request.Messages)
		}
		corpusRequest := provider.requests[index*4+2]
		if strings.Contains(fmt.Sprint(corpusRequest.Messages), want[(index+1)%len(want)]) {
			t.Fatalf("batch %d corpus contains another source: %#v", index, corpusRequest.Messages)
		}
	}
}

func TestSynthesisSmallCorpusUsesOneRuntime(t *testing.T) {
	store, target := seededStore(t)
	sources := testSources(t, store, 2)
	provider := &batchingProvider{sources: sources, includeDoc: true, provideUsage: true}
	config := application.SynthesisOptions{MaxTurns: 10, MaxToolCalls: 10, MaxToolOutputBytes: 4096, MaxResponseTokens: 32, TimeoutSeconds: 5}

	result, err := application.Synthesize(context.Background(), store, provider, config, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 2 || len(provider.batches) != 1 || len(provider.batches[0]) != 2 {
		t.Fatalf("synthesis = %#v, batches = %#v, error = %v", result, provider.batches, err)
	}
	if result.Metrics.Usage == nil || result.Metrics.Usage.TotalTokens != 12 {
		t.Fatalf("usage = %#v", result.Metrics.Usage)
	}
}

func TestSynthesisBoundsStartupEventReadsAndKeepsReadLogs(t *testing.T) {
	store, target := seededStore(t)
	sources := testSources(t, store, 16)
	workspace := &countingWorkspace{Workspace: store}
	provider := &batchingProvider{sources: sources, includeLogs: true, includeDoc: true}
	config := application.SynthesisOptions{MaxTurns: 32, MaxToolCalls: 64, MaxToolOutputBytes: 8192, MaxResponseTokens: 32, TimeoutSeconds: 5}

	result, err := application.Synthesize(context.Background(), workspace, provider, config, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != len(sources) {
		t.Fatalf("synthesis = %#v, error = %v", result, err)
	}
	if len(provider.batches) != 2 || len(provider.batches[0]) != 15 || len(provider.batches[1]) != 1 {
		t.Fatalf("batches = %#v", provider.batches)
	}
	if workspace.eventReads != 0 || workspace.recentReads != 1 || workspace.stateReads != 2 {
		t.Fatalf("workspace reads = events %d, recent %d, state %d", workspace.eventReads, workspace.recentReads, workspace.stateReads)
	}
}

func TestSynthesisUsesDurableSummaryCompletionWithoutStartupEventRead(t *testing.T) {
	store, target := seededStore(t)
	commitRaw(t, store, "https://example.test/article", "article.md", time.Unix(1, 0).UTC(), []byte("fact\n"))
	commitSummary(t, store, application.SummaryCommit{
		SourceKey: "https://example.test/article", Filename: "article.md", DerivedFrom: "article.md",
		RawWrittenAt: time.Unix(1, 0).UTC(), CreatedAt: time.Unix(2, 0).UTC(), UpdatedAt: time.Unix(2, 0).UTC(), Contents: []byte("summary\n"),
	})
	workspace := &countingWorkspace{Workspace: store, eventReadErr: fmt.Errorf("event read must be on demand")}
	provider := &batchingProvider{}
	result, err := application.SynthesizeWithTools(context.Background(), workspace, provider, application.DefaultSynthesisOptions(), []string{"write_summary"}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 0 || result.SummariesSkipped != 1 {
		t.Fatalf("synthesis = %#v, error = %v", result, err)
	}
	if workspace.eventReads != 0 || len(provider.requests) != 0 {
		t.Fatalf("startup event reads or provider calls = %d, %d", workspace.eventReads, len(provider.requests))
	}
}

func TestSynthesisWithoutReadLogsSkipsRecentReadAndPromptInstruction(t *testing.T) {
	store, target := seededStore(t)
	sources := testSources(t, store, 1)
	workspace := &countingWorkspace{Workspace: store}
	provider := &batchingProvider{sources: sources, includeDoc: true}
	result, err := application.SynthesizeWithTools(context.Background(), workspace, provider, application.DefaultSynthesisOptions(), []string{"read_document", "write_summary"}, operationOptionsFor(target))
	if err != nil || result.SummariesWritten != 1 {
		t.Fatalf("synthesis = %#v, error = %v", result, err)
	}
	if workspace.recentReads != 0 {
		t.Fatalf("recent reads = %d", workspace.recentReads)
	}
	if len(provider.requests) == 0 || strings.Contains(fmt.Sprint(provider.requests[0].Messages[0].Content), "read_logs") {
		t.Fatalf("system prompt = %#v", provider.requests)
	}
}

func TestSynthesisLaterFailurePreservesAndResumesEarlierBatches(t *testing.T) {
	store, target := seededStore(t)
	sources := testSources(t, store, 3)
	provider := &batchingProvider{
		sources: sources, includeDoc: true, provideUsage: true, failBatch: 2,
		failure: internalerrors.ProviderRejected("later batch failed", false),
	}
	config := application.SynthesisOptions{MaxTurns: 2, MaxToolCalls: 2, MaxToolOutputBytes: 4096, MaxResponseTokens: 32, TimeoutSeconds: 5}
	result, err := application.Synthesize(context.Background(), store, provider, config, operationOptionsFor(target))
	if !internalerrors.IsKind(err, internalerrors.KindProviderRejected) || result.SummariesWritten != 1 || result.SummariesSkipped != 0 || result.Metrics.Usage != nil {
		t.Fatalf("failed synthesis = %#v, error = %v", result, err)
	}
	firstSummary, err := store.ReadDocument(context.Background(), domain.SummaryRef("source-00.md"))
	if err != nil {
		t.Fatal(err)
	}
	state, _, err := store.ReadState(context.Background())
	if err != nil || state.Sources[0].Summary == nil {
		t.Fatalf("state after failure = %#v, error = %v", state, err)
	}

	resumeProvider := &batchingProvider{sources: sources, includeDoc: true, provideUsage: true}
	resumed, err := application.Synthesize(context.Background(), store, resumeProvider, config, operationOptionsFor(target))
	if err != nil || resumed.SummariesWritten != 2 || resumed.SummariesSkipped != 1 || len(resumeProvider.batches) != 2 {
		t.Fatalf("resumed synthesis = %#v, batches = %#v, error = %v", resumed, resumeProvider.batches, err)
	}
	for _, batch := range resumeProvider.batches {
		if len(batch) != 1 || batch[0] == "https://example.test/00" {
			t.Fatalf("resume rewrote completed source: %#v", resumeProvider.batches)
		}
	}
	lastSummary, err := store.ReadDocument(context.Background(), domain.SummaryRef("source-00.md"))
	if err != nil || string(lastSummary) != string(firstSummary) {
		t.Fatalf("completed summary changed: before %q, after %q, error %v", firstSummary, lastSummary, err)
	}
	page, err := store.ReadEvents(context.Background(), 0, application.MaxOperationPageLimit)
	if err != nil {
		t.Fatal(err)
	}
	writes := 0
	for _, event := range page.Entries {
		if event.Command == domain.CommandWriteSummary && event.Outcome == domain.OutcomeCommitted {
			writes++
		}
	}
	if writes != 3 {
		t.Fatalf("committed summary events = %d", writes)
	}
}

func TestSynthesisUsesOneDeadlineAcrossBatches(t *testing.T) {
	store, target := seededStore(t)
	sources := testSources(t, store, 2)
	provider := &batchingProvider{sources: sources, includeDoc: true, provideUsage: true, blockBatch: 2}
	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()
	config := application.SynthesisOptions{MaxTurns: 2, MaxToolCalls: 2, MaxToolOutputBytes: 4096, MaxResponseTokens: 32, TimeoutSeconds: 5}

	result, err := application.Synthesize(ctx, store, provider, config, operationOptionsFor(target))
	if !internalerrors.IsKind(err, internalerrors.KindDeadline) || result.SummariesWritten != 1 || result.SummariesSkipped != 0 {
		t.Fatalf("timed synthesis = %#v, error = %v", result, err)
	}
	if len(provider.deadlines) < 3 {
		t.Fatalf("provider deadlines = %#v", provider.deadlines)
	}
	for _, deadline := range provider.deadlines[1:] {
		if !deadline.Equal(provider.deadlines[0]) {
			t.Fatalf("deadlines differ: %#v", provider.deadlines)
		}
	}
}
