# Overall plan

1. Define the minimal `chamber_agent.proto`: one chat request and streamed `started`, `text_delta`, `completed`, and `error` events.
2. Build the Python sidecar with `uv`, `grpc.aio`, and PydanticAI. Start with local Ollama and test it independently.
3. Build the Rust host with `tonic`. It starts the Python process, waits for readiness, connects over loopback, and shuts it down cleanly.
4. Scaffold the minimal Tauri application. React invokes a Rust command; Rust converts gRPC messages into Tauri events.
5. Add the React chat page with a Chamber heading, transcript, input, and streamed response rendering.
6. Test the full path: React -> Tauri -> Rust -> gRPC -> PydanticAI -> Ollama, then back through streamed events.
7. Start a second sidecar to prove per-mind process isolation and independent failure handling.
8. Package the Python runtime as a Tauri external binary and test Windows, macOS, and Linux builds.

The initial sidecar will use `glm-5.3-flash:cloud` through Bigbertha's Ollama server at `http://bigbertha:11434/v1`. Bigbertha owns the Ollama Cloud authentication.
