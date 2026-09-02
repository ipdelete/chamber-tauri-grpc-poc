# Chamber Tauri gRPC POC

A small proof of concept for running framework-neutral agent sidecars behind a Tauri desktop application.

```text
React -> Tauri command -> Rust -> gRPC -> PydanticAI -> Ollama
React <- Tauri events  <- Rust <- streamed agent events
```

Rust owns the sidecar process. The shared protobuf contract contains Chamber concepts rather than PydanticAI types.

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
