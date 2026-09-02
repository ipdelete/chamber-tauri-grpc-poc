# Overall plan 2

- [x] Run two sidecars concurrently. Kill one and prove the other keeps working.
- [x] Add request cancellation without holding a shared mutex for the full response.
- [x] Prove bidirectional host tools by letting the agent create and revise a sandboxed Canvas Lens through `lens_upsert`.
- [x] Authenticate each loopback gRPC connection with an ephemeral process token.
