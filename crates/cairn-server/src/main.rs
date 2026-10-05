//! cairn-server: configure with environment variables.
//!
//!   CAIRN_TOKENS   comma-separated bearer tokens (required)
//!   CAIRN_DATA     data folder for the SQLite database (default ./data)
//!   CAIRN_ADDR     listen address (default 0.0.0.0:8787)
//!   CAIRN_MAX_BODY_MB  largest upload in MB (default 200; Cairn clients
//!                      send and read at most 200, files up to about 150)

use std::path::PathBuf;

fn main() {
    // `cairn-server --health` is the container health check.
    if std::env::args().nth(1).as_deref() == Some("--health") {
        let addr = std::env::var("CAIRN_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".into());
        let port = addr.rsplit(':').next().unwrap_or("8787");
        let ok = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let tokens: Vec<String> = std::env::var("CAIRN_TOKENS")
        .unwrap_or_default()
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        eprintln!("CAIRN_TOKENS is not set. Set it to one or more secret tokens, e.g. CAIRN_TOKENS=$(openssl rand -hex 24)");
        std::process::exit(2);
    }
    if tokens.iter().any(|t| t.len() < 16) {
        eprintln!("warning: tokens shorter than 16 characters are easy to guess");
    }
    let data = PathBuf::from(std::env::var("CAIRN_DATA").unwrap_or_else(|_| "data".into()));
    std::fs::create_dir_all(&data).expect("cannot create data folder");
    let db_path = data.join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).expect("cannot open database");
    let max_body = std::env::var("CAIRN_MAX_BODY_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(200usize) * 1024 * 1024;
    let addr = std::env::var("CAIRN_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let st = cairn_server::state(conn, cairn_server::Config { tokens, max_body });
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&addr).await.expect("cannot bind");
        log::info!("cairn-server listening on {addr}, database {}", db_path.display());
        cairn_server::serve_until(listener, st, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    });
}
