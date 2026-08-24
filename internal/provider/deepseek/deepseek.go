package deepseek

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"

	"github.com/skillicinski/bo/internal/agent"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const (
	DefaultEndpoint      = "https://api.deepseek.com/chat/completions"
	DefaultModel         = "deepseek-v4-flash"
	maxResponseBodyBytes = 1 << 20
)

type completionRequest struct {
	Model      string                `json:"model"`
	Messages   []completionMessage   `json:"messages"`
	Tools      []toolDefinition      `json:"tools,omitempty"`
	ToolChoice string                `json:"tool_choice"`
	Stream     bool                  `json:"stream"`
	MaxTokens  int                   `json:"max_tokens"`
	Thinking   thinkingConfiguration `json:"thinking"`
}

type completionMessage struct {
	Role             string     `json:"role"`
	Content          any        `json:"content"`
	Name             string     `json:"name,omitempty"`
	ToolCalls        []toolCall `json:"tool_calls,omitempty"`
	ToolCallID       string     `json:"tool_call_id,omitempty"`
	ReasoningContent any        `json:"reasoning_content,omitempty"`
}

type toolCall struct {
	ID       string       `json:"id"`
	Type     string       `json:"type"`
	Function toolFunction `json:"function"`
}

type toolFunction struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

type toolDefinition struct {
	Type     string          `json:"type"`
	Function toolDeclaration `json:"function"`
}

type toolDeclaration struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Parameters  map[string]any `json:"parameters"`
}

type thinkingConfiguration struct {
	Type string `json:"type"`
}

type completionResponse struct {
	Choices []completionChoice `json:"choices"`
	Usage   *usage             `json:"usage"`
}

type completionChoice struct {
	Message      *completionMessage `json:"message"`
	FinishReason *string            `json:"finish_reason"`
}

type usage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

type Client struct {
	APIKey     string
	Endpoint   string
	Model      string
	HTTPClient *http.Client
}

func New(apiKey, endpoint string) *Client {
	if endpoint == "" {
		endpoint = DefaultEndpoint
	}
	return &Client{APIKey: apiKey, Endpoint: endpoint, HTTPClient: &http.Client{}}
}

func (c *Client) Complete(ctx context.Context, request agent.CompletionRequest) (agent.CompletionResponse, error) {
	model := c.Model
	if model == "" {
		model = DefaultModel
	}
	body, err := json.Marshal(completionRequest{
		Model: model, Messages: wireMessages(request.Messages), Tools: wireTools(request.Tools),
		ToolChoice: "auto", MaxTokens: request.MaxTokens,
		Thinking: thinkingConfiguration{Type: "disabled"},
	})
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderTransport("encoding API request failed", err)
	}
	endpoint := c.Endpoint
	if endpoint == "" {
		endpoint = DefaultEndpoint
	}
	httpClient := c.HTTPClient
	if httpClient == nil {
		httpClient = &http.Client{}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(body))
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderTransport("creating API request failed", err)
	}
	req.Header.Set("Authorization", "Bearer "+c.APIKey)
	req.Header.Set("Content-Type", "application/json")
	response, err := httpClient.Do(req)
	if err != nil {
		if contextErr := internalerrors.Context(err); contextErr != nil {
			return agent.CompletionResponse{}, contextErr
		}
		return agent.CompletionResponse{}, internalerrors.TransientProviderTransport("API request failed", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		data, _ := io.ReadAll(io.LimitReader(response.Body, 512))
		retryable := response.StatusCode == http.StatusTooManyRequests || response.StatusCode >= http.StatusInternalServerError
		return agent.CompletionResponse{}, internalerrors.ProviderRejected(fmt.Sprintf("provider returned HTTP %d: %s", response.StatusCode, string(data)), retryable)
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, maxResponseBodyBytes+1))
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.TransientProviderTransport("reading API response failed", err)
	}
	if len(data) > maxResponseBodyBytes {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("API response exceeds size limit", nil)
	}
	var payload completionResponse
	if err := json.Unmarshal(data, &payload); err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response", err)
	}
	if len(payload.Choices) == 0 {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response: missing choices[0].message", nil)
	}
	choice := payload.Choices[0]
	if choice.Message == nil || choice.Message.Role != "assistant" {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response: invalid choices[0].message", nil)
	}
	if choice.Message.Content != nil {
		if _, ok := choice.Message.Content.(string); !ok {
			return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response: message.content must be a string or null", nil)
		}
	}
	if choice.FinishReason == nil {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response: invalid choices[0].finish_reason", nil)
	}
	if err := validateToolCalls(choice.Message.ToolCalls); err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response: "+err.Error(), nil)
	}
	if payload.Usage != nil && (payload.Usage.PromptTokens < 0 || payload.Usage.CompletionTokens < 0 || payload.Usage.TotalTokens < 0) {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed API response: token usage must not be negative", nil)
	}
	result := agent.CompletionResponse{Message: agentMessage(*choice.Message), Usage: agentUsage(payload.Usage)}
	if err := finishReasonError(*choice.FinishReason, len(choice.Message.ToolCalls) > 0); err != nil {
		return result, err
	}
	return result, nil
}

