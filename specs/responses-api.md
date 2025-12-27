# OpenAI Responses API Specification

## Abstract

The OpenAI Responses API is a stateful API that unifies the Chat Completions and Assistants API capabilities into a single, more powerful interface. This specification defines the simulated Responses API endpoints for LLMSim, enabling developers to test agentic workflows without incurring API costs.

## Requirements

### R1: Core Endpoint

**R1.1**: Implement `POST /openai/responses` endpoint that accepts Responses API requests and returns simulated responses.

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
- `previous_response_id`: Chain responses together
- `tool_choice`: Control tool usage
- `reasoning`: Reasoning configuration for o-series models

### R4.3: Reasoning Configuration

Support reasoning configuration for o-series models (o1, o3, o4-mini, etc.):
```json
{
  "reasoning": {
    "effort": "none|low|medium|high",
    "summary": "auto|concise|detailed"
  }
}
```

**R4.3.1**: Simulate reasoning tokens based on effort level:
- `none`: No reasoning tokens
- `low`: ~1.5x output tokens as reasoning
- `medium`: ~3x output tokens as reasoning (default)
- `high`: ~6x output tokens as reasoning

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

**R5.2**: Each SSE event format:
```
event: <event_type>
data: <json_payload>

```

**R5.3**: Include `sequence_number` in delta events for ordering.

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

## Non-Requirements (Out of Scope)

- Tool execution (web_search, file_search, code_interpreter)
- MCP server integration
- Image generation
- Audio processing
- Background processing mode
- Conversation persistence (previous_response_id chains)

## API Examples

### Simple Text Request

```bash
curl -X POST http://localhost:8080/openai/responses \
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

### Streaming Request

```bash
curl -X POST http://localhost:8080/openai/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5",
    "input": "Tell me a story",
    "stream": true
  }'
```
