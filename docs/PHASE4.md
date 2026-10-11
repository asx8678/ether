# Phase 4: GLM through Chat Completions

GLM 5.3 is reached through Zro's OpenAI-compatible gateway, the same one pi uses (`zro` configures pi with `api: "openai-completions"`). The engine stays unchanged: a Chat Completions client plugs in under the `ModelClient` interface and produces the same `ResponseEvent`s as the Responses client.

Facts from the installed `@moonmath-ai/zro` 0.2.2 package:

- Gateway `https://zro.moonmath.ai/v1`; API key in `ZRO_API_KEY` (bearer auth).
- pi is configured with no `developer` role (system messages instead), `reasoning_effort` (GLM levels `none`, `high`, `max`) and `max_tokens` as the output-limit field.
- Codex is configured against the same gateway with `wire_api = "responses"`.
- The bundled catalog lists `glm-5.2` (524,288-token context, 64,000 output tokens). The live `/v1/models` lists `glm-5.3` with `max_input_tokens` 1,048,576 and `max_output_tokens` 131,000.
- Live checks: two `system` messages, and a `system` message mid-conversation, are accepted; a repeated 4k-token prefix reports `prompt_tokens_details.cached_tokens` (4,160 of 4,181).

## Provider and model

- [x] Built-in `zro` provider: base URL above, `env_key = "ZRO_API_KEY"`, `wire_api = "chat_completions"`.
- [x] `ModelProfile` entries for GLM 5.3 (and 5.2): context window, truncation policy, reasoning support (no output limit is sent: the provider's default applies, and a cut-off response is handled as described in DECISIONS); function-form `apply_patch`.
- [x] Model client chosen by the provider's wire API (`ether-llm`), so the CLI no longer hard-codes the Responses client.

## Chat Completions client (`ether-llm`)

- [x] `POST /chat/completions` with `stream: true` and `stream_options: {include_usage: true}`.
- [x] Stream to `ResponseEvent`s: content deltas as output text; `reasoning_content` deltas as reasoning; tool-call deltas accumulated by index into one `FunctionCall` each when the stream ends; usage as token usage, with `prompt_tokens_details.cached_tokens` as cached input tokens.
- [x] History to messages: instructions and developer items as `system`; each model response (reasoning, text, tool calls) as one `assistant` message replaying `reasoning_content` and tool-call argument strings byte for byte; tool outputs as `tool` messages.
- [x] Tools: function tools only, converted to the chat `function` form.
- [x] Reasoning parameters: `reasoning_effort` from config (`model_reasoning_effort`: `none`, `high`, `max`), plus `thinking: {type: "enabled", clear_thinking: false}` for GLM. Both are accepted live; either one alone produces `reasoning_content`.
- [x] Responses-only fields (`prompt_cache_key`, `store`, `include`, Responses headers) are not sent.

## Cache rules

Already true from Phase 3: system prompt and tools are byte-identical across requests, history is append-only, tool output is truncated once when recorded, and the date is in the first message.

- [x] Request log of the exact Chat Completions body (`--request-log FILE`), also written by `--replay` so it runs without a network.
- [x] `scripts/check-prefix.py`: each request's `messages` start with exactly the previous request's, and `tools` and the system message are identical (the idea of Codex's `core/tests/suite/prompt_caching.rs`, as a check script since there are no tests).

## Gate

- [x] The prefix check passes on a replayed session.
- [x] A live ten-turn session with GLM 5.3 on your machine, including a shell command and an `apply_patch` edit; the second request reports cached tokens and the CLI shows them. Run on 2026-10-10: 10 turns, 19 requests, 6 shell commands, 4 patches; `check-prefix.py` passed on all 19; every request after the first was cached (for example 7256 in, 7232 cached).
