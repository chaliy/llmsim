# API Endpoints Specification

## Abstract

This specification defines the URL structure and routing conventions for LLMSim API endpoints. The design ensures compatibility with official provider SDKs while supporting multiple LLM providers through a unified service.

## Requirements

### R1: Provider-Prefixed Endpoints

**R1.1**: All provider-specific API endpoints MUST be prefixed with the provider name, followed by the original API path.

**Pattern:** `/{provider}{original_api_path}`

**R1.2**: The path after the provider prefix MUST exactly match the original provider's API path. This ensures that official SDKs work correctly when configured with `{base_url}/{provider}` as their base URL.

**R1.3**: Supported providers and their base paths:
| Provider | Prefix | Example Original Path | LLMSim Path |
|----------|--------|----------------------|-------------|
| OpenAI | `/openai` | `/v1/chat/completions` | `/openai/v1/chat/completions` |
| OpenAI | `/openai` | `/v1/responses` | `/openai/v1/responses` |
| OpenAI | `/openai` | `/v1/models` | `/openai/v1/models` |
| Anthropic | `/anthropic` | `/v1/messages` | `/anthropic/v1/messages` |

### R2: OpenAI Endpoints

**R2.1**: Implement the following OpenAI-compatible endpoints:

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/openai/v1/chat/completions` | Chat Completions API |
| `POST` | `/openai/v1/responses` | Responses API |
| `GET` | `/openai/v1/models` | List available models |
| `GET` | `/openai/v1/models/:model_id` | Get model details |

**R2.2**: These endpoints accept the same request/response formats as the official OpenAI API.

**R2.3**: The models endpoint (`/openai/v1/models`) returns extended model information sourced from [models.dev](https://models.dev):

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Model identifier (e.g., "gpt-5") |
| `object` | string | Always "model" |
| `created` | integer | Unix timestamp of model release |
| `owned_by` | string | Model owner (e.g., "openai", "anthropic") |
| `context_window` | integer | Maximum input tokens (e.g., 400000 for GPT-5) |
| `max_output_tokens` | integer | Maximum output tokens (e.g., 128000 for GPT-5) |

### R3: Anthropic Endpoints (Future)

**R3.1**: When implemented, Anthropic endpoints MUST follow the same pattern:

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/anthropic/v1/messages` | Messages API |

### R4: System Endpoints

**R4.1**: System endpoints are not provider-specific and use simple paths:

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check endpoint |
| `GET` | `/llmsim/stats` | Server statistics (requests, tokens, latency) |

### R5: SDK Compatibility

**R5.1**: When using official SDKs, configure the base URL as follows:

```python
# OpenAI Python SDK
from openai import OpenAI
client = OpenAI(
    base_url="http://localhost:8080/openai/v1",
    api_key="not-needed"
)
```

```typescript
// OpenAI Node.js SDK
import OpenAI from 'openai';
const client = new OpenAI({
    baseURL: 'http://localhost:8080/openai/v1',
    apiKey: 'not-needed'
});
```

```python
# Anthropic Python SDK (future)
from anthropic import Anthropic
client = Anthropic(
    base_url="http://localhost:8080/anthropic",
    api_key="not-needed"
)
```

## Rationale

### Why provider prefixes?

1. **Multi-provider support**: A single LLMSim instance can simulate multiple providers
2. **SDK compatibility**: Official SDKs work with minimal configuration changes
3. **Clear routing**: Request routing is unambiguous based on the URL path
4. **Future extensibility**: New providers can be added without path conflicts

### Why preserve `/v1/` in paths?

1. **SDK expectations**: Official SDKs append paths like `/chat/completions` to the base URL
2. **API versioning**: Preserves the original API version semantics
3. **Documentation alignment**: Examples from provider docs work with minimal changes

## Non-Requirements

- Authentication/authorization (LLMSim is for local testing)
- Rate limiting enforcement (simulated via error injection)
- Request logging/analytics endpoints
