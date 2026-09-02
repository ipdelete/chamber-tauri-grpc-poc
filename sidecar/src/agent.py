import argparse
import asyncio
import json
import os
from collections.abc import AsyncIterator, Awaitable, Callable

from pydantic_ai import Agent
from pydantic_ai.models.ollama import OllamaModel
from pydantic_ai.providers.ollama import OllamaProvider

DEFAULT_BASE_URL = "http://bigbertha:11434/v1"
DEFAULT_MODEL = "glm-5.3-flash:cloud"

HostTool = Callable[[str, str], Awaitable[str]]

LENS_INSTRUCTIONS = """
You are an agent inside Chamber. When the user asks for a dashboard, view,
panel, report, form, command center, or change to the current UI, use the
lens_upsert tool. Create a complete, self-contained HTML document. Use a
short lowercase ID containing only letters, numbers, and hyphens. Canvas
buttons may call window.canvas.sendAction(action, data) to send intent back
to you. Do not use external scripts, styles, images, or network requests.
Use the Chamber classes ch-page, ch-grid, ch-card, ch-button,
ch-button-secondary, ch-input, ch-table, ch-badge, and ch-muted.
"""


def build_agent(host_tool: HostTool | None = None) -> Agent[None, str]:
    provider = OllamaProvider(
        base_url=os.getenv("OLLAMA_BASE_URL", DEFAULT_BASE_URL),
    )
    model = OllamaModel(
        os.getenv("CHAMBER_MODEL", DEFAULT_MODEL),
        provider=provider,
    )

    if host_tool is None:
        return Agent(model)

    async def lens_upsert(
        id: str,
        name: str,
        icon: str,
        html: str,
    ) -> str:
        """Create or replace a sandboxed Canvas Lens in Chamber."""
        return await host_tool(
            "lens_upsert",
            json.dumps(
                {"id": id, "name": name, "icon": icon, "html": html},
                separators=(",", ":"),
            ),
        )

    return Agent(
        model,
        instructions=LENS_INSTRUCTIONS,
        tools=[lens_upsert],
    )


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