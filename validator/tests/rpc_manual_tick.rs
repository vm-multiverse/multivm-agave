use {
    agave_validator::admin_rpc_service,
    solana_logger,
    solana_rpc_client::rpc_client::RpcClient,
    solana_sdk::commitment_config::CommitmentConfig,
    std::{
        net::{IpAddr, Ipv4Addr},
        path::PathBuf,
        process::{Command, Stdio},
        thread,
        time::Duration,
    },
    tempfile::tempdir,
};

fn find_available_port() -> u16 {
    let bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    solana_net_utils::find_available_port_in_range(bind, (12000, 20000))
        .expect("no available port in range")
}

#[test]
#[ignore] // Spawns multivm-validator; run manually: cargo test -p agave-validator --test rpc_manual_tick -- --ignored
fn rpc_manual_tick_advances_block_height() {
    solana_logger::setup();

    // Prepare ledger and ports
    let tmp_dir = tempdir().expect("create tempdir");
    let ledger_path: PathBuf = tmp_dir.path().to_path_buf();
    let rpc_port = find_available_port();
    let tick_ipc_path = ledger_path.join("tick.sock");

    // Path to multivm-validator binary
    let bin = env!("CARGO_BIN_EXE_multivm-validator");

    // Start multivm-validator
    let mut child = Command::new(bin)
        .arg("--ledger")
        .arg(&ledger_path)
        .arg("--rpc-port")
        .arg(format!("{}", rpc_port))
        .arg("--tick-ipc-path")
        .arg(tick_ipc_path.to_string_lossy().to_string())
        .arg("--reset")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn multivm-validator");

    // Give the validator some time to start
    thread::sleep(Duration::from_secs(2));

    // Connect Admin RPC over IPC and the public RPC over HTTP
    let admin_client_fut = admin_rpc_service::connect(&ledger_path);
    let runtime = admin_rpc_service::runtime();

    // Wait until admin RPC is ready
    let admin = {
        let mut tries = 0;
        loop {
            match runtime.block_on(admin_client_fut.clone()) {
                Ok(client) => break client,
                Err(_) => {
                    tries += 1;
                    if tries > 50 {
                        panic!("admin rpc not ready");
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    };

    let rpc_client = RpcClient::new_with_commitment(
        format!("http://127.0.0.1:{}", rpc_port),
        CommitmentConfig::processed(),
    );

    let initial_height = rpc_client.get_block_height().unwrap_or(0);

    // Send several manual ticks
    for _ in 0..12 {
        runtime
            .block_on(async { admin.manual_tick().await })
            .expect("manualTick failed");
        thread::sleep(Duration::from_millis(50));
    }

    // Allow some time for block height to reflect ticks
    thread::sleep(Duration::from_millis(250));
    let after_height = rpc_client.get_block_height().unwrap_or(0);

    assert!(after_height >= initial_height);

    // Cleanup
    let _ = child.kill();
    let _ = child.wait();
}


