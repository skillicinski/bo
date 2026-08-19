package agent

import (
	"context"
	"fmt"
	"unicode/utf8"
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
	Role             string     `json:"role"`
	Content          any        `json:"content"`
	Name             string     `json:"name,omitempty"`
	ToolCalls        []ToolCall `json:"tool_calls,omitempty"`
	ToolCallID       string     `json:"tool_call_id,omitempty"`
	ReasoningContent any        `json:"reasoning_content,omitempty"`
}

type ToolCall struct {
	ID       string       `json:"id"`
	Type     string       `json:"type"`
	Function ToolFunction `json:"function"`
}

type ToolFunction struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

type ToolDefinition struct {
	Type     string          `json:"type"`
	Function ToolDeclaration `json:"function"`
}

type ToolDeclaration struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Parameters  map[string]any `json:"parameters"`
}

type CompletionRequest struct {
	Model      string            `json:"model"`
	Messages   []ChatMessage     `json:"messages"`
	Tools      []ToolDefinition  `json:"tools"`
	ToolChoice string            `json:"tool_choice"`
	Stream     bool              `json:"stream"`
	MaxTokens  int               `json:"max_tokens"`
	Thinking   map[string]string `json:"thinking"`
}

type CompletionResponse struct {
	Message      ChatMessage
	FinishReason string
}

type CompletionProvider interface {
	Complete(context.Context, CompletionRequest) (CompletionResponse, error)
}

type Provider = CompletionProvider

type Tool struct {
	Definition ToolDefinition
	Execute    func(context.Context, ToolCall) (string, error)
}

type Runtime struct {
	Provider CompletionProvider
	Tools    []Tool
}

type Result struct {
	Messages  []ChatMessage
	Message   ChatMessage
	Turns     int
	ToolCalls int
}

func (runtime Runtime) Run(ctx context.Context, messages []ChatMessage, options Options) (Result, error) {
	if runtime.Provider == nil {
		return Result{}, fmt.Errorf("agent provider is not configured")
	}
	options = normalizedOptions(options)
	definitions := make([]ToolDefinition, 0, len(runtime.Tools))
	executors := make(map[string]func(context.Context, ToolCall) (string, error), len(runtime.Tools))
	for _, tool := range runtime.Tools {
		name := tool.Definition.Function.Name
		definitions = append(definitions, tool.Definition)
		executors[name] = tool.Execute
	}
	turns, toolCalls := 0, 0
	for {
		if err := ctx.Err(); err != nil {
			return Result{Messages: messages, Turns: turns, ToolCalls: toolCalls}, err
		}
		if turns >= options.MaxTurns {
			return Result{Messages: messages, Turns: turns, ToolCalls: toolCalls}, fmt.Errorf("max turns reached (%d)", options.MaxTurns)
		}
		turns++
		response, err := runtime.Provider.Complete(ctx, CompletionRequest{
			Messages: messages, Tools: definitions, ToolChoice: "auto", Stream: false,
			MaxTokens: options.MaxResponseTokens, Thinking: map[string]string{"type": "disabled"},
		})
		if err != nil {
			return Result{Messages: messages, Turns: turns, ToolCalls: toolCalls}, err
		}
		message := response.Message
		if message.Role == "" {
			message.Role = "assistant"
		}
		if len(message.ToolCalls) > 0 {
			messages = append(messages, message)
			for _, call := range message.ToolCalls {
				if toolCalls >= options.MaxToolCalls {
					return Result{Messages: messages, Turns: turns, ToolCalls: toolCalls}, fmt.Errorf("max tool calls reached (%d)", options.MaxToolCalls)
				}
				toolCalls++
				if call.ID == "" {
					return Result{Messages: messages, Turns: turns, ToolCalls: toolCalls}, fmt.Errorf("assistant tool call has no id")
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
					output = "ERROR: " + toolErr.Error()
				}
				messages = append(messages, ChatMessage{Role: "tool", ToolCallID: call.ID, Content: BoundedOutput(output, options.MaxToolOutputBytes)})
			}
			continue
		}
		messages = append(messages, message)
		return Result{Messages: messages, Message: message, Turns: turns, ToolCalls: toolCalls}, nil
	}
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
