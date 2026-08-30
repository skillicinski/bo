package gemini

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"

	"golang.org/x/oauth2"
	"golang.org/x/oauth2/google"

	"github.com/skillicinski/bo/internal/agent"
	internalerrors "github.com/skillicinski/bo/internal/errors"
)

const (
	DefaultEndpoint       = "https://generativelanguage.googleapis.com/v1beta"
	DefaultVertexEndpoint = "https://aiplatform.googleapis.com"
	DefaultModel          = "gemini-2.5-flash"
	cloudPlatformScope    = "https://www.googleapis.com/auth/cloud-platform"
	maxResponseBodyBytes  = 1 << 20
)

type Config struct {
	APIKey     string
	ProjectID  string
	Location   string
	Endpoint   string
	Model      string
	HTTPClient *http.Client
}

type Client struct {
	APIKey      string
	ProjectID   string
	Location    string
	Endpoint    string
	Model       string
	HTTPClient  *http.Client
	vertex      bool
	tokenSource oauth2.TokenSource
}

func New(config Config) *Client {
	return &Client{
		APIKey: config.APIKey, Endpoint: config.Endpoint, Model: config.Model,
		HTTPClient: config.HTTPClient,
	}
}

func NewVertex(ctx context.Context, config Config) (*Client, error) {
	if config.ProjectID == "" || config.Location == "" {
		return nil, internalerrors.Request("Vertex AI requires project ID and location")
	}
	credentials, err := google.FindDefaultCredentials(ctx, cloudPlatformScope)
	if err != nil {
		return nil, internalerrors.ProviderTransport("finding Application Default Credentials failed", err)
	}
	if credentials == nil || credentials.TokenSource == nil {
		return nil, internalerrors.ProviderTransport("Application Default Credentials returned no token source", nil)
	}
	return &Client{
		ProjectID: config.ProjectID, Location: config.Location, Endpoint: config.Endpoint,
		Model: config.Model, HTTPClient: config.HTTPClient, vertex: true,
		tokenSource: credentials.TokenSource,
	}, nil
}

func (c *Client) Complete(ctx context.Context, request agent.CompletionRequest) (agent.CompletionResponse, error) {
	if c == nil {
		return agent.CompletionResponse{}, internalerrors.Request("Gemini provider is not configured")
	}
	if c.vertex {
		if c.tokenSource == nil {
			return agent.CompletionResponse{}, internalerrors.Request("Vertex AI credentials are not configured")
		}
	} else if c.APIKey == "" {
		return agent.CompletionResponse{}, internalerrors.Request("Gemini API key is not configured")
	}
	model := c.Model
	if model == "" {
		model = DefaultModel
	}
	endpoint := c.endpoint(model)
	payload, err := wireRequest(request)
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("invalid Gemini request", err)
	}
	body, err := json.Marshal(payload)
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderTransport("encoding Gemini request failed", err)
	}
	httpClient := c.HTTPClient
	if httpClient == nil {
		httpClient = &http.Client{}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(body))
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderTransport("creating Gemini request failed", err)
	}
	req.Header.Set("Content-Type", "application/json")
	if c.vertex {
		token, err := c.tokenSource.Token()
		if err != nil {
			if contextErr := internalerrors.Context(err); contextErr != nil {
				return agent.CompletionResponse{}, contextErr
			}
			return agent.CompletionResponse{}, internalerrors.ProviderTransport("getting Vertex AI access token failed", err)
		}
		if token == nil || token.AccessToken == "" {
			return agent.CompletionResponse{}, internalerrors.ProviderTransport("Vertex AI token source returned no access token", nil)
		}
		req.Header.Set("Authorization", "Bearer "+token.AccessToken)
	} else {
		req.Header.Set("x-goog-api-key", c.APIKey)
	}
	response, err := httpClient.Do(req)
	if err != nil {
		if contextErr := internalerrors.Context(err); contextErr != nil {
			return agent.CompletionResponse{}, contextErr
		}
		return agent.CompletionResponse{}, internalerrors.TransientProviderTransport("Gemini request failed", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		data, _ := io.ReadAll(io.LimitReader(response.Body, 512))
		retryable := response.StatusCode == http.StatusTooManyRequests || response.StatusCode >= http.StatusInternalServerError
		return agent.CompletionResponse{}, internalerrors.ProviderRejected(fmt.Sprintf("provider returned HTTP %d: %s", response.StatusCode, string(data)), retryable)
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, maxResponseBodyBytes+1))
	if err != nil {
		if contextErr := internalerrors.Context(err); contextErr != nil {
			return agent.CompletionResponse{}, contextErr
		}
		return agent.CompletionResponse{}, internalerrors.TransientProviderTransport("reading Gemini response failed", err)
	}
	if len(data) > maxResponseBodyBytes {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("Gemini response exceeds size limit", nil)
	}
	var responsePayload completionResponse
	if err := json.Unmarshal(data, &responsePayload); err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed Gemini response", err)
	}
	if len(responsePayload.Candidates) == 0 {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed Gemini response: missing candidates[0]", nil)
	}
	candidate := responsePayload.Candidates[0]
	if candidate.Content == nil {
		if err := finishReasonError(candidate.FinishReason); err != nil {
			return agent.CompletionResponse{}, err
		}
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed Gemini response: missing candidates[0].content", nil)
	}
	if candidate.Content.Role != "" && candidate.Content.Role != "model" {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed Gemini response: invalid candidates[0].content.role", nil)
	}
	if responsePayload.Usage != nil && (responsePayload.Usage.PromptTokens < 0 || responsePayload.Usage.CompletionTokens < 0 || responsePayload.Usage.TotalTokens < 0) {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed Gemini response: token usage must not be negative", nil)
	}
	message, err := agentMessage(*candidate.Content)
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.ProviderMalformed("malformed Gemini response: "+err.Error(), nil)
	}
	result := agent.CompletionResponse{Message: message, Usage: agentUsage(responsePayload.Usage)}
	if err := finishReasonError(candidate.FinishReason); err != nil {
		return result, err
	}
	return result, nil
}

