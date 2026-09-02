# Overall plan

- [x] Define the minimal `chamber_agent.proto`: one chat request and streamed `started`, `text_delta`, `completed`, and `error` events.
- [x] Build the Python sidecar with `uv`, `grpc.aio`, and PydanticAI. Start with local Ollama and test it independently.
- [x] Build the Rust host with `tonic`. It starts the Python process, waits for readiness, connects over loopback, and shuts it down cleanly.
- [x] Scaffold the minimal Tauri application. React invokes a Rust command; Rust converts gRPC messages into Tauri events.
- [x] Add the React chat page with a Chamber heading, transcript, input, and streamed response rendering.
- [x] Test the full path: React -> Tauri -> Rust -> gRPC -> PydanticAI -> Ollama, then back through streamed events.
- [x] Start a second sidecar to prove per-mind process isolation and independent failure handling.
- [x] Package the Python runtime as a Tauri external binary and test Windows and Linux builds. macOS is deferred.

The initial sidecar will use `glm-5.3-flash:cloud` through Bigbertha's Ollama server at `http://bigbertha:11434/v1`. Bigbertha owns the Ollama Cloud authentication.
