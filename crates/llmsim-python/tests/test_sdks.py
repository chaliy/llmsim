"""Official SDKs pointed at the in-process server."""

import pytest

from llmsim import Server

openai = pytest.importorskip("openai")
anthropic = pytest.importorskip("anthropic")


@pytest.fixture(scope="module")
def sim():
    with Server(generator="echo", latency="instant") as server:
        yield server


def test_openai_chat_completion(sim):
    client = openai.OpenAI(base_url=sim.openai_base_url, api_key="unused")
    reply = client.chat.completions.create(model="gpt-5", messages=[{"role": "user", "content": "hello sdk"}])
    assert reply.choices[0].message.content == "Echo: hello sdk"
    assert reply.usage.total_tokens > 0


def test_openai_streaming(sim):
    client = openai.OpenAI(base_url=sim.openai_base_url, api_key="unused")
    stream = client.chat.completions.create(
        model="gpt-5", messages=[{"role": "user", "content": "stream me"}], stream=True
    )
    text = "".join(chunk.choices[0].delta.content or "" for chunk in stream if chunk.choices)
    assert text == "Echo: stream me"


def test_openai_responses(sim):
    client = openai.OpenAI(base_url=sim.openai_base_url, api_key="unused")
    resp = client.responses.create(model="gpt-5", input="responses api")
    assert resp.output_text == "Echo: responses api"


def test_anthropic_messages(sim):
    client = anthropic.Anthropic(base_url=sim.anthropic_base_url, api_key="unused")
    msg = client.messages.create(
        model="claude-sonnet-4-6", max_tokens=100, messages=[{"role": "user", "content": "hi claude"}]
    )
    assert msg.content[0].text == "Echo: user: hi claude"


def test_async_openai_client():
    import asyncio

    async def main():
        async with Server(generator="echo", latency="instant") as server:
            client = openai.AsyncOpenAI(base_url=server.openai_base_url, api_key="unused")
            reply = await client.chat.completions.create(model="gpt-5", messages=[{"role": "user", "content": "async"}])
            return reply.choices[0].message.content

    assert asyncio.run(main()) == "Echo: async"
