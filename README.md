# Multivm-Agave

该仓库由 Agave v2.2.15 fork 而来。

Multivm 入口在 `validator/src/bin/multivm-tpu-client.rs` 和 `validator/src/bin/multivm-validator.rs`。

目前代码中，tick 由线程主动控制，1s 一个 tick，2s 一个块，块内最多一个交易：

```rust
// Tick manually now
let (tick_sender, tick_receiver) = unbounded();
std::thread::spawn({
    let tick_sender = tick_sender.clone();
    move || loop {
        if tick_sender.send(()).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
});
```

未来需要修改可能是：

- 修改 PoH 的 tick 触发条件，一个投票交易一个 tick，一个普通交易一个 tick，而不再由时间控制。
- 禁用 P2P。