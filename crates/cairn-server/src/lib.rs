//! Cairn sync server.
//!
//! A small HTTP service that stores encrypted file revisions per vault and
//! offers a changes feed and compare-and-swap uploads. It cannot read notes:
//! paths and contents are encrypted on the devices.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequest, FromRequestParts, Path as UrlPath, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::serve::Listener;
use axum::{Json, Router};
use cairn_sync::protocol::*;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tower_http::add_extension::AddExtension;
use tower_http::timeout::RequestBodyTimeoutLayer;

/// How long a client may take to send the headers of a request. hyper starts
/// this clock when a connection opens and again after each response, so it is
/// also how long an idle connection stays open.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a request body may stall (no new bytes) before the request fails.
/// A slow upload that keeps sending is not cut off.
pub const BODY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Config {
    /// Accepted bearer tokens.
    pub tokens: Vec<String>,
    /// Maximum request body in bytes.
    pub max_body: usize,
}

pub struct AppState {
    db: Mutex<Connection>,
    config: Config,
}

pub type Shared = Arc<AppState>;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS vaults (
    id TEXT PRIMARY KEY,
    created INTEGER NOT NULL,
    keys TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS revisions (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    vault TEXT NOT NULL REFERENCES vaults(id),
    file_id TEXT NOT NULL,
    parent_seq INTEGER,
    device TEXT NOT NULL,
    deleted INTEGER NOT NULL,
    size INTEGER NOT NULL,
    created INTEGER NOT NULL,
    blob BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS revisions_file ON revisions(vault, file_id, seq);
CREATE TABLE IF NOT EXISTS heads (
    vault TEXT NOT NULL,
    file_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    PRIMARY KEY (vault, file_id)
);
CREATE INDEX IF NOT EXISTS heads_seq ON heads(vault, seq);
";

pub fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn memory_db() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn state(conn: Connection, config: Config) -> Shared {
    Arc::new(AppState { db: Mutex::new(conn), config })
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        log::error!("database error: {e}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
    }
}

type ApiResult<T> = Result<T, ApiError>;

// axum's Json, Query and Path extractors answer bad input (malformed JSON,
// a missing field, a bad query or path value, no JSON content type) with a
// text/plain body. These wrappers keep axum's status and message but answer
// in the server's JSON error format.

struct ApiJson<T>(T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> ApiResult<Self> {
        let Json(v) = Json::from_request(req, state).await.map_err(|e| ApiError(e.status(), e.body_text()))?;
        Ok(ApiJson(v))
    }
}

struct ApiQuery<T>(T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> ApiResult<Self> {
        let Query(v) = Query::from_request_parts(parts, state).await.map_err(|e| ApiError(e.status(), e.body_text()))?;
        Ok(ApiQuery(v))
    }
}

struct ApiPath<T>(T);

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> ApiResult<Self> {
        let UrlPath(v) = UrlPath::from_request_parts(parts, state).await.map_err(|e| ApiError(e.status(), e.body_text()))?;
        Ok(ApiPath(v))
    }
}

fn check_id(id: &str) -> ApiResult<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::BAD_REQUEST, format!("invalid id: {id:?}")))
    }
}

async fn auth(State(st): State<Shared>, headers: HeaderMap, req: Request, next: Next) -> Response {
    // Auth schemes are case-insensitive (RFC 9110 section 11.1).
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map_or("", |(_, token)| token);
    let ok = st.config.tokens.iter().any(|t| {
        use subtle::ConstantTimeEq;
        t.len() == presented.len() && bool::from(t.as_bytes().ct_eq(presented.as_bytes()))
    });
    if !ok {
        // Never log what was presented: it may be a real token with a typo.
        let from = req.extensions().get::<ConnectInfo<SocketAddr>>().map_or("unknown address".into(), |c| c.0.to_string());
        let why = if headers.contains_key("authorization") { "wrong token" } else { "no token" };
        log::warn!("refused request from {from}: {why}");
        return ApiError(StatusCode::UNAUTHORIZED, "missing or wrong token".into()).into_response();
    }
    next.run(req).await
}

async fn health() -> &'static str {
    "ok"
}

