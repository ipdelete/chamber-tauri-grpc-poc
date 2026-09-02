# Overall plan 2

- [x] Run two sidecars concurrently. Kill one and prove the other keeps working.
- [x] Add request cancellation without holding a shared mutex for the full response.
- [ ] Port one real Lens view behind a host-neutral API.
- [ ] Add a Canvas Lens fixture and prove iframe behavior in Tauri.
- [ ] Authenticate each loopback gRPC connection with an ephemeral process token.
