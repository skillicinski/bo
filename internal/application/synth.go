package application

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/skillicinski/bo/internal/agent"
)

func Synthesize(ctx context.Context, opener WorkspaceOpener, workspaceName string, provider agent.CompletionProvider, config SynthesisOptions) (int, error) {
	if opener == nil {
		return 0, RequestError("workspace opener is not configured")
	}
	workspace, err := opener.Open(ctx, workspaceName)
	if err != nil {
		return 0, err
	}
	if workspace == nil {
		return 0, RequestError("workspace opener returned no workspace")
	}
	defer workspace.Close()
	return runSynthesis(ctx, workspace.RootPath(), workspace.TargetPath(), workspace.Storage(), provider, config)
}

func runSynthesis(ctx context.Context, rootPath, targetPath string, storage Storage, provider agent.CompletionProvider, config SynthesisOptions) (int, error) {
	if provider == nil {
		return 0, RequestError("synthesis provider is not configured")
	}
	config = normalizedSynthesisOptions(config)
	runContext, cancel := context.WithTimeout(ctx, time.Duration(config.TimeoutSeconds)*time.Second)
	defer cancel()
	root, err := filepath.EvalSymlinks(rootPath)
	if err != nil {
		return 0, fmt.Errorf("canonicalizing %s failed: %w", rootPath, err)
	}
	target, err := filepath.EvalSymlinks(targetPath)
	if err != nil {
		return 0, fmt.Errorf("canonicalizing %s failed: %w", targetPath, err)
	}
	if err := ensureInside(target, root); err != nil {
		return 0, err
	}
	info, err := os.Stat(target)
	if err != nil {
		return 0, fmt.Errorf("reading %s failed: %w", target, err)
	}
	if !info.IsDir() {
		return 0, fmt.Errorf("target is not a directory: %s", target)
	}
	documents, err := DiscoverDocuments(root, target)
	if err != nil {
		return 0, err
	}
	if len(documents) == 0 {
		return 0, fmt.Errorf("no raw Markdown documents in %s", target)
	}
	state, generation, err := storage.ReadState(runContext)
	if err != nil {
		return 0, err
	}
	sources := sourceGroups(documents, state)
	contextState := &agentContext{
		ctx: runContext, root: root, target: target, storage: storage, documents: documents, sources: sources,
		state: state, generation: generation, cwd: target, maxOutputBytes: config.MaxToolOutputBytes,
	}
	names := make([]string, 0, len(documents))
	for name := range documents {
		names = append(names, name)
	}
	sort.Strings(names)
	messages := []agent.ChatMessage{
		{Role: "system", Content: systemPrompt(contextState, names)},
		{Role: "user", Content: fmt.Sprintf("Call read_state first. Then inspect the latest raw snapshot for every source identity and write one concise Markdown summary per source. Raw documents: %s", strings.Join(names, ", "))},
	}
	summarized := map[string]bool{}
	runtime := agent.Runtime{Provider: provider, Tools: synthTools(contextState, summarized)}
	turns, toolCalls := 0, 0
	correctionSent := false
	for {
		if err := runContext.Err(); err != nil {
			return 0, err
		}
		if turns >= config.MaxTurns {
			return 0, fmt.Errorf("max turns reached (%d) with %d of %d summaries written", config.MaxTurns, len(summarized), len(sources))
		}
		if toolCalls >= config.MaxToolCalls {
			return 0, fmt.Errorf("max tool calls reached (%d) with %d of %d summaries written", config.MaxToolCalls, len(summarized), len(sources))
		}
		result, err := runtime.Run(runContext, messages, agent.Options{
			MaxTurns: config.MaxTurns - turns, MaxToolCalls: config.MaxToolCalls - toolCalls,
			MaxToolOutputBytes: config.MaxToolOutputBytes, MaxResponseTokens: config.MaxResponseTokens,
		})
		turns += result.Turns
		toolCalls += result.ToolCalls
		if err != nil {
			return 0, err
		}
		messages = result.Messages
		missing := make([]string, 0)
		for sourceKey := range sources {
			if !summarized[sourceKey] {
				missing = append(missing, sourceKey)
			}
		}
		sort.Strings(missing)
		if len(missing) == 0 {
			return len(summarized), nil
		}
		if correctionSent {
			return 0, fmt.Errorf("model stopped with missing summaries: %s", strings.Join(missing, ", "))
		}
		correctionSent = true
		messages = append(messages, agent.ChatMessage{Role: "user", Content: fmt.Sprintf("You stopped before completing the task. Use the bounded tools now and write successful summaries for every missing source identity: %s", strings.Join(missing, ", "))})
	}
}

func normalizedSynthesisOptions(config SynthesisOptions) SynthesisOptions {
	defaults := DefaultSynthesisOptions()
	if config.MaxTurns <= 0 {
		config.MaxTurns = defaults.MaxTurns
	}
	if config.MaxToolCalls <= 0 {
		config.MaxToolCalls = defaults.MaxToolCalls
	}
	if config.MaxToolOutputBytes <= 0 {
		config.MaxToolOutputBytes = defaults.MaxToolOutputBytes
	}
	if config.MaxResponseTokens <= 0 {
		config.MaxResponseTokens = defaults.MaxResponseTokens
	}
	if config.TimeoutSeconds <= 0 {
		config.TimeoutSeconds = defaults.TimeoutSeconds
	}
	return config
}
