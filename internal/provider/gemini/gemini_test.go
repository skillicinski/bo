package gemini

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"

	"golang.org/x/oauth2"

	"github.com/skillicinski/bo/internal/agent"
)

func TestCompleteUsesGeminiHTTPContract(t *testing.T) {
	thinkingBudget := 0
	parameters := map[string]any{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]any{
			"groups": map[string]any{
				"type":     "array",
				"minItems": 1,
				"items": map[string]any{
					"type":     "array",
					"minItems": 2,
					"items":    map[string]any{"type": "string"},
				},
			},
		},
	}
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Method != http.MethodPost || request.URL.Path != "/models/test-model:generateContent" || request.URL.RawQuery != "" {
			t.Fatalf("request = %s %s?%s", request.Method, request.URL.Path, request.URL.RawQuery)
		}
		if got := request.Header.Get("x-goog-api-key"); got != "gemini-key" {
			t.Fatalf("API key = %q", got)
		}
		if got := request.Header.Get("Authorization"); got != "" {
			t.Fatalf("unexpected authorization header = %q", got)
		}
		var payload completionRequest
		if err := json.NewDecoder(request.Body).Decode(&payload); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		if payload.SystemInstruction == nil || payload.SystemInstruction.Parts[0].Text == nil || *payload.SystemInstruction.Parts[0].Text != "rules" {
			t.Fatalf("system instruction = %#v", payload.SystemInstruction)
		}
		if len(payload.Contents) != 3 || payload.Contents[0].Role != "user" || payload.Contents[1].Role != "model" || payload.Contents[2].Role != "user" {
			t.Fatalf("contents = %#v", payload.Contents)
		}
		call := payload.Contents[1].Parts[0].FunctionCall
		if call == nil || call.ID != "call-1" || call.Name != "read" || call.Args["filename"] != "note.md" {
			t.Fatalf("function call = %#v", call)
		}
		response := payload.Contents[2].Parts[0].FunctionResponse
		if response == nil || response.ID != "call-1" || response.Name != "read" || response.Response["output"] != "document" {
			t.Fatalf("function response = %#v", response)
		}
		if payload.GenerationConfig.MaxOutputTokens != 77 || payload.GenerationConfig.ThinkingConfig == nil || payload.GenerationConfig.ThinkingConfig.ThinkingBudget == nil || *payload.GenerationConfig.ThinkingConfig.ThinkingBudget != 0 || len(payload.Tools) != 1 || len(payload.Tools[0].FunctionDeclarations) != 1 {
			t.Fatalf("request config = %#v", payload)
		}
		if payload.ToolConfig == nil || payload.ToolConfig.FunctionCallingConfig.Mode != "ANY" {
			t.Fatalf("tool config = %#v", payload.ToolConfig)
		}
		declaration := payload.Tools[0].FunctionDeclarations[0]
		if declaration.ParametersJSONSchema["additionalProperties"] != false {
			t.Fatalf("parametersJsonSchema = %#v", declaration.ParametersJSONSchema)
		}
		properties, ok := declaration.ParametersJSONSchema["properties"].(map[string]any)
		if !ok {
			t.Fatalf("properties = %#v", declaration.ParametersJSONSchema["properties"])
		}
		groups, ok := properties["groups"].(map[string]any)
		if !ok || groups["type"] != "array" || groups["minItems"] != float64(1) {
			t.Fatalf("groups schema = %#v", properties["groups"])
		}
		items, ok := groups["items"].(map[string]any)
		if !ok || items["type"] != "array" || items["minItems"] != float64(2) {
			t.Fatalf("nested array schema = %#v", groups["items"])
		}
		writer.Header().Set("Content-Type", "application/json")
		io.WriteString(writer, `{"candidates":[{"content":{"role":"model","parts":[{"text":"done"},{"functionCall":{"id":"call-2","name":"write","args":{"filename":"note.md"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5,"thoughtsTokenCount":7}}`)
	}))
	defer server.Close()

	result, err := New(Config{APIKey: "gemini-key", Endpoint: server.URL, Model: "test-model", ThinkingBudget: &thinkingBudget}).Complete(context.Background(), agent.CompletionRequest{
		Messages: []agent.ChatMessage{
			{Role: "system", Content: "rules"},
			{Role: "user", Content: "read note"},
			{Role: "assistant", ToolCalls: []agent.ToolCall{{ID: "call-1", Function: agent.ToolFunction{Name: "read", Arguments: `{"filename":"note.md"}`}}}},
			{Role: "tool", ToolCallID: "call-1", Content: "document"},
		},
		Tools: []agent.ToolDefinition{{Function: agent.ToolDeclaration{Name: "read", Parameters: parameters}}}, MaxTokens: 77,
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Message.Content != "done" || len(result.Message.ToolCalls) != 1 || result.Message.ToolCalls[0].ID != "call-2" || result.Message.ToolCalls[0].Function.Arguments != `{"filename":"note.md"}` {
		t.Fatalf("result message = %#v", result.Message)
	}
	if result.Usage == nil || result.Usage.PromptTokens != 3 || result.Usage.CompletionTokens != 2 || result.Usage.TotalTokens != 5 || result.Usage.ThoughtsTokens != 7 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}

func TestCompletePreservesUsageWhenProviderReturnsNoContent(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		io.WriteString(writer, `{"candidates":[{"finishReason":"MAX_TOKENS"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"totalTokenCount":7,"thoughtsTokenCount":3}}`)
	}))
	defer server.Close()

	result, err := New(Config{APIKey: "gemini-key", Endpoint: server.URL, Model: "test-model"}).Complete(context.Background(), agent.CompletionRequest{
		Messages: []agent.ChatMessage{{Role: "user", Content: "continue"}}, MaxTokens: 10,
	})
	if err == nil {
		t.Fatal("completion succeeded")
	}
	if result.Usage == nil || result.Usage.PromptTokens != 5 || result.Usage.CompletionTokens != 2 || result.Usage.TotalTokens != 7 || result.Usage.ThoughtsTokens != 3 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}

