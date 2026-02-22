# OpenAI Responses API Specification

## Abstract

The OpenAI Responses API is a stateful API that unifies the Chat Completions and Assistants API capabilities into a single, more powerful interface. This specification defines the simulated Responses API endpoints for LLMSim, enabling developers to test agentic workflows without incurring API costs.

## Requirements

### R1: Core Endpoint

**R1.1**: Implement `POST /openai/v1/responses` endpoint that accepts Responses API requests and returns simulated responses.

**R1.2**: Support both string and array input formats:
- Simple string: `"input": "Hello, world!"`
- Array of messages with roles: `"input": [{"role": "user", "content": "Hello"}]`

**R1.3**: Return response structure with:
- `id`: Unique response identifier (format: `resp_<uuid>`)
- `object`: Always `"response"`
- `created_at`: Unix timestamp
- `model`: Requested model name
- `status`: Response status (`"completed"`, `"failed"`, `"in_progress"`)
- `output`: Array of output items
- `output_text`: Simplified text representation
- `usage`: Token usage statistics

### R2: Input Item Types

**R2.1**: Support message input items:
```json
{
  "type": "message",
  "role": "user|assistant|system",
  "content": "string or array"
}
```

**R2.2**: Support content part types in message content arrays:
- `input_text`: `{"type": "input_text", "text": "..."}`
- `input_image`: `{"type": "input_image", "image_url": "..."}`

### R3: Output Item Types

**R3.1**: Generate message output items:
```json
{
  "type": "message",
  "id": "msg_<uuid>",
  "role": "assistant",
  "status": "completed",
  "content": [{"type": "output_text", "text": "..."}]
}
```

**R3.2**: Generate reasoning output items for reasoning models (o-series and GPT-5 family) when reasoning tokens are produced (effort is not `"none"`). The reasoning item appears before the message item in the output array:
```json
{
  "type": "reasoning",
  "id": "rs_<uuid>",
  "status": "completed",
  "summary": [{"type": "summary_text", "text": "..."}]
}
```

**R3.2.1**: The `summary` field is only present when the request's `reasoning.summary` is set to `"auto"`, `"concise"`, or `"detailed"`. When not requested, the reasoning item has `summary: null`.

**R3.2.2**: Summary text length scales with the summary mode:
- `"concise"`: ~5% of reasoning tokens as words
- `"auto"`: ~10% of reasoning tokens as words
- `"detailed"`: ~15% of reasoning tokens as words

### R4: Request Parameters

**R4.1**: Support core parameters:
- `model` (required): Model identifier
- `input` (required): String or array of input items
- `instructions`: System instructions for this request
- `temperature`: Sampling temperature (0.0 - 2.0)
- `top_p`: Nucleus sampling parameter
- `max_output_tokens`: Maximum tokens to generate
- `stream`: Enable streaming response

**R4.2**: Support optional parameters:
- `metadata`: Custom key-value metadata
- `previous_response_id`: Chain responses together (stateful multi-turn)
- `tool_choice`: Control tool usage
- `reasoning`: Reasoning configuration for reasoning models (o-series and GPT-5)
- `background`: Enable async processing for long-running tasks
- `include`: Request additional data in response (e.g., `["reasoning.encrypted_content"]`)

### R4.3: Reasoning Configuration

Support reasoning configuration for reasoning models (o-series and GPT-5 family):

**Supported Models:**
- o-series: o1, o3, o4-mini (explicit reasoning models with chain-of-thought)
- GPT-5 family: gpt-5, gpt-5-mini, gpt-5-nano, gpt-5.1, gpt-5.2 (trained with RL for reasoning)
- GPT-4.1 series: gpt-4.1, gpt-4.1-mini, gpt-4.1-nano (also supports tools in Responses API)

```json
{
  "reasoning": {
    "effort": "none|minimal|low|medium|high|xhigh",
    "summary": "auto|concise|detailed"
  }
}
```

**R4.3.1**: Simulate reasoning tokens based on effort level:
- `none`: No reasoning tokens
- `minimal`: ~0.5x output tokens as reasoning (GPT-5 only, fastest)
- `low`: ~1.5x output tokens as reasoning
- `medium`: ~3x output tokens as reasoning (default)
- `high`: ~6x output tokens as reasoning
- `xhigh`: ~10x output tokens as reasoning (GPT-5.2 only, most thorough)

**R4.3.2**: Include reasoning tokens in usage statistics:
```json
{
  "usage": {
    "input_tokens": 100,
    "output_tokens": 50,
    "total_tokens": 300,
    "output_tokens_details": {
      "reasoning_tokens": 150
    }
  }
}
```

### R5: Streaming

