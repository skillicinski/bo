package agent

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
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
	// ProviderMetadata is immutable provider-owned state for message replay.
	ProviderMetadata string
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
	Message              ChatMessage
	Usage                *TokenUsage
	ProviderRetries      int
	ProviderRetryReasons []string
}

type TokenUsage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
	ThoughtsTokens   int `json:"thoughts_tokens,omitempty"`
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

// ToolCallTelemetry records bounded metadata for one provider-requested tool call.
// ArgumentsPreview is populated only for read and terminal tools.
type ToolCallTelemetry struct {
	Turn                int    `json:"turn"`
	Index               int    `json:"index"`
	ID                  string `json:"id,omitempty"`
	Name                string `json:"name"`
	ArgumentsBytes      int    `json:"arguments_bytes"`
	ArgumentsSHA256     string `json:"arguments_sha256"`
	ArgumentsPreview    string `json:"arguments_preview,omitempty"`
	OutputBytes         int    `json:"output_bytes"`
	OutputReturnedBytes int    `json:"output_returned_bytes"`
	OutputSHA256        string `json:"output_sha256"`
	OutputTruncated     bool   `json:"output_truncated,omitempty"`
	Error               string `json:"error,omitempty"`
}

type Telemetry struct {
	ToolCalls            []ToolCallTelemetry `json:"tool_calls,omitempty"`
	ProviderRetries      int                 `json:"provider_retries,omitempty"`
	ProviderRetryReasons []string            `json:"provider_retry_reasons,omitempty"`
	TerminalReason       string              `json:"terminal_reason,omitempty"`
}

type Result struct {
	Messages []ChatMessage
	Message  ChatMessage
	Metrics
	Telemetry Telemetry
}

func (runtime Runtime) Run(ctx context.Context, messages []ChatMessage, options Options) (Result, error) {
	started := time.Now()
	turns, toolCalls := 0, 0
	telemetry := Telemetry{}
	var usage TokenUsage
	usageKnown := true
	usageReceived := false
	finish := func(reason string, message ChatMessage, err error) (Result, error) {
		metrics := Metrics{Turns: turns, ToolCalls: toolCalls, Duration: time.Since(started)}
		if usageKnown && usageReceived && turns > 0 {
			metrics.Usage = &usage
		}
		telemetry.TerminalReason = reason
		return Result{Messages: messages, Message: message, Metrics: metrics, Telemetry: telemetry}, err
	}
	if runtime.Provider == nil {
		return finish("provider_not_configured", ChatMessage{}, internalerrors.Request("agent provider is not configured"))
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
			return finish("context_error", ChatMessage{}, internalerrors.Context(err))
		}
		if turns >= options.MaxTurns {
			return finish("max_turns", ChatMessage{}, terminalError(internalerrors.ProviderMalformed(fmt.Sprintf("max turns reached (%d)", options.MaxTurns), nil), nil))
		}
		turns++
		response, err := runtime.Provider.Complete(ctx, CompletionRequest{
			Messages: messages, Tools: definitions, MaxTokens: options.MaxResponseTokens,
		})
		if response.Usage != nil && usageKnown {
			usage.PromptTokens += response.Usage.PromptTokens
			usage.CompletionTokens += response.Usage.CompletionTokens
			usage.TotalTokens += response.Usage.TotalTokens
			usage.ThoughtsTokens += response.Usage.ThoughtsTokens
			usageReceived = true
		} else if err == nil {
			usageKnown = false
		}
		if response.ProviderRetries > 0 {
			telemetry.ProviderRetries += response.ProviderRetries
		}
		if len(response.ProviderRetryReasons) > 0 {
			telemetry.ProviderRetryReasons = append(telemetry.ProviderRetryReasons, response.ProviderRetryReasons...)
		}
		if err != nil {
			return finish("provider_error", ChatMessage{}, providerError(err))
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
					return finish("max_tool_calls", message, terminalError(internalerrors.ProviderMalformed(fmt.Sprintf("max tool calls reached (%d)", options.MaxToolCalls), nil), batchToolError))
				}
				toolCalls++
				telemetry.ToolCalls = append(telemetry.ToolCalls, ToolCallTelemetry{
					Turn: turns, Index: toolCalls, ID: call.ID, Name: call.Function.Name,
					ArgumentsBytes: len(call.Function.Arguments), ArgumentsSHA256: sha256Hex(call.Function.Arguments),
					ArgumentsPreview: toolArgumentsPreview(call.Function.Name, call.Function.Arguments),
				})
				trace := &telemetry.ToolCalls[len(telemetry.ToolCalls)-1]
				if call.ID == "" {
					trace.Error = "assistant tool call has no id"
					return finish("invalid_tool_call", message, internalerrors.ProviderMalformed("assistant tool call has no id", nil))
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
					trace.Error = toolErr.Error()
					var categorized *internalerrors.Error
					if errors.As(toolErr, &categorized) {
						batchToolError = toolErr
					}
					output = "ERROR: " + toolErr.Error()
				}
				boundedOutput := BoundedOutput(output, options.MaxToolOutputBytes)
				trace.OutputBytes = len(output)
				trace.OutputReturnedBytes = len(boundedOutput)
				trace.OutputSHA256 = sha256Hex(output)
				trace.OutputTruncated = len(output) != len(boundedOutput)
				messages = append(messages, ChatMessage{Role: "tool", ToolCallID: call.ID, Content: boundedOutput})
			}
			lastToolError = batchToolError
			if runtime.Done != nil && runtime.Done() {
				reason := "done"
				if lastToolError != nil {
					reason = "done_with_tool_error"
				}
				return finish(reason, message, lastToolError)
			}
			continue
		}
		messages = append(messages, message)
		reason := "assistant_message"
		if lastToolError != nil {
			reason = "assistant_message_with_tool_error"
		}
		return finish(reason, message, lastToolError)
	}
}

func sha256Hex(value string) string {
	digest := sha256.Sum256([]byte(value))
	return hex.EncodeToString(digest[:])
}

func toolArgumentsPreview(name, arguments string) string {
	if strings.HasPrefix(name, "read_") || name == "skip_distill" {
		return BoundedOutput(arguments, 2048)
	}
	return ""
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
