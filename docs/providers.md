# Providers

bo supports OpenAI-compatible providers. Each built-in provider enumerates a fixed set of models; the `custom` provider accepts any OpenAI-compatible endpoint and model.

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

## Z.ai (GLM)

bo targets the Z.ai **GLM Coding Plan** (subscription) endpoint, not the pay-per-token PaaS endpoint.

| Model | Context | Notes |
|---|---|---|
| `glm-4.7` | 200K | Default. Routine tasks, 1x quota consumption. |
| `glm-4.5-air` | 200K | Lightweight text model. |
| `glm-5.1` | 200K | Flagship-class. |
| `glm-5-turbo` | 200K | Fast flagship-class, 2–3x quota. |
| `glm-5.2` | 1M | Flagship. Complex/large compiles, 2–3x quota. |

### Notes

- **Coding Plan endpoint** — bo hits `https://api.z.ai/api/coding/paas/v4/chat/completions` with Bearer auth. Requires a GLM Coding Plan subscription; a pay-per-token PaaS key returns `code 1113` (insufficient balance). Pin a compile model (`bo config --compile-model glm-5.2`) for heavier work.
- **Quota** — glm-5.2 / glm-5-turbo consume 2–3x quota (3x peak 14:00–18:00 UTC+8); glm-4.7 consumes 1x. Prefer glm-4.7 for routine queries.
- **No structured output mode** — like DeepSeek, Z.ai has no `response_format: json_schema`. bo falls back to JSON-mode prompting (system message instructions + `response_format: json_object`).
- **reasoning toggle** — `{"thinking": {"type": "disabled"}}` suppresses reasoning tokens (same parameter shape as DeepSeek).
- **Vision not supported** — the plan exposes `glm-5v-turbo`, but bo is text-only; it is intentionally not listed.
- **Auth** — set `ZAI_API_KEY` or store `zai_api_key` in `~/.bo/auth.json`.

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

---

## Custom (any OpenAI-compatible endpoint)

Point bo at any endpoint that speaks the OpenAI chat-completions dialect — a
self-hosted model, a proxy, or a compatibility layer:

```bash
bo config --provider custom --base-url https://api.example.com/v1 --model my-model
export CUSTOM_API_KEY=...
```

### Notes

- **Base URL** — everything before `/chat/completions`; bo appends that path.
  Required: `bo config` refuses to select `custom` without one.
- **Models** — no registry; any non-empty model id is accepted and passed through.
- **Context window** — assumed 128K tokens (conservative fixed default).
- **No structured output mode** — treated like DeepSeek/Z.ai: JSON-mode prompting
  (system message instructions + `response_format: json_object`), with the
  validation gate catching malformed responses.
- **Auth** — set `CUSTOM_API_KEY` or store `custom_api_key` in `~/.bo/auth.json`.
  Sent as a Bearer token.