func wireMessages(messages []agent.ChatMessage) []completionMessage {
	if messages == nil {
		return nil
	}
	wire := make([]completionMessage, len(messages))
	for index, message := range messages {
		wire[index] = completionMessage{
			Role: message.Role, Content: message.Content, Name: message.Name, ToolCallID: message.ToolCallID,
			ToolCalls: wireToolCalls(message.ToolCalls),
		}
	}
	return wire
}

func wireToolCalls(calls []agent.ToolCall) []toolCall {
	if calls == nil {
		return nil
	}
	wire := make([]toolCall, len(calls))
	for index, call := range calls {
		wire[index] = toolCall{ID: call.ID, Type: "function", Function: toolFunction{Name: call.Function.Name, Arguments: call.Function.Arguments}}
	}
	return wire
}

func wireTools(tools []agent.ToolDefinition) []toolDefinition {
	if tools == nil {
		return nil
	}
	wire := make([]toolDefinition, len(tools))
	for index, tool := range tools {
		wire[index] = toolDefinition{Type: "function", Function: toolDeclaration{
			Name: tool.Function.Name, Description: tool.Function.Description, Parameters: tool.Function.Parameters,
		}}
	}
	return wire
}

func agentMessage(message completionMessage) agent.ChatMessage {
	return agent.ChatMessage{
		Role: message.Role, Content: message.Content, Name: message.Name, ToolCallID: message.ToolCallID,
		ToolCalls: agentToolCalls(message.ToolCalls),
	}
}

func agentToolCalls(calls []toolCall) []agent.ToolCall {
	if calls == nil {
		return nil
	}
	converted := make([]agent.ToolCall, len(calls))
	for index, call := range calls {
		converted[index] = agent.ToolCall{
			ID:       call.ID,
			Function: agent.ToolFunction{Name: call.Function.Name, Arguments: call.Function.Arguments},
		}
	}
	return converted
}

func agentUsage(value *usage) *agent.TokenUsage {
	if value == nil {
		return nil
	}
	return &agent.TokenUsage{PromptTokens: value.PromptTokens, CompletionTokens: value.CompletionTokens, TotalTokens: value.TotalTokens}
}

func finishReasonError(reason string, hasToolCalls bool) error {
	switch reason {
	case "stop":
		if hasToolCalls {
			return internalerrors.ProviderMalformed("malformed API response: finish_reason does not match tool_calls", nil)
		}
	case "tool_calls":
		if !hasToolCalls {
			return internalerrors.ProviderMalformed("malformed API response: finish_reason does not match tool_calls", nil)
		}
	case "length", "content_filter":
		if hasToolCalls {
			return internalerrors.ProviderMalformed("malformed API response: finish_reason does not match tool_calls", nil)
		}
		return internalerrors.ProviderRejected("provider completion ended with finish_reason="+reason, false)
	case "insufficient_system_resource":
		if hasToolCalls {
			return internalerrors.ProviderMalformed("malformed API response: finish_reason does not match tool_calls", nil)
		}
		return internalerrors.ProviderRejected("provider completion ended with finish_reason="+reason, true)
	default:
		return internalerrors.ProviderMalformed("malformed API response: invalid choices[0].finish_reason", nil)
	}
	return nil
}

func validateToolCalls(calls []toolCall) error {
	for index, call := range calls {
		if call.ID == "" {
			return fmt.Errorf("tool call %d has no id", index)
		}
		if call.Type != "function" {
			return fmt.Errorf("tool call %d has invalid type", index)
		}
		if call.Function.Name == "" {
			return fmt.Errorf("tool call %d has no function name", index)
		}
		if call.Function.Arguments == "" || !json.Valid([]byte(call.Function.Arguments)) {
			return fmt.Errorf("tool call %d has invalid function arguments", index)
		}
	}
	return nil
}