async fn create_vault(State(st): State<Shared>, ApiPath(vault): ApiPath<String>, ApiJson(keys): ApiJson<KeyEnvelope>) -> ApiResult<StatusCode> {
    check_id(&vault)?;
    let db = st.db.lock();
    let n = db.execute(
        "INSERT OR IGNORE INTO vaults (id, created, keys) VALUES (?1, ?2, ?3)",
        params![vault, now(), serde_json::to_string(&keys).unwrap()],
    )?;
    if n == 0 {
        return Err(ApiError(StatusCode::CONFLICT, "vault exists".into()));
    }
    log::info!("created vault {vault}");
    Ok(StatusCode::CREATED)
}

fn load_vault(db: &Connection, vault: &str) -> ApiResult<VaultInfo> {
    let keys: Option<String> = db.query_row("SELECT keys FROM vaults WHERE id = ?1", [vault], |r| r.get(0)).optional()?;
    let Some(keys) = keys else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such vault".into()));
    };
    let head_seq: Option<i64> = db.query_row("SELECT MAX(seq) FROM revisions WHERE vault = ?1", [vault], |r| r.get(0))?;
    Ok(VaultInfo {
        vault: vault.to_string(),
        keys: serde_json::from_str(&keys).map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "bad key data".into()))?,
        head_seq: head_seq.unwrap_or(0) as u64,
    })
}

async fn get_vault(State(st): State<Shared>, ApiPath(vault): ApiPath<String>) -> ApiResult<Json<VaultInfo>> {
    check_id(&vault)?;
    Ok(Json(load_vault(&st.db.lock(), &vault)?))
}

#[derive(Deserialize)]
struct ChangesQuery {
    since: Option<u64>,
    limit: Option<u32>,
}

async fn changes(State(st): State<Shared>, ApiPath(vault): ApiPath<String>, ApiQuery(q): ApiQuery<ChangesQuery>) -> ApiResult<Json<ChangesResponse>> {
    check_id(&vault)?;
    let db = st.db.lock();
    load_vault(&db, &vault)?;
    let since = q.since.unwrap_or(0);
    // Seqs are SQLite integers, so none is above i64::MAX; a larger cursor
    // (which `as i64` would wrap negative) has nothing after it.
    let since_sql = i64::try_from(since).unwrap_or(i64::MAX);
    let limit = q.limit.unwrap_or(500).clamp(1, 2000) as i64;
    // Only the current head of each file changed since `since` is needed.
    let mut stmt = db.prepare(
        "SELECT r.file_id, r.seq, r.parent_seq, r.device, r.deleted, r.blob
         FROM heads h JOIN revisions r ON r.seq = h.seq
         WHERE h.vault = ?1 AND h.seq > ?2 ORDER BY h.seq LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![vault, since_sql, limit + 1], |r| {
        Ok(RemoteHead {
            file_id: r.get(0)?,
            seq: r.get::<_, i64>(1)? as u64,
            parent_seq: r.get::<_, Option<i64>>(2)?.map(|x| x as u64),
            device: r.get(3)?,
            deleted: r.get::<_, i64>(4)? != 0,
            blob: b64(&r.get::<_, Vec<u8>>(5)?),
        })
    })?;
    let mut heads = rows.collect::<Result<Vec<_>, _>>()?;
    let more = heads.len() as i64 > limit;
    heads.truncate(limit as usize);
    let cursor = heads.last().map(|h| h.seq).unwrap_or(since);
    Ok(Json(ChangesResponse { heads, cursor, more }))
}

async fn put_revision(
    State(st): State<Shared>,
    ApiPath((vault, file_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<PutRevision>,
) -> ApiResult<Response> {
    check_id(&vault)?;
    check_id(&file_id)?;
    let blob = unb64(&body.blob).ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "blob is not base64".into()))?;
    let mut db = st.db.lock();
    load_vault(&db, &vault)?;
    let tx = db.transaction()?;
    let current: Option<i64> = tx
        .query_row("SELECT seq FROM heads WHERE vault = ?1 AND file_id = ?2", params![vault, file_id], |r| r.get(0))
        .optional()?;
    if current.map(|c| c as u64) != body.parent_seq {
        let c = Conflict { current_seq: current.map(|c| c as u64) };
        return Ok((StatusCode::CONFLICT, Json(c)).into_response());
    }
    tx.execute(
        "INSERT INTO revisions (vault, file_id, parent_seq, device, deleted, size, created, blob)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            vault,
            file_id,
            body.parent_seq.map(|p| p as i64),
            body.device,
            body.deleted as i64,
            blob.len() as i64,
            now(),
            blob
        ],
    )?;
    let seq = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO heads (vault, file_id, seq) VALUES (?1, ?2, ?3)
         ON CONFLICT(vault, file_id) DO UPDATE SET seq = excluded.seq",
        params![vault, file_id, seq],
    )?;
    tx.commit()?;
    Ok((StatusCode::OK, Json(PutResult { seq: seq as u64 })).into_response())
}

