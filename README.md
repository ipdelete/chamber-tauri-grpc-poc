# Chamber Tauri gRPC POC

A small proof of concept for running framework-neutral agent sidecars behind a Tauri desktop application.

```text
React -> Tauri command -> Rust -> gRPC -> PydanticAI -> Ollama
React <- Tauri events  <- Rust <- streamed agent events
```

The sidecar boundary is a framework boundary, not a privilege boundary. Framework types stop at the
protobuf contract so the framework behind it can be replaced without rewriting Chamber.

The sidecar represents a mind, so it owns the mind's tools and writes the mind's directory. Today
that is `lens_upsert`, which validates a Canvas Lens, writes `index.html` and `view.json` under
`<mind>/.github/lens/<id>/`, and announces a `LensChanged` snapshot.

Rust core keeps three jobs: the sidecar process lifecycle, the consent channel, and the renderer.
Before a tool with side effects runs, the sidecar sends an `ApprovalRequest` and waits for an
`ApprovalDecision`. This POC auto-approves every request; the approval UI is not built yet. Rust
revalidates each `LensChanged` before handing it to the webview, which renders it in a sandboxed
iframe.

Security is policy, expressed two ways: user approval before an action, and restricting which tools a
mind is given.

## Run in development

Install the JavaScript and Python dependencies:

```bash
npm install
uv sync --project sidecar
```

Start the app:

```bash
npm run tauri -- dev
```

The sidecar defaults to `glm-5.3-flash:cloud` on `http://bigbertha:11434/v1`. Override either value when using another Ollama server:

```bash
OLLAMA_BASE_URL=http://localhost:11434/v1 \
CHAMBER_MODEL=qwen3 \
npm run tauri -- dev
```

## Build the Arch Linux package

```bash
npm run package:arch
```

The build uses PyInstaller to create a target-specific standalone sidecar, registers it as a Tauri external binary, and produces:

```text
packaging/arch/chamber-tauri-grpc-poc-0.1.0-1-x86_64.pkg.tar.zst
```

The packaged application does not require `uv` or a system Python at runtime.
