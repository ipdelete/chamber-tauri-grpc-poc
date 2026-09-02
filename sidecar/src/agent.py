import argparse
import asyncio
import os
from collections.abc import AsyncIterator

from pydantic_ai import Agent
from pydantic_ai.models.ollama import OllamaModel
from pydantic_ai.providers.ollama import OllamaProvider

DEFAULT_BASE_URL = "http://bigbertha:11434/v1"
DEFAULT_MODEL = "glm-5.3-flash:cloud"


def build_agent() -> Agent[None, str]:
    provider = OllamaProvider(
        base_url=os.getenv("OLLAMA_BASE_URL", DEFAULT_BASE_URL),
    )
    model = OllamaModel(
        os.getenv("CHAMBER_MODEL", DEFAULT_MODEL),
        provider=provider,
    )
    return Agent(model)


async def stream_response(agent: Agent[None, str], prompt: str) -> AsyncIterator[str]:
    async with agent.run_stream(prompt) as response:
        async for text in response.stream_text(delta=True):
            yield text


async def print_response(prompt: str) -> None:
    async for text in stream_response(build_agent(), prompt):
        print(text, end="", flush=True)
    print()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("prompt")
    args = parser.parse_args()
    asyncio.run(print_response(args.prompt))


if __name__ == "__main__":
    main()