async fn history(State(st): State<Shared>, ApiPath((vault, file_id)): ApiPath<(String, String)>) -> ApiResult<Json<Vec<HistoryEntry>>> {
    check_id(&vault)?;
    check_id(&file_id)?;
    let db = st.db.lock();
    load_vault(&db, &vault)?;
    let mut stmt = db.prepare(
        "SELECT seq, created, device, deleted, size FROM revisions WHERE vault = ?1 AND file_id = ?2 ORDER BY seq DESC",
    )?;
    let rows = stmt.query_map(params![vault, file_id], |r| {
        Ok(HistoryEntry {
            seq: r.get::<_, i64>(0)? as u64,
            created: r.get(1)?,
            device: r.get(2)?,
            deleted: r.get::<_, i64>(3)? != 0,
            size: r.get::<_, i64>(4)? as u64,
        })
    })?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

async fn revision(State(st): State<Shared>, ApiPath((vault, seq)): ApiPath<(String, u64)>) -> ApiResult<Json<RevisionBlob>> {
    check_id(&vault)?;
    let db = st.db.lock();
    let row = db
        .query_row(
            "SELECT file_id, deleted, blob FROM revisions WHERE vault = ?1 AND seq = ?2",
            params![vault, seq as i64],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?)),
        )
        .optional()?;
    let Some((file_id, deleted, blob)) = row else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such revision".into()));
    };
    Ok(Json(RevisionBlob { seq, file_id, deleted: deleted != 0, blob: b64(&blob) }))
}

/// Takes no body extractor on purpose: the fallback runs without
/// authentication, so it must answer without reading what the client sends.
async fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "not found".into())
}

pub fn router(st: Shared) -> Router {
    let max_body = st.config.max_body;
    let api = Router::new()
        .route("/vaults/{vault}", put(create_vault).get(get_vault))
        .route("/vaults/{vault}/changes", get(changes))
        .route("/vaults/{vault}/files/{file_id}", post(put_revision))
        .route("/vaults/{vault}/files/{file_id}/history", get(history))
        .route("/vaults/{vault}/revisions/{seq}", get(revision))
        .layer(middleware::from_fn_with_state(st.clone(), auth));
    Router::new()
        .route("/health", get(health))
        .nest(API_PREFIX, api)
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(max_body))
        .layer(RequestBodyTimeoutLayer::new(BODY_TIMEOUT))
        .with_state(st)
}

/// Serve forever (used by tests).
pub async fn serve(listener: tokio::net::TcpListener, st: Shared) {
    serve_until(listener, st, std::future::pending()).await
}

/// Serve until `shutdown` resolves, then let open requests finish.
///
/// This uses hyper directly because `axum::serve` sets no timer, so hyper's
/// header read timeout never fires there: a client that sends half a request,
/// or nothing, would hold its connection (and a file descriptor) forever.
pub async fn serve_until(mut listener: tokio::net::TcpListener, st: Shared, shutdown: impl Future<Output = ()>) {
    let app = router(st);
    let mut http = hyper::server::conn::http1::Builder::new();
    http.timer(TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
    let graceful = GracefulShutdown::new();
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        // axum's accept logs errors such as "too many open files" and retries
        // after a second.
        let (io, addr) = tokio::select! {
            conn = Listener::accept(&mut listener) => conn,
            _ = &mut shutdown => break,
        };
        // The client address, for handlers and the auth log (axum::serve
        // would add this too).
        let svc = AddExtension::new(app.clone(), ConnectInfo(addr));
        let conn = graceful.watch(http.serve_connection(TokioIo::new(io), TowerToHyperService::new(svc)));
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                log::debug!("connection closed: {e}");
            }
        });
    }
    graceful.shutdown().await;
}
