# Providers

bo supports OpenAI-compatible providers. Each provider enumerates a fixed set of models.

---

## OpenAI

| Model | Notes |
|---|---|
| `gpt-4.1-mini` | Default. Best cost/performance balance for most trees. |
| `gpt-4.1` | Higher compile quality for large or complex trees. |
| `gpt-4.1-nano` | Smallest context window. Fast but hits compile limits sooner. |
| `gpt-4o` | Legacy. Prefer `gpt-4.1`. |
| `gpt-4o-mini` | Legacy. Prefer `gpt-4.1-mini` or `gpt-4.1-nano`. |

### Notes

- **Structured output mode** — bo uses OpenAI's `response_format: json_schema` to guarantee well-formed compile responses. Schema normalization is enforced at the type level across all providers.
- **Model selection** — choose at seed time (flag or interactive prompt). No silent default; `bo seed` always requires an explicit `--model` or prompts for one.

---

## DeepSeek

| Model | Notes |
|---|---|
| `deepseek-v4-flash` | Default DeepSeek model. |
| `deepseek-v4-pro` | Larger context, higher quality output. |

### Notes

- **No structured output mode** — DeepSeek's API does not support `response_format: json_schema`. bo falls back to JSON-mode prompting (system message instructions + `response_format: json_object`). Schema normalization is enforced at the type level; validation gate catches any malformed responses.

---

## Google (Gemini)

| Model | Notes |
|---|---|
| `gemini-2.5-flash-lite` | Cheapest, fastest. Good for simple queries. |
| `gemini-2.5-flash` | Default Google model. Best cost/performance balance. |
| `gemini-2.5-pro` | Higher quality for large or complex trees. |

### Notes

- **Native API** — bo uses the Gemini native `generateContent` endpoint, not the OpenAI compatibility layer. Auth goes via `x-goog-api-key` header.
- **Structured output** — Gemini supports `responseSchema` with `responseMimeType: application/json`. bo uses this for compile responses.
- **System instructions** — System messages are mapped to Gemini's `systemInstruction` field rather than inserted as conversation turns.
- **Auth** — set `GEMINI_API_KEY` or store `google_api_key` in `~/.bo/auth.json`.