func TestCompleteRetriesMalformedFunctionCallOnce(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		writer.Header().Set("Content-Type", "application/json")
		if requests.Add(1) == 1 {
			io.WriteString(writer, `{"candidates":[{"finishReason":"MALFORMED_FUNCTION_CALL","finishMessage":"invalid function arguments"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"totalTokenCount":7,"thoughtsTokenCount":3}}`)
			return
		}
		io.WriteString(writer, `{"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":4,"totalTokenCount":15,"thoughtsTokenCount":1}}`)
	}))
	defer server.Close()

	result, err := New(Config{APIKey: "gemini-key", Endpoint: server.URL, Model: "test-model"}).Complete(context.Background(), agent.CompletionRequest{
		Messages: []agent.ChatMessage{{Role: "user", Content: "continue"}}, MaxTokens: 10,
	})
	if err != nil || result.Message.Content != "ok" {
		t.Fatalf("result = %#v, error = %v", result, err)
	}
	if requests.Load() != 2 || result.ProviderRetries != 1 || len(result.ProviderRetryReasons) != 1 || result.ProviderRetryReasons[0] != "malformed_function_call" {
		t.Fatalf("retry metadata = %#v, requests = %d", result, requests.Load())
	}
	if result.Usage == nil || result.Usage.PromptTokens != 16 || result.Usage.CompletionTokens != 6 || result.Usage.TotalTokens != 22 || result.Usage.ThoughtsTokens != 4 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}

func TestCompleteStopsAfterOneMalformedFunctionCallRetry(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
		requests.Add(1)
		io.WriteString(writer, `{"candidates":[{"finishReason":"MALFORMED_FUNCTION_CALL","finishMessage":"invalid function arguments"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"totalTokenCount":7}}`)
	}))
	defer server.Close()

	result, err := New(Config{APIKey: "gemini-key", Endpoint: server.URL, Model: "test-model"}).Complete(context.Background(), agent.CompletionRequest{
		Messages: []agent.ChatMessage{{Role: "user", Content: "continue"}}, MaxTokens: 10,
	})
	if err == nil || requests.Load() != 2 || result.ProviderRetries != 1 {
		t.Fatalf("result = %#v, error = %v, requests = %d", result, err, requests.Load())
	}
	if result.Usage == nil || result.Usage.TotalTokens != 14 {
		t.Fatalf("usage = %#v", result.Usage)
	}
}

