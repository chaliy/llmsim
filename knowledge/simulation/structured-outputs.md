---
type: Specification
title: "Structured Outputs and Forced Tool Calls"
description: "Schema-valid JSON for response formats and tool arguments when a request forces them, without a script."
tags:
  - llmsim
  - simulation
  - tools
---
# Structured Outputs and Forced Tool Calls

## Abstract

Clients that use structured outputs or `tool_choice` parse what comes back.
Lorem text breaks them, so a simulator that ignores these controls only
tests the happy path of plain chat. When no script or scenario is playing,
llmsim follows the controls: it calls the forced tool with arguments valid
against the tool's schema, and answers a JSON-schema format with JSON valid
against that schema. Code: [`src/schema_fake.rs`](../../src/schema_fake.rs)
(value generation) and [`src/forced.rs`](../../src/forced.rs) (request
policy); user docs in [docs/api.md](../../docs/api.md#structured-output-and-tool-calls).

## Requirements

### R1: Tool choice

**R1.1**: OpenAI `tool_choice: "required"` or a named function, and
Anthropic `{"type": "any"}` or `{"type": "tool", "name"}`, produce one tool
call with schema-valid arguments and the provider's tool stop reason.

**R1.2**: `auto` (or unset) produces a call only when `[response]
auto_tool_calls` is on and the conversation ends with a user message that is
not a tool result, so an agent loop calls once and then gets an answer.

**R1.3**: Anthropic streams tool input as `input_json_delta` chunks of the
JSON text, in a `tool_use` block after any text; a turn with only tool calls
has no text block. Scripted streaming turns with tool calls use the same path.

### R2: Structured output

**R2.1**: Chat `response_format` `json_schema`, Responses `text.format`
`json_schema` and Anthropic `output_config.format` `json_schema` answer with
JSON valid against the schema; chat `json_object` answers with a JSON object.

**R2.2**: Generation covers the structured-output subset (types, `required`,
`enum`, `const`, `anyOf`/`oneOf`, `allOf`, local `$ref`, string formats,
numeric and length bounds, `minItems`/`maxItems`); `pattern` is ignored.
Recursion stops at depth 8.

**R2.3**: Values are deterministic for the same last message.

### R3: Precedence

**R3.1**: Script and scenario turns win, then a forced tool call, then
structured output, then the configured generator.

## Non-goals

- Validating schemas the way providers do (unsupported keywords are ignored,
  not rejected).
- Content that relates to the prompt.