func (c *Client) endpoint(model string) string {
	base := strings.TrimRight(c.Endpoint, "/")
	if c.vertex {
		if base == "" {
			base = vertexEndpoint(c.Location)
		}
		return fmt.Sprintf("%s/v1/projects/%s/locations/%s/publishers/google/models/%s:generateContent", base, url.PathEscape(c.ProjectID), url.PathEscape(c.Location), url.PathEscape(model))
	}
	if base == "" {
		base = DefaultEndpoint
	}
	return fmt.Sprintf("%s/models/%s:generateContent", base, url.PathEscape(model))
}

func vertexEndpoint(location string) string {
	if location == "global" {
		return DefaultVertexEndpoint
	}
	return "https://" + url.PathEscape(location) + "-aiplatform.googleapis.com"
}

type completionRequest struct {
	Contents          []requestContent `json:"contents"`
	SystemInstruction *requestContent  `json:"systemInstruction,omitempty"`
	Tools             []requestTool    `json:"tools,omitempty"`
	ToolConfig        *toolConfig      `json:"toolConfig,omitempty"`
	GenerationConfig  generationConfig `json:"generationConfig,omitempty"`
}

type requestContent struct {
	Role  string        `json:"role,omitempty"`
	Parts []requestPart `json:"parts"`
}

type requestPart struct {
	Text             *string                  `json:"text,omitempty"`
	FunctionCall     *requestFunctionCall     `json:"functionCall,omitempty"`
	FunctionResponse *requestFunctionResponse `json:"functionResponse,omitempty"`
	ThoughtSignature string                   `json:"thoughtSignature,omitempty"`
}

type requestFunctionCall struct {
	ID   string         `json:"id,omitempty"`
	Name string         `json:"name"`
	Args map[string]any `json:"args"`
}

type requestFunctionResponse struct {
	ID       string         `json:"id,omitempty"`
	Name     string         `json:"name"`
	Response map[string]any `json:"response"`
}

type requestTool struct {
	FunctionDeclarations []functionDeclaration `json:"functionDeclarations"`
}

type toolConfig struct {
	FunctionCallingConfig functionCallingConfig `json:"functionCallingConfig"`
}

type functionCallingConfig struct {
	Mode string `json:"mode"`
}

type functionDeclaration struct {
	Name                 string         `json:"name"`
	Description          string         `json:"description,omitempty"`
	ParametersJSONSchema map[string]any `json:"parametersJsonSchema,omitempty"`
}

type generationConfig struct {
	MaxOutputTokens int `json:"maxOutputTokens,omitempty"`
}

type completionResponse struct {
	Candidates []candidate `json:"candidates"`
	Usage      *usage      `json:"usageMetadata"`
}

type candidate struct {
	Content      *responseContent `json:"content"`
	FinishReason string           `json:"finishReason"`
}

type responseContent struct {
	Role  string         `json:"role"`
	Parts []responsePart `json:"parts"`
}

type responsePart struct {
	Text             *string               `json:"text"`
	FunctionCall     *responseFunctionCall `json:"functionCall"`
	ThoughtSignature string                `json:"thoughtSignature"`
}

