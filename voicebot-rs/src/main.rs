//! voicebot-rs — rsaga-trpg-voicebot 的 Rust(axum) 改写（bin 入口）。

use voicebot_rs::{build_app, config::{cfg, log_line}, make_router};

#[tokio::main]
async fn main() {
    let c = cfg();
    let listen_host = c.vb_host.clone();
    let port = c.vb_port;
    log_line(&format!("voicebot-rs base_dir={} port={}", c.base_dir.display(), port));

    let app = build_app().await;
    let router = make_router(app);

    let addr = format!("{listen_host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind failed");
    log_line(&format!("voicebot-rs listening on {addr}"));
    axum::serve(listener, router)
        .await
        .expect("server error");
}
