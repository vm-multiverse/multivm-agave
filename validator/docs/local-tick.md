# Multivm-Validator Automatic Ticking

multivm-validator now ticks automatically. Manual ticks via Admin RPC and IPC tick are removed from this variant.

## What this means

- No tick calls needed from your client process.
- No `manualTick` RPC and no Unix-socket IPC tick server in multivm-validator.
- Just submit transactions over RPC; PoH ticking happens inside the validator.

## How to run

```bash
cargo run -p agave-validator --bin multivm-validator -- \
  --ledger /tmp/ledger \
  --rpc-port 8899 \
  --faucet-port 9900
```

## Migrating your client code

- Remove any calls to:
  - `Admin RPC manualTick`
  - `bridge::ipc::IpcClient::tick()`
- Keep using your existing `RpcClient` to send transactions and poll confirmations.

If you used helper utilities that performed ticks between polls, you can switch to a standard confirmation loop (polling `get_signature_status_with_commitment`) without tick calls.

## Notes

- This change only affects the multivm-validator binary. Other binaries may still expose manual tick features.
- `--tick-ipc-path` and `--auto-tick` flags are no longer used by multivm-validator.