type responseFunctionCall struct {
	ID   string         `json:"id"`
	Name string         `json:"name"`
	Args map[string]any `json:"args"`
}

type usage struct {
	PromptTokens     int `json:"promptTokenCount"`
	CompletionTokens int `json:"candidatesTokenCount"`
	TotalTokens      int `json:"totalTokenCount"`
}

func wireRequest(request agent.CompletionRequest) (completionRequest, error) {
	payload := completionRequest{
		Contents:         make([]requestContent, 0, len(request.Messages)),
		GenerationConfig: generationConfig{MaxOutputTokens: request.MaxTokens},
	}
	lastWasTool := false
	nativeToolIDs := make(map[string]string)
	for index, message := range request.Messages {
		switch message.Role {
		case "system":
			text, err := messageText(message.Content)
			if err != nil {
				return completionRequest{}, fmt.Errorf("system message %d: %w", index, err)
			}
			if payload.SystemInstruction == nil {
				payload.SystemInstruction = &requestContent{}
			}
			payload.SystemInstruction.Parts = append(payload.SystemInstruction.Parts, textPart(text))
			lastWasTool = false
		case "user":
			text, err := messageText(message.Content)
			if err != nil {
				return completionRequest{}, fmt.Errorf("user message %d: %w", index, err)
			}
			payload.Contents = append(payload.Contents, requestContent{Role: "user", Parts: []requestPart{textPart(text)}})
			lastWasTool = false
		case "assistant":
			content, err := assistantContent(message)
			if err != nil {
				return completionRequest{}, fmt.Errorf("assistant message %d: %w", index, err)
			}
			payload.Contents = append(payload.Contents, content)
			toolIndex := 0
			for _, part := range content.Parts {
				if part.FunctionCall == nil || toolIndex >= len(message.ToolCalls) {
					continue
				}
				nativeToolIDs[message.ToolCalls[toolIndex].ID] = part.FunctionCall.ID
				toolIndex++
			}
			lastWasTool = false
		case "tool":
			name := previousToolName(request.Messages[:index], message.ToolCallID)
			if name == "" {
				return completionRequest{}, fmt.Errorf("tool message %d has no matching assistant call", index)
			}
			nativeID := message.ToolCallID
			if id, ok := nativeToolIDs[message.ToolCallID]; ok {
				nativeID = id
			}
			toolPart := requestPart{FunctionResponse: &requestFunctionResponse{
				ID: nativeID, Name: name, Response: map[string]any{"output": message.Content},
			}}
			if !lastWasTool {
				payload.Contents = append(payload.Contents, requestContent{Role: "user", Parts: []requestPart{toolPart}})
			} else {
				last := &payload.Contents[len(payload.Contents)-1]
				last.Parts = append(last.Parts, toolPart)
			}
			lastWasTool = true
		default:
			return completionRequest{}, fmt.Errorf("unsupported message role %q", message.Role)
		}
	}
	if len(request.Tools) > 0 {
		tool := requestTool{FunctionDeclarations: make([]functionDeclaration, len(request.Tools))}
		for index, definition := range request.Tools {
			tool.FunctionDeclarations[index] = functionDeclaration{
				Name: definition.Function.Name, Description: definition.Function.Description,
				ParametersJSONSchema: definition.Function.Parameters,
			}
		}
		payload.Tools = []requestTool{tool}
		payload.ToolConfig = &toolConfig{FunctionCallingConfig: functionCallingConfig{Mode: "ANY"}}
	}
	return payload, nil
}

func messageText(value any) (string, error) {
	if value == nil {
		return "", nil
	}
	text, ok := value.(string)
	if !ok {
		return "", fmt.Errorf("content must be a string or null")
	}
	return text, nil
}

func textPart(value string) requestPart { return requestPart{Text: &value} }

