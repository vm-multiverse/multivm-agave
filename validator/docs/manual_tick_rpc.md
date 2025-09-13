### Multivm Manual Tick via Admin RPC

Overview
- **Purpose**: Replace local Unix socket IPC tick trigger with an Admin RPC method, enabling remote/manual ticks for multivm PoH.
- **Scope**: Adds `manualTick` to Admin RPC. Keeps legacy IPC server for backward compatibility in multivm validator.

API Reference
- **Method**: `manualTick`
- **Namespace**: Admin RPC (`validator/src/admin_rpc_service.rs`)
- **Params**: none
- **Returns**: null on success; RPC error on failure
- **Behavior**:
  - Triggers one manual PoH tick by sending a signal on the tick channel
  - Blocks until tick completion is acknowledged

Usage Examples
- JSON-RPC (IPC transport to `admin.rpc` file)
```json
{"jsonrpc":"2.0", "id":1, "method":"manualTick", "params":[]}
```

Implementation Notes
- The channels are available after multivm startup initializes the tick channels.
- If channels are not yet available, `manualTick` returns `Invalid params: Manual tick not available yet`.

Design Decisions
- Reused Admin RPC service to avoid introducing a new server/port.
- Kept legacy IPC tick server for transition; will be deprecated in future release.

Tests
- Manual invocation via JSON-RPC should succeed during multivm runs.
- Existing tests referencing IPC tick remain unchanged.

File Map
- `validator/src/admin_rpc_service.rs`: adds `manualTick` and `ManualTickChannels` plumbing
- `validator/src/multivm_validator.rs`: wires tick channels into Admin RPC metadata
- `validator/src/bridge/ipc.rs`: unchanged; still supported for compatibility


