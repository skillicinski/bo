package agent

import (
	"context"
	"errors"
	"fmt"
	"time"
	"unicode/utf8"

	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const (
	DefaultMaxTurns           = 32
	DefaultMaxToolCalls       = 64
	DefaultMaxToolOutputBytes = 65_536
	DefaultMaxResponseTokens  = 4_096
)

type Options struct {
	MaxTurns           int
	MaxToolCalls       int
	MaxToolOutputBytes int
	MaxResponseTokens  int
}

func DefaultOptions() Options {
	return Options{
		MaxTurns: DefaultMaxTurns, MaxToolCalls: DefaultMaxToolCalls,
		MaxToolOutputBytes: DefaultMaxToolOutputBytes, MaxResponseTokens: DefaultMaxResponseTokens,
	}
}

type ChatMessage struct {
	Role       string
	Content    any
	Name       string
	ToolCalls  []ToolCall
	ToolCallID string
}

type ToolCall struct {
	ID       string
	Function ToolFunction
}

type ToolFunction struct {
	Name      string
	Arguments string
}

type ToolDefinition struct {
	Function ToolDeclaration
}

type ToolDeclaration struct {
	Name        string
	Description string
	Parameters  map[string]any
}

type CompletionRequest struct {
	Messages  []ChatMessage
	Tools     []ToolDefinition
	MaxTokens int
}

type CompletionResponse struct {
	Message ChatMessage
	Usage   *TokenUsage
}

type TokenUsage struct {
	PromptTokens     int
	CompletionTokens int
	TotalTokens      int
}

type CompletionProvider interface {
	Complete(context.Context, CompletionRequest) (CompletionResponse, error)
}

type Tool struct {
	Definition ToolDefinition
	Execute    func(context.Context, ToolCall) (string, error)
}

type Runtime struct {
	Provider CompletionProvider
	Tools    []Tool
	Done     func() bool
}

type Metrics struct {
	Turns     int           `json:"turns"`
	ToolCalls int           `json:"tool_calls"`
	Usage     *TokenUsage   `json:"usage,omitempty"`
	Duration  time.Duration `json:"duration"`
}

type Result struct {
	Messages []ChatMessage
	Message  ChatMessage
	Metrics
}

func (runtime Runtime) Run(ctx context.Context, messages []ChatMessage, options Options) (Result, error) {
	started := time.Now()
	turns, toolCalls := 0, 0
	var usage TokenUsage
	usageKnown := true
	usageReceived := false
	finish := func(message ChatMessage, err error) (Result, error) {
		metrics := Metrics{Turns: turns, ToolCalls: toolCalls, Duration: time.Since(started)}
		if usageKnown && usageReceived && turns > 0 {
			metrics.Usage = &usage
		}
		return Result{Messages: messages, Message: message, Metrics: metrics}, err
	}
	if runtime.Provider == nil {
		return finish(ChatMessage{}, internalerrors.Request("agent provider is not configured"))
	}
	options = normalizedOptions(options)
	var lastToolError error
	terminalError := func(fallback, batchError error) error {
		if batchError != nil {
			return batchError
		}
		if lastToolError != nil {
			return lastToolError
		}
		return fallback
	}
	definitions := make([]ToolDefinition, 0, len(runtime.Tools))
	executors := make(map[string]func(context.Context, ToolCall) (string, error), len(runtime.Tools))
	for _, tool := range runtime.Tools {
		name := tool.Definition.Function.Name
		definitions = append(definitions, tool.Definition)
		executors[name] = tool.Execute
	}
	for {
		if err := ctx.Err(); err != nil {
			return finish(ChatMessage{}, internalerrors.Context(err))
		}
		if turns >= options.MaxTurns {
			return finish(ChatMessage{}, terminalError(internalerrors.ProviderMalformed(fmt.Sprintf("max turns reached (%d)", options.MaxTurns), nil), nil))
		}
		turns++
		response, err := runtime.Provider.Complete(ctx, CompletionRequest{
			Messages: messages, Tools: definitions, MaxTokens: options.MaxResponseTokens,
		})
		if response.Usage != nil && usageKnown {
			usage.PromptTokens += response.Usage.PromptTokens
			usage.CompletionTokens += response.Usage.CompletionTokens
			usage.TotalTokens += response.Usage.TotalTokens
			usageReceived = true
		} else if err == nil {
			usageKnown = false
		}
		if err != nil {
			return finish(ChatMessage{}, providerError(err))
		}
		message := response.Message
		if message.Role == "" {
			message.Role = "assistant"
		}
		if len(message.ToolCalls) > 0 {
			messages = append(messages, message)
			batchToolError := error(nil)
			for _, call := range message.ToolCalls {
				if toolCalls >= options.MaxToolCalls {
					return finish(message, terminalError(internalerrors.ProviderMalformed(fmt.Sprintf("max tool calls reached (%d)", options.MaxToolCalls), nil), batchToolError))
				}
				toolCalls++
				if call.ID == "" {
					return finish(message, internalerrors.ProviderMalformed("assistant tool call has no id", nil))
				}
				execute, ok := executors[call.Function.Name]
				var output string
				var toolErr error
				if !ok {
					toolErr = fmt.Errorf("unsupported tool: %s", call.Function.Name)
				} else if execute == nil {
					toolErr = fmt.Errorf("tool is not configured: %s", call.Function.Name)
				} else {
					output, toolErr = execute(ctx, call)
				}
				if toolErr != nil {
					var categorized *internalerrors.Error
					if errors.As(toolErr, &categorized) {
						batchToolError = toolErr
					}
					output = "ERROR: " + toolErr.Error()
				}
				messages = append(messages, ChatMessage{Role: "tool", ToolCallID: call.ID, Content: BoundedOutput(output, options.MaxToolOutputBytes)})
			}
			lastToolError = batchToolError
			if runtime.Done != nil && runtime.Done() {
				return finish(message, lastToolError)
			}
			continue
		}
		messages = append(messages, message)
		return finish(message, lastToolError)
	}
}

func providerError(err error) error {
	var categorized *internalerrors.Error
	if errors.As(err, &categorized) {
		return err
	}
	if contextErr := internalerrors.Context(err); contextErr != nil {
		return contextErr
	}
	return internalerrors.ProviderTransport(err.Error(), err)
}

func normalizedOptions(options Options) Options {
	defaults := DefaultOptions()
	if options.MaxTurns <= 0 {
		options.MaxTurns = defaults.MaxTurns
	}
	if options.MaxToolCalls <= 0 {
		options.MaxToolCalls = defaults.MaxToolCalls
	}
	if options.MaxToolOutputBytes <= 0 {
		options.MaxToolOutputBytes = defaults.MaxToolOutputBytes
	}
	if options.MaxResponseTokens <= 0 {
		options.MaxResponseTokens = defaults.MaxResponseTokens
	}
	return options
}

func BoundedOutput(value string, limit int) string {
	if len(value) <= limit {
		return value
	}
	marker := fmt.Sprintf("\n[truncated at %d bytes]\n", len(value))
	if len(marker) >= limit {
		return takePrefix(marker, limit)
	}
	return takePrefix(value, limit-len(marker)) + marker
}

func takePrefix(value string, limit int) string {
	if limit >= len(value) {
		return value
	}
	if limit <= 0 {
		return ""
	}
	for limit > 0 && !utf8.RuneStart(value[limit]) {
		limit--
	}
	return value[:limit]
}