func TestCompleteReplaysThoughtSignatureAcrossToolTurn(t *testing.T) {
	var requestCount atomic.Int32
	var executed atomic.Int32
	executeTool := func(context.Context, agent.ToolCall) (string, error) {
		executed.Add(1)
		return "document", nil
	}
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		var payload completionRequest
		if err := json.NewDecoder(request.Body).Decode(&payload); err != nil {
			t.Errorf("decode request: %v", err)
			return
		}
		writer.Header().Set("Content-Type", "application/json")
		switch requestCount.Add(1) {
		case 1:
			if len(payload.Contents) != 1 || payload.Contents[0].Role != "user" {
				t.Errorf("first request contents = %#v", payload.Contents)
			}
			io.WriteString(writer, `{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"read","args":{"filename":"note.md"}},"thoughtSignature":"signature-1"}]},"finishReason":"STOP"}]}`)
		case 2:
			if len(payload.Contents) != 3 {
				t.Errorf("second request contents = %#v", payload.Contents)
				return
			}
			model := payload.Contents[1]
			if model.Role != "model" || len(model.Parts) != 1 || model.Parts[0].ThoughtSignature != "signature-1" {
				t.Errorf("replayed model content = %#v", model)
			}
			call := model.Parts[0].FunctionCall
			if call == nil || call.ID != "" || call.Name != "read" || call.Args["filename"] != "note.md" {
				t.Errorf("replayed function call = %#v", call)
			}
			response := payload.Contents[2].Parts[0].FunctionResponse
			if response == nil || response.ID != "" || response.Name != "read" || response.Response["output"] != "document" {
				t.Errorf("function response = %#v", response)
			}
			io.WriteString(writer, `{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"write","args":{"markdown":"summary"}},"thoughtSignature":"signature-2"}]},"finishReason":"STOP"}]}`)
		default:
			t.Errorf("unexpected request number")
		}
	}))
	defer server.Close()

	runtime := agent.Runtime{
		Provider: New(Config{APIKey: "gemini-key", Endpoint: server.URL, Model: "test-model"}),
		Tools: []agent.Tool{
			{Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "read"}}, Execute: executeTool},
			{Definition: agent.ToolDefinition{Function: agent.ToolDeclaration{Name: "write"}}, Execute: executeTool},
		},
		Done: func() bool { return executed.Load() == 2 },
	}
	result, err := runtime.Run(context.Background(), []agent.ChatMessage{{Role: "user", Content: "read"}}, agent.Options{MaxTurns: 2, MaxToolCalls: 2})
	if err != nil || len(result.Message.ToolCalls) != 1 || result.Message.ToolCalls[0].Function.Name != "write" {
		t.Fatalf("runtime result = %#v, error = %v", result, err)
	}
	if got := executed.Load(); got != 2 {
		t.Fatalf("executed tool calls = %d", got)
	}
	if got := requestCount.Load(); got != 2 {
		t.Fatalf("request count = %d", got)
	}
}

func TestVertexUsesBearerTokenAndProjectLocationEndpoint(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/v1/projects/project/locations/europe-west1/publishers/google/models/test-model:generateContent" {
			t.Fatalf("path = %q", request.URL.Path)
		}
		if got := request.Header.Get("Authorization"); got != "Bearer adc-token" {
			t.Fatalf("authorization = %q", got)
		}
		if got := request.Header.Get("x-goog-api-key"); got != "" {
			t.Fatalf("unexpected API key = %q", got)
		}
		io.WriteString(writer, `{"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}]}`)
	}))
	defer server.Close()

	client := &Client{
		ProjectID: "project", Location: "europe-west1", Endpoint: server.URL, Model: "test-model",
		HTTPClient: server.Client(), vertex: true, tokenSource: staticTokenSource{},
	}
	result, err := client.Complete(context.Background(), agent.CompletionRequest{Messages: []agent.ChatMessage{{Role: "user", Content: "hello"}}})
	if err != nil || result.Message.Content != "ok" {
		t.Fatalf("result = %#v, error = %v", result, err)
	}
}

func TestNewVertexFindsApplicationDefaultCredentials(t *testing.T) {
	credentialsPath := filepath.Join(t.TempDir(), "application_default_credentials.json")
	if err := os.WriteFile(credentialsPath, []byte(`{"type":"authorized_user","client_id":"client","client_secret":"secret","refresh_token":"refresh"}`), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("GOOGLE_APPLICATION_CREDENTIALS", credentialsPath)

	client, err := NewVertex(context.Background(), Config{ProjectID: "project", Location: "global"})
	if err != nil {
		t.Fatal(err)
	}
	if client == nil || !client.vertex || client.tokenSource == nil {
		t.Fatalf("client = %#v", client)
	}
}

type staticTokenSource struct{}

func (staticTokenSource) Token() (*oauth2.Token, error) {
	return &oauth2.Token{AccessToken: "adc-token"}, nil
}
