package bo

import "context"

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
