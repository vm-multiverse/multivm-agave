use {
    crossbeam_channel::{Receiver, Sender},
    hyper::{service::{make_service_fn, service_fn}, Body, Request, Response, Server},
    log::{error, info},
    serde_json::Value,
    solana_rpc_client::rpc_client::RpcClient,
    solana_sdk::transaction::Transaction,
    std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration},
    tokio::runtime::Runtime,
};

#[derive(Clone)]
struct Ctx {
    tick_sender: Sender<()>,
    tick_done_receiver: Receiver<()>,
    rpc_client: Arc<RpcClient>,
    ticks_per_slot: u64,
}

async fn handle(req: Request<Body>, ctx: Ctx) -> Result<Response<Body>, Infallible> {
    let bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::Null);
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
    match method {
        "engine_tick" => {
            let _ = ctx.tick_sender.send(());
            let _ = ctx.tick_done_receiver.recv();
            let body = serde_json::json!({"jsonrpc":"2.0","result":"ok","id": id});
            Ok(Response::new(Body::from(body.to_string())))
        }
        "engine_step_slot" => {
            for _ in 0..ctx.ticks_per_slot { let _ = ctx.tick_sender.send(()); let _ = ctx.tick_done_receiver.recv(); }
            let body = serde_json::json!({"jsonrpc":"2.0","result":"ok","id": id});
            Ok(Response::new(Body::from(body.to_string())))
        }
        "engine_send_and_confirm_tx" => {
            let params = v.get("params").and_then(|p| p.as_array()).cloned().unwrap_or_default();
            let b64 = params.get(0).and_then(|x| x.as_str()).unwrap_or("");
            let raw = match base64::decode(b64) { Ok(r) => r, Err(e) => {
                let body = serde_json::json!({"jsonrpc":"2.0","error":{"code":-32602,"message":format!("invalid base64: {}", e)},"id": id});
                return Ok(Response::new(Body::from(body.to_string())));
            }};
            // Pre-tick
            let _ = ctx.tick_sender.send(()); let _ = ctx.tick_done_receiver.recv();
            let tx: Transaction = match bincode::deserialize(&raw) {
                Ok(t) => t,
                Err(e) => {
                    let body = serde_json::json!({"jsonrpc":"2.0","error":{"code":-32602,"message":format!("invalid tx encoding: {}", e)},"id": id});
                    return Ok(Response::new(Body::from(body.to_string())));
                }
            };
            let sig = match ctx.rpc_client.send_transaction(&tx) {
                Ok(s) => s,
                Err(e) => {
                    let body = serde_json::json!({"jsonrpc":"2.0","error":{"code":-32003,"message":format!("send failed: {}", e)},"id": id});
                    return Ok(Response::new(Body::from(body.to_string())));
                }
            };
            // Post-tick (mirror previous behavior: 3 ticks after send)
            for _ in 0..3 { let _ = ctx.tick_sender.send(()); let _ = ctx.tick_done_receiver.recv(); }
            let max_retries = 60u32;
            for _ in 0..max_retries {
                if let Ok(Some(Ok(_))) = ctx.rpc_client.get_signature_status(&sig) {
                    let body = serde_json::json!({"jsonrpc":"2.0","result": sig.to_string(),"id": id});
                    return Ok(Response::new(Body::from(body.to_string())));
                }
                let _ = ctx.tick_sender.send(()); let _ = ctx.tick_done_receiver.recv();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let body = serde_json::json!({"jsonrpc":"2.0","error":{"code":-32005,"message":"confirmation timeout"},"id": id});
            Ok(Response::new(Body::from(body.to_string())))
        }
        _ => {
            let body = serde_json::json!({"jsonrpc":"2.0","error":{"code":-32601,"message":"method not found"},"id": id});
            Ok(Response::new(Body::from(body.to_string())))
        }
    }
}

pub fn start_control_server(
    bind_addr: SocketAddr,
    tick_sender: Sender<()>,
    tick_done_receiver: Receiver<()>,
    rpc_port: u16,
    ticks_per_slot: u64,
) {
    std::thread::spawn(move || {
        let rt = Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            let rpc_client = Arc::new(RpcClient::new(format!("http://127.0.0.1:{}", rpc_port)));
            let ctx = Ctx { tick_sender, tick_done_receiver, rpc_client, ticks_per_slot };
            let make_svc = make_service_fn(move |_conn| {
                let ctx = ctx.clone();
                async move { Ok::<_, Infallible>(service_fn(move |req| handle(req, ctx.clone()))) }
            });
            info!("Engine control RPC listening on http://{}", bind_addr);
            let server = Server::bind(&bind_addr).serve(make_svc);
            if let Err(e) = server.await { error!("control server error: {}", e); }
        });
    });
}
