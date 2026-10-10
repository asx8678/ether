# Phase 4: GLM through Chat Completions

GLM 5.3 is reached through Zro's OpenAI-compatible gateway, the same one pi uses (`zro` configures pi with `api: "openai-completions"`). The engine stays unchanged: a Chat Completions client plugs in under the `ModelClient` interface and produces the same `ResponseEvent`s as the Responses client.

Facts from the installed `@moonmath-ai/zro` 0.2.2 package:

- Gateway `https://zro.moonmath.ai/v1`; API key in `ZRO_API_KEY` (bearer auth).
- pi is configured with no `developer` role (system messages instead), `reasoning_effort` (GLM levels `none`, `high`, `max`) and `max_tokens` as the output-limit field.
- Codex is configured against the same gateway with `wire_api = "responses"`.
- The bundled catalog lists `glm-5.2` (524,288-token context, 64,000 output tokens); `glm-5.3` is checked against the live `/v1/models`.

## Provider and model

- [ ] Built-in `zro` provider: base URL above, `env_key = "ZRO_API_KEY"`, `wire_api = "chat_completions"`.
- [ ] `ModelProfile` entries for GLM 5.3 (and 5.2): context window, output limit, truncation policy, reasoning support; function-form `apply_patch`.
- [ ] Model client chosen by the provider's wire API (`ether-llm`), so the CLI no longer hard-codes the Responses client.

## Chat Completions client (`ether-llm`)

- [ ] `POST /chat/completions` with `stream: true` and `stream_options: {include_usage: true}`.
- [ ] Stream to `ResponseEvent`s: content deltas as output text; `reasoning_content` deltas as reasoning; tool-call deltas accumulated by index into one `FunctionCall` each when the stream ends; usage as token usage, with `prompt_tokens_details.cached_tokens` as cached input tokens.
- [ ] History to messages: instructions and developer items as `system`; each model response (reasoning, text, tool calls) as one `assistant` message replaying `reasoning_content` and tool-call argument strings byte for byte; tool outputs as `tool` messages.
- [ ] Tools: function tools only, converted to the chat `function` form.
- [ ] Reasoning parameters: `reasoning_effort` from config; the brief's `thinking: {type: "enabled", clear_thinking: false}` is checked against the live gateway.
- [ ] Responses-only fields (`prompt_cache_key`, `store`, `include`, Responses headers) are not sent.

## Cache rules

Already true from Phase 3: system prompt and tools are byte-identical across requests, history is append-only, tool output is truncated once when recorded, and the date is in the first message.

- [ ] Request log of the exact Chat Completions body (`--request-log FILE`), also written by `--replay` so it runs without a network.
- [ ] `scripts/check-prefix.py`: each request's `messages` start with exactly the previous request's, and `tools` and the system message are identical (the idea of Codex's `core/tests/suite/prompt_caching.rs`, as a check script since there are no tests).

## Gate

- [ ] The prefix check passes on a replayed session.
- [ ] A live ten-turn session with GLM 5.3 on your machine, including a shell command and an `apply_patch` edit; the second request reports cached tokens and the CLI shows them.