**R5.1**: When `stream: true`, return Server-Sent Events with event types:
- `response.created`: Initial response creation
- `response.in_progress`: Generation started
- `response.output_item.added`: New output item
- `response.content_part.added`: New content part
- `response.output_text.delta`: Text chunk
- `response.output_text.done`: Text complete
- `response.content_part.done`: Content part complete
- `response.output_item.done`: Output item complete
- `response.completed`: Response finished with usage

**R5.1.1**: When reasoning is enabled, additional streaming events are emitted for the reasoning output item before the message output item:
- `response.output_item.added`: Reasoning item at `output_index` 0
- `response.reasoning_summary_part.added`: Summary part (when summary requested)
- `response.reasoning_summary_text.delta`: Summary text chunks (when summary requested)
- `response.reasoning_summary_text.done`: Summary text complete
- `response.reasoning_summary_part.done`: Summary part complete
- `response.output_item.done`: Reasoning item complete
- The message output item follows at `output_index` 1

**R5.2**: Each SSE event format:
```
event: <event_type>
data: <json_payload>

```

**R5.3**: Include `sequence_number` in delta events for ordering. Sequence numbers are shared across reasoning summary deltas and message text deltas within the same response.

### R6: Usage Statistics

**R6.1**: Return token usage in completed responses:
```json
{
  "usage": {
    "input_tokens": 100,
    "output_tokens": 50,
    "total_tokens": 150,
    "output_tokens_details": {
      "reasoning_tokens": 0
    }
  }
}
```

### R7: Error Handling

**R7.1**: Use same error injection system as Chat Completions.

**R7.2**: Return errors in Responses API format:
```json
{
  "error": {
    "type": "rate_limit_error",
    "message": "Rate limit exceeded",
    "code": "rate_limit_exceeded"
  }
}
```

### R8: Model Support

**R8.1**: Support same model list as Chat Completions endpoint.

**R8.2**: Use model-specific latency profiles for realistic simulation.

### R9: Tool Definitions

**R9.1**: Accept tool definitions in requests (parsed but not executed):
- `function`: Custom function definitions with name, description, parameters
- `web_search`: Web search capability
- `file_search`: File search capability
- `code_interpreter`: Python code execution in sandboxed environment
- `mcp`: Remote MCP (Model Context Protocol) server with server_url and optional headers
- `image_generation`: Image generation via gpt-image-1 series

**R9.2**: Support `tool_choice` parameter:
- `"auto"`: Model decides when to use tools
- `"none"`: Disable tool usage
- `"required"`: Force tool usage
- `{"type": "function", "name": "..."}`: Force specific function

## Non-Requirements (Out of Scope for Simulation)

- Actual tool execution (tools are parsed but responses are simulated)
- MCP server connections (server_url accepted but not connected)
- Image generation output (accepted but not produced)
- Audio processing
- Background processing polling (background flag accepted, returns immediately)
- Conversation persistence (previous_response_id accepted but not stored)

## API Examples

### Simple Text Request

```bash
curl -X POST http://localhost:8080/openai/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "What is the capital of France?"
  }'
```

### Response

```json
{
  "id": "resp_abc123",
  "object": "response",
  "created_at": 1234567890,
  "model": "gpt-5",
  "status": "completed",
  "output": [
    {
      "type": "message",
      "id": "msg_xyz789",
      "role": "assistant",
      "status": "completed",
      "content": [
        {
          "type": "output_text",
          "text": "The capital of France is Paris."
        }
      ]
    }
  ],
  "output_text": "The capital of France is Paris.",
  "usage": {
    "input_tokens": 8,
    "output_tokens": 8,
    "total_tokens": 16,
    "output_tokens_details": {
      "reasoning_tokens": 0
    }
  }
}
```

### Reasoning Request

```bash
curl -X POST http://localhost:8080/openai/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "o3",
    "input": "What is 2+2?",
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    }
  }'
```

### Reasoning Response

```json
{
  "id": "resp_abc123",
  "object": "response",
  "created_at": 1234567890,
  "model": "o3",
  "status": "completed",
  "output": [
    {
      "type": "reasoning",
      "id": "rs_def456",
      "status": "completed",
      "summary": [
        {
          "type": "summary_text",
          "text": "The model considered evaluating possible approaches..."
        }
      ]
    },
    {
      "type": "message",
      "id": "msg_xyz789",
      "role": "assistant",
      "status": "completed",
      "content": [
        {
          "type": "output_text",
          "text": "2 + 2 = 4."
        }
      ]
    }
  ],
  "output_text": "2 + 2 = 4.",
  "usage": {
    "input_tokens": 5,
    "output_tokens": 8,
    "total_tokens": 37,
    "output_tokens_details": {
      "reasoning_tokens": 24
    }
  }
}
```

### Streaming Request

```bash
curl -X POST http://localhost:8080/openai/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "Tell me a story",
    "stream": true
  }'
```
