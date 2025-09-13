### Migrating from IPC Tick to Admin RPC manualTick

Overview
- **Goal**: Replace Unix socket tick IPC with an Admin RPC method `manualTick` that triggers a single PoH tick in multivm mode.
- **Impact**: Client code that previously connected to the dedicated tick socket now calls Admin RPC over the validator's admin IPC channel (`<ledger>/admin.rpc`).

What changed
- Old: `validator/src/bridge/ipc.rs` with `IpcClient::tick()` over a Unix domain socket path (`--tick-ipc-path`).
- New: `validator/src/admin_rpc_service.rs` adds `manualTick` method; multivm wires internal tick channels to Admin RPC. No additional ports; the Admin RPC is already running.

Prerequisites
- Use the multivm validator entrypoint so the manual tick channels are available:
  - Binary: `multivm-validator`
  - Example run:
```bash
target/debug/multivm-validator \
  --ledger /path/to/ledger \
  --rpc-port 8899 \
  --tick-ipc-path /tmp/solana-private-validator  # optional; legacy compatibility
```

Client migration
- Before (IPC):
```rust
use agave_validator::bridge::ipc::IpcClient;

let ipc = IpcClient::new("/tmp/solana-private-validator".to_string());
ipc.tick().expect("tick failed");
```

- After (Admin RPC):
```rust
use std::path::Path;
use agave_validator::admin_rpc_service;

let ledger_path = Path::new("/path/to/ledger");
let admin_client = admin_rpc_service::connect(ledger_path);
admin_rpc_service::runtime()
    .block_on(async move { admin_client.await?.manual_tick().await })
    .expect("manualTick failed");
```

Behavior parity
- Both paths trigger the same internal tick by sending on `tick_sender` and block until a `tick_done_receiver` confirms completion.
- Differences:
  - Transport: custom Unix socket vs Admin RPC over IPC file (`admin.rpc`).
  - Response: IPC returns a boolean; Admin RPC returns `null` on success or JSON-RPC error on failure.

Compatibility and deprecation
- The legacy tick IPC server is still started by multivm for compatibility. The CLI help marks it as deprecated.
- New development should use Admin RPC `manualTick`.

Troubleshooting
- Error `Invalid params: Manual tick not available yet`:
  - Cause: Channels not initialized (e.g., using standard validator entrypoint).
  - Fix: Use `multivm-validator` or ensure multivm tick channels are wired.
- `admin.rpc` not found:
  - Ensure the validator is running and the `ledger` path is correct when calling `admin_rpc_service::connect()`.
- Request appears to hang:
  - The call blocks until the tick completes. Confirm the multivm path is active and not stalled, or verify logs.

Security notes
- Admin RPC uses a local IPC file; do not expose it over the network. Ensure filesystem permissions restrict access to trusted users.

Relevant files
- `validator/src/admin_rpc_service.rs`: `manualTick` and `ManualTickChannels`
- `validator/src/multivm_validator.rs`: wires tick channels into Admin RPC
- `validator/src/cli.rs`: marks tick IPC path help as deprecated
- `validator/src/bridge/ipc.rs`: legacy IPC tick (retained for transition)


