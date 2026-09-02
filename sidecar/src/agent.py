import argparse
import asyncio
import json
import os
from collections.abc import AsyncIterator, Awaitable, Callable
from pathlib import Path
from typing import Any

from pydantic_ai import Agent
from pydantic_ai.capabilities import HandleDeferredToolCalls
from pydantic_ai.exceptions import ModelRetry, ToolFailed
from pydantic_ai.models.ollama import OllamaModel
from pydantic_ai.providers.ollama import OllamaProvider
from pydantic_ai.tools import (
    DeferredToolRequests,
    DeferredToolResults,
    RunContext,
    ToolDenied,
)
from pydantic_ai.toolsets import ApprovalRequiredToolset, FunctionToolset

DEFAULT_BASE_URL = "http://bigbertha:11434/v1"
DEFAULT_MODEL = "glm-5.3-flash:cloud"

MAX_HTML_BYTES = 512 * 1024
ID_ALPHABET = frozenset("abcdefghijklmnopqrstuvwxyz0123456789-")

# Returns None when approved, or a denial reason.
Approve = Callable[[str, str, str], Awaitable[str | None]]
# Receives a lens snapshot: id, name, icon, html.
LensListener = Callable[[dict[str, str]], Awaitable[None]]

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


def validate_lens(id: str, name: str, icon: str, html: str) -> None:
    """Raise ModelRetry with a reason the model can act on."""
    if (
        not id
        or len(id.encode()) > 64
        or not set(id) <= ID_ALPHABET
        or id.startswith("-")
        or id.endswith("-")
    ):
        raise ModelRetry("Lens id must use 1-64 lowercase letters, numbers, or hyphens")
    if not name.strip() or len(name.encode()) > 80:
        raise ModelRetry("Lens name must use 1-80 characters")
    if not icon.strip() or len(icon.encode()) > 32:
        raise ModelRetry("Lens icon must use 1-32 characters")
    if len(html.encode()) > MAX_HTML_BYTES:
        raise ModelRetry("Lens HTML exceeds the 512 KiB limit")
    lowercase = html.lower()
    if "<html" not in lowercase or "</html>" not in lowercase:
        raise ModelRetry("Lens HTML must be a complete HTML document")


def write_lens(mind_root: Path, id: str, name: str, icon: str, html: str) -> None:
    """Write the lens into the mind directory. Raise ToolFailed on an I/O failure."""
    directory = mind_root / ".github" / "lens" / id
    manifest = json.dumps(
        {"name": name, "icon": icon, "view": "canvas", "source": "index.html"},
        indent=2,
        ensure_ascii=False,
    )
    try:
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "index.html").write_text(html, encoding="utf-8")
        (directory / "view.json").write_text(f"{manifest}\n", encoding="utf-8")
    except OSError as error:
        raise ToolFailed(f"Could not write lens {id!r}: {error}") from error


def build_agent(
    mind_root: Path | None = None,
    approve: Approve | None = None,
    on_lens_changed: LensListener | None = None,
) -> Agent[None, str]:
    provider = OllamaProvider(
        base_url=os.getenv("OLLAMA_BASE_URL", DEFAULT_BASE_URL),
    )
    model = OllamaModel(
        os.getenv("CHAMBER_MODEL", DEFAULT_MODEL),
        provider=provider,
    )

    if mind_root is None or approve is None:
        return Agent(model)

    async def lens_upsert(id: str, name: str, icon: str, html: str) -> str:
        """Create or replace a sandboxed Canvas Lens in Chamber."""
        validate_lens(id, name, icon, html)
        await asyncio.to_thread(write_lens, mind_root, id, name, icon, html)
        if on_lens_changed is not None:
            await on_lens_changed({"id": id, "name": name, "icon": icon, "html": html})
        return json.dumps({"ok": True, "id": id, "message": "Lens saved and displayed"})

    async def handle_deferred(
        ctx: RunContext[Any],
        requests: DeferredToolRequests,
    ) -> DeferredToolResults:
        decisions: dict[str, Any] = {}
        for call in requests.approvals:
            denial = await approve(
                call.tool_call_id,
                call.tool_name,
                call.args_as_json_str(),
            )
            decisions[call.tool_call_id] = True if denial is None else ToolDenied(denial)
        return requests.build_results(approvals=decisions)

    return Agent(
        model,
        instructions=LENS_INSTRUCTIONS,
        toolsets=[ApprovalRequiredToolset(FunctionToolset([lens_upsert]))],
        capabilities=[HandleDeferredToolCalls(handler=handle_deferred)],
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
