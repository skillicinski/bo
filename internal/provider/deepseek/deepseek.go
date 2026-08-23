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
	DefaultEndpoint = "https://api.deepseek.com/chat/completions"
	DefaultModel    = "deepseek-v4-flash"
)

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

func NewClient(apiKey, endpoint string) *Client { return New(apiKey, endpoint) }

func (c *Client) Complete(ctx context.Context, request agent.CompletionRequest) (agent.CompletionResponse, error) {
	if request.Model == "" {
		request.Model = c.Model
		if request.Model == "" {
			request.Model = DefaultModel
		}
	}
	body, err := json.Marshal(request)
	if err != nil {
		return agent.CompletionResponse{}, internalerrors.Request(fmt.Sprintf("encoding API request failed: %v", err))
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
		return agent.CompletionResponse{}, internalerrors.Request(fmt.Sprintf("creating API request failed: %v", err))
	}
	req.Header.Set("Authorization", "Bearer "+c.APIKey)
	req.Header.Set("Content-Type", "application/json")
	response, err := httpClient.Do(req)
	if err != nil {
		return agent.CompletionResponse{}, fmt.Errorf("API request failed: %v", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		data, _ := io.ReadAll(io.LimitReader(response.Body, 512))
		return agent.CompletionResponse{}, fmt.Errorf("DeepSeek HTTP %d: %s", response.StatusCode, string(data))
	}
	var payload struct {
		Choices []struct {
			Message      agent.ChatMessage `json:"message"`
			FinishReason string            `json:"finish_reason"`
		} `json:"choices"`
		Usage *agent.TokenUsage `json:"usage"`
	}
	if err := json.NewDecoder(response.Body).Decode(&payload); err != nil {
		return agent.CompletionResponse{}, fmt.Errorf("malformed API response: %v", err)
	}
	if len(payload.Choices) == 0 {
		return agent.CompletionResponse{}, fmt.Errorf("malformed API response: missing choices[0].message")
	}
	message := payload.Choices[0].Message
	if message.Role == "" {
		message.Role = "assistant"
	}
	return agent.CompletionResponse{Message: message, FinishReason: payload.Choices[0].FinishReason, Usage: payload.Usage}, nil
}
