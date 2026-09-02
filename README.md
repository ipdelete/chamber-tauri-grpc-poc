# Chamber Tauri gRPC POC

A desktop proof of concept for running framework-neutral agent sidecars behind a Tauri application.

## What it demonstrates

- React chat over a Rust Tauri command.
- Bidirectional gRPC between Rust and a Python PydanticAI sidecar.
- Streamed responses, request cancellation, and per-sidecar authentication.
- Canvas Lenses that the agent can write to a mind directory and the app renders in sandboxed iframes.

```text
React -> Tauri command -> Rust -> gRPC -> PydanticAI -> Ollama
React <- Tauri events  <- Rust <- streamed agent events
```

## Prerequisites

- Node.js and npm. The release workflow uses Node.js 26.
- Rust 1.98.0, as pinned in `.mise.toml`.
- Python 3.14.7, as pinned in `sidecar/.python-version`, and [uv](https://docs.astral.sh/uv/).
- The system dependencies required by [Tauri](https://v2.tauri.app/start/prerequisites/).
- An Ollama server with the selected model available.

## Architecture

Rust and the webview talk to the sidecar through the protobuf contract. The host only sees
protobuf messages, so the PydanticAI implementation can change without rewriting the host. The
contract does not sandbox the sidecar or its filesystem access.

Each sidecar owns its mind's tools and directory. The only tool in this POC,
`lens_upsert`, validates a Canvas Lens, writes `index.html` and `view.json` under
`<mind>/.github/lens/<id>/`, and announces a `LensChanged` snapshot.

Rust starts and stops the sidecar, carries approval messages, and decides which lenses reach the
webview. Before a tool with side effects runs, the sidecar sends an `ApprovalRequest` and waits for
an `ApprovalDecision`. Rust revalidates each `LensChanged` before handing it to the webview, which
renders it in a sandboxed iframe.

The security model is deliberately simple. A mind gets only its assigned tools, and side-effecting
tools use the approval channel. Rust currently approves every request automatically, so this is not
a user-consent system yet.

## Development

Install the JavaScript and Python dependencies:

```bash
npm ci
uv sync --project sidecar --locked
```

Start the app:

```bash
npm run tauri -- dev
```

The sidecar uses `glm-5.3-flash:cloud` at `http://bigbertha:11434/v1` by default.

### Configuration

Override either value when using another Ollama server:

```bash
OLLAMA_BASE_URL=http://localhost:11434/v1 \
CHAMBER_MODEL=qwen3 \
npm run tauri -- dev
```

| Variable | Default | Purpose |
| --- | --- | --- |
| `OLLAMA_BASE_URL` | `http://bigbertha:11434/v1` | Ollama OpenAI-compatible API endpoint |
| `CHAMBER_MODEL` | `glm-5.3-flash:cloud` | Model name sent to Ollama |

## Testing

Run the sidecar smoke test separately:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin agent-smoke
```

It starts two sidecars, sends a prompt to each, kills one, and checks that the
other still answers. It also tests invalid authentication, request cancellation,
and lens approval and denial. It uses the same Ollama and model environment
variables as the app.

If you change `proto/chamber_agent.proto`, regenerate the checked-in Python stubs:

```bash
uv run --project sidecar --locked python -m grpc_tools.protoc \
  -I proto \
  --python_out=sidecar/src \
  --grpc_python_out=sidecar/src \
  proto/chamber_agent.proto
```

## Packaging

Build the Arch Linux package:

```bash
npm run package:arch
```

PyInstaller freezes the Python sidecar into a target-specific binary. Tauri
launches that binary as an external sidecar. The Arch build produces:

```text
packaging/arch/chamber-tauri-grpc-poc-0.1.0-1-x86_64.pkg.tar.zst
```

The packaged application does not need `uv` or a system Python at runtime.

The [release workflow](.github/workflows/package-release.yml) also builds an
unsigned Windows NSIS installer.

## Help and contributing

Open an issue for bugs, questions, or proposed changes. Keep changes focused and
run the smoke test when changing sidecar behavior.

## License

This is proprietary software. The repository does not include a public license
for redistribution.
