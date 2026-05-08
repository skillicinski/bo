# Research: Structured Output via async-openai

## Structured Output Support

**Question:** Does `async-openai 0.34` support `response_format: json_schema`?

**Answer:** Yes. The `CreateChatCompletionRequest` type has a `response_format` field accepting `ResponseFormat::JsonSchema { json_schema }`. This was added for OpenAI's structured output feature (GA since August 2024).

Usage pattern:
```rust
use async_openai::types::chat::{
    CreateChatCompletionRequestArgs,
    ResponseFormat, JsonSchema,
};

let request = CreateChatCompletionRequestArgs::default()
    .model(model)
    .messages(messages)
    .response_format(ResponseFormat::JsonSchema {
        json_schema: JsonSchema {
            name: "compile_response".to_string(),
            description: Some("Concepts extracted from the document collection".to_string()),
            schema: schema_value,  // serde_json::Value
            strict: Some(true),
        },
    })
    .build()?;
```

With `strict: true`, the API guarantees the response conforms to the schema. Parse failures become impossible (barring network issues or refusals).

## Model Compatibility

Structured output with JSON schema is supported by:
- gpt-4o (all versions)
- gpt-4o-mini (all versions)
- gpt-4.1 and later

Bo's default compile model (`gpt-4o-mini` or user-configured) is guaranteed compatible.

## Token Limits

Context window overflow manifests as an API error (400 status, message mentioning token limit). The pipeline catches this at the `LlmProvider::complete()` level and surfaces it as a user-facing error. No pre-flight token counting is strictly necessary — the API is the authoritative check.

A rough heuristic for early warning (optional): `total_chars / 4 > model_context_limit * 0.9`. This can inform a warning before the call but is not a gate.

## No Other Technical Unknowns

- `serde_json::Value` for schema definition: already in deps
- `domain::branch::write()` and `domain::frontmatter::patch_fields()`: proven, tested
- Prompt design: out of scope for this research (iterative post-merge)