func assistantContent(message agent.ChatMessage) (requestContent, error) {
	if message.ProviderMetadata != "" {
		var parts []requestPart
		if err := json.Unmarshal([]byte(message.ProviderMetadata), &parts); err != nil || len(parts) == 0 {
			return requestContent{}, fmt.Errorf("invalid provider metadata")
		}
		for index, part := range parts {
			if part.Text == nil && part.FunctionCall == nil && part.ThoughtSignature == "" {
				return requestContent{}, fmt.Errorf("provider metadata part %d has no supported content", index)
			}
			if part.FunctionCall != nil && part.FunctionCall.Name == "" {
				return requestContent{}, fmt.Errorf("provider metadata function call %d has no function name", index)
			}
		}
		return requestContent{Role: "model", Parts: parts}, nil
	}
	content := requestContent{Role: "model"}
	if message.Content != nil {
		text, err := messageText(message.Content)
		if err != nil {
			return requestContent{}, err
		}
		content.Parts = append(content.Parts, textPart(text))
	}
	for index, call := range message.ToolCalls {
		if call.Function.Name == "" {
			return requestContent{}, fmt.Errorf("tool call %d has no function name", index)
		}
		var args map[string]any
		if call.Function.Arguments == "" {
			args = map[string]any{}
		} else if err := json.Unmarshal([]byte(call.Function.Arguments), &args); err != nil || args == nil {
			return requestContent{}, fmt.Errorf("tool call %d has invalid arguments", index)
		}
		content.Parts = append(content.Parts, requestPart{FunctionCall: &requestFunctionCall{
			ID: call.ID, Name: call.Function.Name, Args: args,
		}})
	}
	if len(content.Parts) == 0 {
		return requestContent{}, fmt.Errorf("has no content or tool calls")
	}
	return content, nil
}

func previousToolName(messages []agent.ChatMessage, id string) string {
	for index := len(messages) - 1; index >= 0; index-- {
		if messages[index].Role != "assistant" {
			continue
		}
		for _, call := range messages[index].ToolCalls {
			if call.ID == id {
				return call.Function.Name
			}
		}
	}
	return ""
}

func agentMessage(content responseContent) (agent.ChatMessage, error) {
	message := agent.ChatMessage{Role: "assistant"}
	parts := make([]requestPart, 0, len(content.Parts))
	var text strings.Builder
	for index, part := range content.Parts {
		if part.Text == nil && part.FunctionCall == nil && part.ThoughtSignature == "" {
			return agent.ChatMessage{}, fmt.Errorf("part %d has no supported content", index)
		}
		replay := requestPart{ThoughtSignature: part.ThoughtSignature}
		if part.Text != nil {
			text.WriteString(*part.Text)
			replay.Text = part.Text
		}
		if part.FunctionCall == nil {
			parts = append(parts, replay)
			continue
		}
		if part.FunctionCall.Name == "" {
			return agent.ChatMessage{}, fmt.Errorf("function call %d has no name", index)
		}
		functionArgs := part.FunctionCall.Args
		if functionArgs == nil {
			functionArgs = map[string]any{}
		}
		args, err := json.Marshal(functionArgs)
		if err != nil {
			return agent.ChatMessage{}, fmt.Errorf("function call %d has invalid arguments", index)
		}
		id := part.FunctionCall.ID
		if id == "" {
			id = fmt.Sprintf("gemini-call-%d", index)
		}
		replay.FunctionCall = &requestFunctionCall{ID: part.FunctionCall.ID, Name: part.FunctionCall.Name, Args: functionArgs}
		parts = append(parts, replay)
		message.ToolCalls = append(message.ToolCalls, agent.ToolCall{
			ID: id, Function: agent.ToolFunction{Name: part.FunctionCall.Name, Arguments: string(args)},
		})
	}
	if len(content.Parts) == 0 {
		return agent.ChatMessage{}, fmt.Errorf("content has no parts")
	}
	if text.Len() > 0 || hasTextPart(content.Parts) {
		message.Content = text.String()
	}
	metadata, err := json.Marshal(parts)
	if err != nil {
		return agent.ChatMessage{}, fmt.Errorf("invalid response parts")
	}
	message.ProviderMetadata = string(metadata)
	return message, nil
}

func hasTextPart(parts []responsePart) bool {
	for _, part := range parts {
		if part.Text != nil {
			return true
		}
	}
	return false
}

func agentUsage(value *usage) *agent.TokenUsage {
	if value == nil {
		return nil
	}
	if value.PromptTokens < 0 || value.CompletionTokens < 0 || value.TotalTokens < 0 {
		return nil
	}
	return &agent.TokenUsage{PromptTokens: value.PromptTokens, CompletionTokens: value.CompletionTokens, TotalTokens: value.TotalTokens}
}

func finishReasonError(reason string) error {
	switch strings.ToUpper(reason) {
	case "", "STOP":
		return nil
	case "MAX_TOKENS", "SAFETY", "RECITATION", "OTHER", "BLOCKLIST", "PROHIBITED_CONTENT", "SPII", "MALFORMED_FUNCTION_CALL", "IMAGE_SAFETY", "UNEXPECTED_TOOL_CALL", "NO_IMAGE":
		return internalerrors.ProviderRejected("provider completion ended with finish_reason="+strings.ToLower(reason), false)
	default:
		return internalerrors.ProviderMalformed("malformed Gemini response: invalid candidates[0].finishReason", nil)
	}
}
