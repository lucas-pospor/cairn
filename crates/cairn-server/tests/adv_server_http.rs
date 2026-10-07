//! Adversarial raw-HTTP tests of cairn-server.
//!
//! The other tests drive the server only through the typed client. These talk
//! to a real server with raw `std::net::TcpStream` so they can send malformed
//! requests: broken JSON, wrong content types, bad query values, odd vault
//! names, lying Content-Length, oversized chunked bodies, long URLs/headers and
//! Authorization variants. After each probe the server must still answer a
//! normal request, and the SQLite file must pass `PRAGMA integrity_check`.
//!
//! Volumes are deliberately small (this is robustness testing, not a DoS).
//!
//! Run with:
//!   CARGO_BUILD_JOBS=2 cargo test -p cairn-server --test adv_server_http
//! and add `-- --nocapture` to see the measurements the tests print.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "tok_ht_aaaaaaaaaaaaaaaaaaaaaa";

struct Server {
    addr: SocketAddr,
    db_path: PathBuf,
    _dir: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
}

fn server_with(max_body: usize) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cairn.sqlite");
    let conn = cairn_server::open_db(&db_path).unwrap();
    let st = cairn_server::state(conn, cairn_server::Config { tokens: vec![TOKEN.into()], max_body });
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    rt.spawn(async move { cairn_server::serve(listener, st).await });
    Server { addr, db_path, _dir: dir, _rt: rt }
}

fn server() -> Server {
    server_with(1 << 20)
}

/// Parsed (or partial) HTTP response.
#[derive(Debug)]
struct Resp {
    status: Option<u16>,
    head: String,
    body: Vec<u8>,
    /// true if we gave up waiting (no complete response within the timeout).
    timed_out: bool,
}

impl Resp {
    fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn header(&self, name: &str) -> Option<String> {
        let n = name.to_ascii_lowercase();
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().to_ascii_lowercase() == n).then(|| v.trim().to_string())
        })
    }
}

fn find(h: &[u8], pat: &[u8]) -> Option<usize> {
    h.windows(pat.len()).position(|w| w == pat)
}

/// Read one response from `s`, waiting at most `timeout` in total.
fn read_resp(s: &mut TcpStream, timeout: Duration) -> Resp {
    let start = Instant::now();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        let left = timeout.checked_sub(start.elapsed()).unwrap_or(Duration::ZERO);
        if left.is_zero() {
            return parse(buf, true);
        }
        s.set_read_timeout(Some(left.max(Duration::from_millis(10)))).unwrap();
        match s.read(&mut chunk) {
            Ok(0) => return parse(buf, false),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = find(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                    let cl = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)));
                    if let Some(cl) = cl
                        && buf.len() >= end + 4 + cl
                    {
                        return parse(buf, false);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                return parse(buf, true)
            }
            Err(_) => return parse(buf, false), // reset by peer
        }
    }
}

fn parse(buf: Vec<u8>, timed_out: bool) -> Resp {
    let (head, body) = match find(&buf, b"\r\n\r\n") {
        Some(end) => (String::from_utf8_lossy(&buf[..end]).into_owned(), buf[end + 4..].to_vec()),
        None => (String::from_utf8_lossy(&buf).into_owned(), Vec::new()),
    };
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).filter(|_| head.starts_with("HTTP/"));
    Resp { status, head, body, timed_out }
}

/// Send raw bytes on a fresh connection and read one response.
fn raw(addr: SocketAddr, bytes: &[u8]) -> Resp {
    let mut s = TcpStream::connect(addr).unwrap();
    let _ = s.write_all(bytes);
    read_resp(&mut s, Duration::from_secs(5))
}

fn req(method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut v = out.into_bytes();
    v.extend_from_slice(body);
    v
}

fn auth() -> String {
    format!("Bearer {TOKEN}")
}

fn get(addr: SocketAddr, path: &str) -> Resp {
    let a = auth();
    raw(addr, &req("GET", path, &[("Authorization", &a)], b""))
}

fn post_json(addr: SocketAddr, method: &str, path: &str, body: &str) -> Resp {
    let a = auth();
    raw(addr, &req(method, path, &[("Authorization", &a), ("Content-Type", "application/json")], body.as_bytes()))
}

const KEYS: &str = r#"{"salt":"AAAA","kdf":{"m_cost_kib":8,"t_cost":1,"p_cost":1},"wrapped_key":"AAAA"}"#;

fn create_vault(addr: SocketAddr, vault: &str) {
    let r = post_json(addr, "PUT", &format!("/v1/vaults/{vault}"), KEYS);
    assert_eq!(r.status, Some(201), "{r:?}");
}

fn put_rev(addr: SocketAddr, vault: &str, fid: &str, parent: Option<u64>, blob: &str) -> Resp {
    let parent = parent.map(|p| p.to_string()).unwrap_or_else(|| "null".into());
    post_json(
        addr,
        "POST",
        &format!("/v1/vaults/{vault}/files/{fid}"),
        &format!(r#"{{"parent_seq":{parent},"device":"dev","deleted":false,"blob":"{blob}"}}"#),
    )
}

fn integrity(db: &PathBuf) -> (String, i64, i64) {
    let c = rusqlite::Connection::open(db).unwrap();
    let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    let revs: i64 = c.query_row("SELECT COUNT(*) FROM revisions", [], |r| r.get(0)).unwrap();
    let vaults: i64 = c.query_row("SELECT COUNT(*) FROM vaults", [], |r| r.get(0)).unwrap();
    (ok, revs, vaults)
}

/// The server still answers normal requests and the DB is intact and unchanged.
fn assert_healthy(srv: &Server, revs: i64, vaults: i64) {
    let r = raw(srv.addr, b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert_eq!(r.status, Some(200), "health after probe: {r:?}");
    let r = get(srv.addr, "/v1/vaults/v1");
    assert_eq!(r.status, Some(200), "authenticated GET after probe: {r:?}");
    let (ok, n, v) = integrity(&srv.db_path);
    assert_eq!(ok, "ok");
    assert_eq!((n, v), (revs, vaults), "database content changed by a rejected request");
}

fn is_4xx(r: &Resp) -> bool {
    matches!(r.status, Some(400..=499))
}

/// A base fixture: vault v1 with one revision (seq 1) of file f1.
fn fixture() -> Server {
    let srv = server();
    create_vault(srv.addr, "v1");
    let r = put_rev(srv.addr, "v1", "f1", None, "QUJD");
    assert_eq!(r.status, Some(200), "{r:?}");
    srv
}

// ===================================================================
// Handled: malformed JSON bodies and wrong content types
// ===================================================================

#[test]
fn malformed_json_bodies_get_4xx_and_change_nothing() {
    let srv = fixture();
    let deep_array = "[".repeat(100_000);
    let deep_obj = format!("{}1{}", r#"{"a":"#.repeat(20_000), "}".repeat(20_000));
    let cases: Vec<(&str, String, &str)> = vec![
        ("truncated", r#"{"parent_seq":1,"device":"d","#.into(), "/v1/vaults/v1/files/f1"),
        ("not json", "hello".into(), "/v1/vaults/v1/files/f1"),
        ("empty", "".into(), "/v1/vaults/v1/files/f1"),
        ("null", "null".into(), "/v1/vaults/v1/files/f1"),
        ("array", "[]".into(), "/v1/vaults/v1/files/f1"),
        ("missing blob", r#"{"parent_seq":1,"device":"d","deleted":false}"#.into(), "/v1/vaults/v1/files/f1"),
        ("deleted is string", r#"{"parent_seq":1,"device":"d","deleted":"yes","blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("parent_seq negative", r#"{"parent_seq":-1,"device":"d","deleted":false,"blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("parent_seq float", r#"{"parent_seq":1.5,"device":"d","deleted":false,"blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("parent_seq huge", r#"{"parent_seq":1e300,"device":"d","deleted":false,"blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("device number", r#"{"parent_seq":1,"device":7,"deleted":false,"blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("blob not base64", r#"{"parent_seq":1,"device":"d","deleted":false,"blob":"not base64!!"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("blob url-safe alphabet", r#"{"parent_seq":1,"device":"d","deleted":false,"blob":"-_-_"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("blob bad padding", r#"{"parent_seq":1,"device":"d","deleted":false,"blob":"QUJ"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("blob is number", r#"{"parent_seq":1,"device":"d","deleted":false,"blob":12}"#.into(), "/v1/vaults/v1/files/f1"),
        ("duplicate field", r#"{"parent_seq":1,"parent_seq":1,"device":"d","deleted":false,"blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("BOM", "\u{feff}{\"parent_seq\":1,\"device\":\"d\",\"deleted\":false,\"blob\":\"QUJD\"}".into(), "/v1/vaults/v1/files/f1"),
        ("lone surrogate", r#"{"parent_seq":1,"device":"\ud800","deleted":false,"blob":"QUJD"}"#.into(), "/v1/vaults/v1/files/f1"),
        ("deeply nested array", deep_array, "/v1/vaults/v1/files/f1"),
        ("deeply nested object", deep_obj, "/v1/vaults/v1/files/f1"),
        ("keys missing kdf", r#"{"salt":"AAAA","wrapped_key":"AAAA"}"#.into(), "/v1/vaults/v2"),
        ("keys kdf negative", r#"{"salt":"AAAA","kdf":{"m_cost_kib":-1,"t_cost":1,"p_cost":1},"wrapped_key":"AAAA"}"#.into(), "/v1/vaults/v2"),
        ("keys kdf overflow", r#"{"salt":"AAAA","kdf":{"m_cost_kib":4294967296,"t_cost":1,"p_cost":1},"wrapped_key":"AAAA"}"#.into(), "/v1/vaults/v2"),
    ];
    for (name, body, path) in cases {
        let method = if path.contains("/files/") { "POST" } else { "PUT" };
        let r = post_json(srv.addr, method, path, &body);
        assert!(is_4xx(&r), "{name}: expected 4xx, got {r:?}");
        assert_healthy(&srv, 1, 1);
    }
}

#[test]
fn wrong_content_types_get_415() {
    let srv = fixture();
    let a = auth();
    let body = br#"{"parent_seq":1,"device":"d","deleted":false,"blob":"QUJD"}"#;
    for ct in [None, Some("text/plain"), Some("application/x-www-form-urlencoded"), Some("multipart/form-data; boundary=x"), Some("application/jsonx")] {
        let mut h = vec![("Authorization", a.as_str())];
        if let Some(ct) = ct {
            h.push(("Content-Type", ct));
        }
        let r = raw(srv.addr, &req("POST", "/v1/vaults/v1/files/f1", &h, body));
        assert_eq!(r.status, Some(415), "content type {ct:?}: {r:?}");
        assert_healthy(&srv, 1, 1);
    }
    // Variants of application/json that are legitimately accepted.
    let r = raw(srv.addr, &req("POST", "/v1/vaults/v1/files/f1", &[("Authorization", &a), ("Content-Type", "application/json; charset=utf-8")], body));
    assert_eq!(r.status, Some(200), "{r:?}");
    assert_healthy(&srv, 2, 1);
}

#[test]
fn extra_unknown_fields_are_ignored_not_stored() {
    let srv = fixture();
    let r = post_json(srv.addr, "POST", "/v1/vaults/v1/files/f1", r#"{"parent_seq":1,"device":"d","deleted":false,"blob":"QUJD","seq":999,"vault":"other","x":{"y":[1,2]}}"#);
    assert_eq!(r.status, Some(200), "{r:?}");
    assert_eq!(r.body_str(), r#"{"seq":2}"#);
    assert_healthy(&srv, 2, 1);
}

// ===================================================================
// Handled: query parameters
// ===================================================================

#[test]
fn bad_query_params_are_rejected_or_clamped() {
    let srv = fixture();
    for q in ["since=-1", "since=1e30", "since=abc", "since=", "limit=4294967296", "limit=-5", "limit=1.5", "since=18446744073709551616"] {
        let r = get(srv.addr, &format!("/v1/vaults/v1/changes?{q}"));
        assert_eq!(r.status, Some(400), "{q}: {r:?}");
        assert_healthy(&srv, 1, 1);
    }
    // limit=0 is clamped to 1; huge legal u32 is clamped to 2000; unknown params ignored.
    for q in ["limit=0", "limit=4294967295", "since=0&since=0", "foo=bar", "since=0&limit=1&limit=1", "since=%30"] {
        let r = get(srv.addr, &format!("/v1/vaults/v1/changes?{q}"));
        assert!(matches!(r.status, Some(200) | Some(400)), "{q}: {r:?}");
        assert_healthy(&srv, 1, 1);
    }
    let r = get(srv.addr, "/v1/vaults/v1/changes?limit=0");
    assert_eq!(r.status, Some(200));
    assert!(r.body_str().contains(r#""seq":1"#), "{}", r.body_str());
}

#[test]
fn bad_revision_numbers_get_4xx() {
    let srv = fixture();
    for seq in ["-1", "abc", "1.0", "18446744073709551616", "0x1", "%31%00"] {
        let r = get(srv.addr, &format!("/v1/vaults/v1/revisions/{seq}"));
        assert!(is_4xx(&r), "{seq}: {r:?}");
    }
    // u64::MAX wraps to -1 in SQL; must still be a clean 404.
    for seq in ["0", "18446744073709551615", "9223372036854775808"] {
        let r = get(srv.addr, &format!("/v1/vaults/v1/revisions/{seq}"));
        assert_eq!(r.status, Some(404), "{seq}: {r:?}");
    }
    assert_healthy(&srv, 1, 1);
}

// ===================================================================
// Handled: vault names and ids
// ===================================================================

#[test]
fn odd_vault_names_and_ids_are_refused() {
    let srv = fixture();
    let long65 = "a".repeat(65);
    let long_10k = "b".repeat(10_000);
    let names = [
        "a%2Fb", "a%2fb", "%2E%2E", "a%00b", "%00", "a%20b", "%C3%A9", "%E2%80%AE", "a.b", "a:b", "a%5Cb",
        "%FF", "%", "%zz", "a%25", long65.as_str(), long_10k.as_str(), "%EF%BC%8F",
    ];
    for n in names {
        let r = post_json(srv.addr, "PUT", &format!("/v1/vaults/{n}"), KEYS);
        assert!(is_4xx(&r), "create {:.40}: {r:?}", n);
        let r = get(srv.addr, &format!("/v1/vaults/{n}"));
        assert!(is_4xx(&r), "get {:.40}: {r:?}", n);
        let r = put_rev(srv.addr, "v1", n, None, "QUJD");
        assert!(is_4xx(&r), "file id {:.40}: {r:?}", n);
    }
    // Path forms that do not match a route.
    for p in ["/v1/vaults/", "/v1/vaults//changes", "/v1/vaults/../vaults/v1", "/v1//vaults/v1", "/v1/vaults/v1/files/", "/v1/vaults/v1/files/f1/history/x"] {
        let r = get(srv.addr, p);
        assert!(is_4xx(&r), "{p}: {r:?}");
    }
    // 64 is the maximum id length and works.
    let id64 = "c".repeat(64);
    let r = put_rev(srv.addr, "v1", &id64, None, "QUJD");
    assert_eq!(r.status, Some(200), "{r:?}");
    assert_healthy(&srv, 2, 1);
}

// ===================================================================
// Handled: HTTP framing
// ===================================================================

#[test]
fn content_length_shorter_than_body_does_not_corrupt() {
    let srv = fixture();
    let a = auth();
    let body = br#"{"parent_seq":1,"device":"d","deleted":false,"blob":"QUJD"}"#;
    let mut r = format!(
        "POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n"
    )
    .into_bytes();
    r.extend_from_slice(body);
    let resp = raw(srv.addr, &r);
    assert!(is_4xx(&resp), "{resp:?}");
    assert_healthy(&srv, 1, 1);
}

#[test]
fn conflicting_or_invalid_framing_headers_get_400() {
    let srv = fixture();
    let a = auth();
    let body = r#"{"parent_seq":1,"device":"d","deleted":false,"blob":"QUJD"}"#;
    let probes = [
        format!("POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nContent-Length: 3\r\n\r\n{body}", body.len()),
        format!("POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nContent-Length: -1\r\n\r\n{body}"),
        format!("POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nContent-Length: 99999999999999999999999\r\n\r\n{body}"),
        format!("POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nTransfer-Encoding: gzip, chunked, identity\r\n\r\n0\r\n\r\n"),
        format!("POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n{body}\r\n0\r\n\r\n"),
        "GARBAGE\r\n\r\n".to_string(),
        "GET /health HTTP/9.9\r\nHost: x\r\n\r\n".to_string(),
        "GET  /health  HTTP/1.1\r\nHost: x\r\n\r\n".to_string(),
        "GET /health HTTP/1.1\r\nHost: x\r\nBad Header Name: y\r\n\r\n".to_string(),
        "GET /health HTTP/1.1\r\nHost: x\r\nX: a\u{0}b\r\n\r\n".to_string(),
        "\u{0}\u{1}\u{2}\u{3}\r\n\r\n".to_string(),
    ];
    for p in &probes {
        let r = raw(srv.addr, p.as_bytes());
        assert!(!r.timed_out, "server hung on {:.80}", p);
        assert!(r.status.is_none() || is_4xx(&r) || r.status == Some(505), "{:.80}: {r:?}", p);
        assert_healthy(&srv, 1, 1);
    }
}

#[test]
fn chunked_body_over_limit_gets_413_and_valid_chunked_works() {
    let srv = server_with(4096);
    create_vault(srv.addr, "v1");
    let a = auth();
    // Valid chunked upload.
    let body = br#"{"parent_seq":null,"device":"d","deleted":false,"blob":"QUJD"}"#;
    let mut r = format!("POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n", body.len()).into_bytes();
    r.extend_from_slice(body);
    r.extend_from_slice(b"\r\n0\r\n\r\n");
    let resp = raw(srv.addr, &r);
    assert_eq!(resp.status, Some(200), "{resp:?}");
    // Chunked body bigger than max_body (16 chunks of 1 KiB, limit 4 KiB).
    let mut s = TcpStream::connect(srv.addr).unwrap();
    s.write_all(format!("POST /v1/vaults/v1/files/f2 HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").as_bytes()).unwrap();
    let blob = "A".repeat(16 * 1024);
    let payload = format!(r#"{{"parent_seq":null,"device":"d","deleted":false,"blob":"{blob}"}}"#);
    if cfg!(unix) {
        for c in payload.as_bytes().chunks(1024) {
            if s.write_all(format!("{:x}\r\n", c.len()).as_bytes()).is_err() {
                break;
            }
            let _ = s.write_all(c);
            let _ = s.write_all(b"\r\n");
        }
        let _ = s.write_all(b"0\r\n\r\n");
    } else {
        // The server answers once it has read more than the limit and
        // closes without reading the rest. On Windows the reset that this
        // close sends throws away the answer before it is read, so the body
        // stops at the first byte over the limit there, sent in one write:
        // the server reads all of it. A server that does not keep to the
        // limit waits for the rest and gives no answer.
        let mut body = Vec::new();
        for c in payload.as_bytes()[..4096].chunks(1024) {
            body.extend_from_slice(format!("{:x}\r\n", c.len()).as_bytes());
            body.extend_from_slice(c);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"1\r\n");
        body.push(payload.as_bytes()[4096]);
        s.write_all(&body).unwrap();
    }
    let resp = read_resp(&mut s, Duration::from_secs(5));
    assert_eq!(resp.status, Some(413), "{resp:?}");
    // Same with Content-Length over the limit (on Windows 1 byte over, for
    // the same reason).
    let over = if cfg!(unix) { payload.as_str() } else { &payload[..4097] };
    let resp = post_json(srv.addr, "POST", "/v1/vaults/v1/files/f3", over);
    assert_eq!(resp.status, Some(413), "{resp:?}");
    let (ok, n, _) = integrity(&srv.db_path);
    assert_eq!((ok.as_str(), n), ("ok", 1));
}

#[test]
fn http10_and_missing_host_are_handled() {
    let srv = fixture();
    let r = raw(srv.addr, b"GET /health HTTP/1.0\r\n\r\n");
    assert_eq!(r.status, Some(200), "{r:?}");
    assert_eq!(r.body_str(), "ok");
    let r = raw(srv.addr, format!("GET /v1/vaults/v1 HTTP/1.0\r\nAuthorization: Bearer {TOKEN}\r\n\r\n").as_bytes());
    assert_eq!(r.status, Some(200), "{r:?}");
    let r = raw(srv.addr, b"GET /health HTTP/1.1\r\nConnection: close\r\n\r\n");
    assert!(matches!(r.status, Some(200) | Some(400)), "{r:?}");
    // Absolute-form request target.
    let r = raw(srv.addr, b"GET http://evil.example/health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert!(matches!(r.status, Some(200) | Some(400) | Some(404)), "{r:?}");
    assert_healthy(&srv, 1, 1);
}

#[test]
fn very_long_url_and_headers_get_4xx_without_hanging() {
    let srv = fixture();
    let a = auth();
    for len in [8 * 1024, 100 * 1024, 1024 * 1024] {
        let path = format!("/v1/vaults/{}", "a".repeat(len));
        let r = raw(srv.addr, &req("GET", &path, &[("Authorization", &a)], b""));
        assert!(!r.timed_out, "hung on {len}-byte URL");
        assert!(r.status.is_none() || is_4xx(&r), "{len}-byte URL: {:?} {:.200}", r.status, r.head);
        let q = format!("/v1/vaults/v1/changes?since=0&x={}", "q".repeat(len));
        let r = raw(srv.addr, &req("GET", &q, &[("Authorization", &a)], b""));
        assert!(!r.timed_out, "hung on {len}-byte query");
        let big = "h".repeat(len);
        let r = raw(srv.addr, &req("GET", "/health", &[("X-Big", &big)], b""));
        assert!(!r.timed_out, "hung on {len}-byte header");
        assert!(r.status.is_none() || r.status == Some(200) || is_4xx(&r), "{len}-byte header: {:?}", r.status);
        assert_healthy(&srv, 1, 1);
    }
    // Many small headers.
    let mut h = String::new();
    for i in 0..500 {
        h.push_str(&format!("X-H{i}: v\r\n"));
    }
    let r = raw(srv.addr, format!("GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{h}\r\n").as_bytes());
    assert!(!r.timed_out);
    assert!(r.status.is_none() || r.status == Some(200) || is_4xx(&r), "{r:?}");
    assert_healthy(&srv, 1, 1);
}

#[test]
fn http2_prior_knowledge_preface_does_not_break_server() {
    let srv = fixture();
    let mut s = TcpStream::connect(srv.addr).unwrap();
    s.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00").unwrap();
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut b = [0u8; 64];
    let _ = s.read(&mut b);
    drop(s);
    assert_healthy(&srv, 1, 1);
}

// ===================================================================
// Handled: Authorization variants
// ===================================================================

#[test]
fn authorization_variants() {
    let srv = fixture();
    let refused = [
        String::new(),
        "Bearer".into(),
        "Bearer ".into(),
        format!("Bearer  {TOKEN}"),
        format!("Bearer\t{TOKEN}"),
        format!("Basic {TOKEN}"),
        TOKEN.to_string(),
        format!("Bearer {}", &TOKEN[..TOKEN.len() - 1]),
        format!("Bearer {TOKEN}x"),
        format!("Bearer {TOKEN},{TOKEN}"),
        format!("Bearer \u{e9}{TOKEN}"),
        format!("Bearer {}", "x".repeat(60_000)),
    ];
    for v in &refused {
        let r = raw(srv.addr, &req("GET", "/v1/vaults/v1", &[("Authorization", v)], b""));
        assert!(matches!(r.status, Some(401) | Some(400) | Some(431)), "{:.40}: {r:?}", v);
    }
    // No Authorization header at all, on every route.
    for (m, p) in [("GET", "/v1/vaults/v1"), ("PUT", "/v1/vaults/v9"), ("GET", "/v1/vaults/v1/changes"), ("POST", "/v1/vaults/v1/files/f1"), ("GET", "/v1/vaults/v1/files/f1/history"), ("GET", "/v1/vaults/v1/revisions/1")] {
        let r = raw(srv.addr, &req(m, p, &[("Content-Type", "application/json")], b"{}"));
        assert_eq!(r.status, Some(401), "{m} {p}: {r:?}");
    }
    // Trailing whitespace is optional whitespace in HTTP and is stripped: accepted.
    let r = raw(srv.addr, &req("GET", "/v1/vaults/v1", &[("Authorization", &format!("Bearer {TOKEN}  "))], b""));
    assert_eq!(r.status, Some(200), "{r:?}");
    // Two Authorization headers: the first one decides (documenting behaviour).
    let good = auth();
    let r = raw(srv.addr, &req("GET", "/v1/vaults/v1", &[("Authorization", "Bearer wrong"), ("Authorization", &good)], b""));
    assert_eq!(r.status, Some(401), "{r:?}");
    let r = raw(srv.addr, &req("GET", "/v1/vaults/v1", &[("Authorization", &good), ("Authorization", "Bearer wrong")], b""));
    assert_eq!(r.status, Some(200), "{r:?}");
    // Header name is case-insensitive.
    let r = raw(srv.addr, &req("GET", "/v1/vaults/v1", &[("aUtHoRiZaTiOn", &good)], b""));
    assert_eq!(r.status, Some(200), "{r:?}");
    assert_healthy(&srv, 1, 1);
}

/// A 401 is sent without reading the request body (auth runs before the
/// body extractor), so an unauthenticated upload cannot make the server
/// buffer data on a real API route.
#[test]
fn unauthenticated_upload_to_api_route_is_refused_before_body() {
    let srv = server_with(200 << 20);
    let mut s = TcpStream::connect(srv.addr).unwrap();
    s.write_all(b"POST /v1/vaults/v1/files/f1 HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 150000000\r\n\r\n").unwrap();
    let r = read_resp(&mut s, Duration::from_secs(3));
    assert_eq!(r.status, Some(401), "{r:?}");
}

// ===================================================================
// FINDING-025: the 404 fallback answers at once, without reading the
// request body (it used to buffer it, unauthenticated, up to
// CAIRN_MAX_BODY_MB, default 200 MB).
// ===================================================================

/// Deterministic form: an unauthenticated request with a huge Content-Length
/// to an unknown path must be answered at once, like an API route answers
/// 401 at once, instead of after the whole body has been sent (buffered).
#[test]
fn unknown_path_is_answered_without_reading_body() {
    let srv = server_with(200 << 20);
    for path in ["/nope", "/v1/nope", "/health/x"] {
        let mut s = TcpStream::connect(srv.addr).unwrap();
        s.write_all(format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/octet-stream\r\nContent-Length: 150000000\r\n\r\n").as_bytes()).unwrap();
        // A server that answers without reading the body closes the
        // connection, so this write can fail with a broken pipe. On Windows
        // the reset that this close sends throws away the answer before it
        // is read, so no body is sent there: a server that waits for the
        // body still gives no answer.
        if cfg!(unix) {
            let _ = s.write_all(&vec![0u8; 1 << 20]);
        }
        let r = read_resp(&mut s, Duration::from_secs(3));
        assert_eq!(r.status, Some(404), "{path}: no answer after 3 s, the server is waiting to buffer the 150 MB body: {r:?}");
    }
}

#[cfg(target_os = "linux")] // Only Linux has /proc.
fn vm_hwm_kib(pid: u32) -> u64 {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    s.lines().find(|l| l.starts_with("VmHWM:")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap()
}

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn spawn_bin(dir: &std::path::Path, port: u16, extra_env: &[(&str, &str)]) -> Proc {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cairn-server"));
    c.env("CAIRN_TOKENS", TOKEN)
        .env("CAIRN_DATA", dir.join("data"))
        .env("CAIRN_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(dir.join("server.log")).unwrap());
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let p = Proc(c.spawn().unwrap());
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return p;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start");
}

/// Memory form, against the real binary with default settings: if the
/// fallback buffered the body, one unauthenticated 100 MB POST to /nope would
/// raise the server's peak RSS by about 100 MB, and N parallel requests would
/// cost N x 200 MB with the default limit.
#[cfg(target_os = "linux")] // The peak memory comes from /proc, which only Linux has.
#[test]
fn unauthenticated_post_to_unknown_path_does_not_grow_memory() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let p = spawn_bin(dir.path(), port, &[]);
    let pid = p.0.id();
    let before = vm_hwm_kib(pid);
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let total = 100usize << 20;
    s.write_all(format!("POST /nope HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {total}\r\n\r\n").as_bytes()).unwrap();
    let chunk = vec![b'z'; 1 << 20];
    let mut sent = 0;
    while sent < total {
        if s.write_all(&chunk).is_err() {
            break;
        }
        sent += chunk.len();
    }
    let r = read_resp(&mut s, Duration::from_secs(10));
    let after = vm_hwm_kib(pid);
    eprintln!("sent {} MiB unauthenticated; response {:?}; server VmHWM {} KiB -> {} KiB", sent >> 20, r.status, before, after);
    assert!(after < before + 20 * 1024, "peak RSS grew by {} MiB for one unauthenticated request", (after - before) / 1024);
}

// ===================================================================
// FINDING-076: header-read / body-read / idle timeouts. A connection that
// sends a partial request or goes quiet is closed after a timeout. Without
// them it was held open forever, keeping a file descriptor, and enough of
// them stopped the server accepting connections.
// ===================================================================

#[test]
fn partial_request_is_closed_after_a_timeout() {
    let srv = server();
    let mut half_header = TcpStream::connect(srv.addr).unwrap();
    half_header.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n").unwrap();
    let mut half_body = TcpStream::connect(srv.addr).unwrap();
    // Unauthenticated: the fallback answers without reading the declared
    // body (FINDING-025).
    half_body.write_all(b"POST /nope HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\n\r\n0123456789").unwrap();
    // 45 s is more generous than any common default (hyper/nginx/Go use 30-60 s
    // for headers; most use well under that).
    let r1 = read_resp(&mut half_header, Duration::from_secs(45));
    let r2 = read_resp(&mut half_body, Duration::from_millis(100));
    assert!(!r1.timed_out, "connection with a half-sent header still open after 45 s");
    assert!(!r2.timed_out, "connection with a half-sent body still open after 45 s");
}

/// Without timeouts, with a low file-descriptor limit (ulimit -n 40), 60
/// half-sent requests stop the server from accepting anything; it logs
/// 'accept error: Too many open files' once a second until the attacker
/// disconnects. Real servers have higher limits (1024 is the common
/// systemd/shell soft limit), so the number of connections needed scales,
/// but nothing would ever free them.
///
/// The server must free them itself, after its header timeout, while the
/// attacker keeps every socket open. Connections waiting in the listen backlog
/// are accepted in order, so the normal client only gets in after the batches
/// of half-sent requests ahead of it have timed out: allow a few timeouts.
#[cfg(unix)] // ulimit -n sets the server's limit on open files. Windows has no such limit for sockets.
#[test]
fn idle_connections_cannot_lock_out_clients() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(format!("ulimit -n 40 && exec '{}'", env!("CARGO_BIN_EXE_cairn-server")))
        .env("CAIRN_TOKENS", TOKEN)
        .env("CAIRN_DATA", dir.path().join("data"))
        .env("CAIRN_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(dir.path().join("server.log")).unwrap());
    let _p = Proc(c.spawn().unwrap());
    let mut up = false;
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok()
            && raw(SocketAddr::from(([127, 0, 0, 1], port)), b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").status == Some(200)
        {
            up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(up);
    let mut held = Vec::new();
    for _ in 0..60 {
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n");
            held.push(s);
        }
    }
    std::thread::sleep(Duration::from_secs(2));
    let start = Instant::now();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let r = read_resp(&mut s, 4 * cairn_server::HEADER_TIMEOUT + Duration::from_secs(5));
    let log = std::fs::read_to_string(dir.path().join("server.log")).unwrap_or_default();
    eprintln!(
        "legit request while 60 half-open: {:?} after {:.1} s\nlog tail: {}",
        r.status,
        start.elapsed().as_secs_f64(),
        log.lines().rev().take(3).collect::<Vec<_>>().join(" | ")
    );
    drop(held);
    assert_eq!(r.status, Some(200), "a normal client is locked out by 60 idle half-sent requests");
}

/// A connection that never sends a byte, and a keep-alive connection that goes
/// quiet after a request, are closed after the header timeout too.
#[test]
fn idle_connections_are_closed_after_a_timeout() {
    let srv = server();
    let mut silent = TcpStream::connect(srv.addr).unwrap();
    let mut kept = TcpStream::connect(srv.addr).unwrap();
    kept.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let r = read_resp(&mut kept, Duration::from_secs(5));
    assert_eq!(r.status, Some(200), "{r:?}");
    let wait = cairn_server::HEADER_TIMEOUT + Duration::from_secs(5);
    let r = read_resp(&mut silent, wait);
    assert!(!r.timed_out && r.status.is_none(), "silent connection: {r:?}");
    let r = read_resp(&mut kept, wait);
    assert!(!r.timed_out && r.status.is_none(), "idle keep-alive connection: {r:?}");
}

/// An authenticated upload whose body stops arriving fails after the body
/// timeout instead of holding the connection, and stores nothing.
#[test]
fn stalled_upload_body_is_closed_after_a_timeout() {
    let srv = fixture();
    let a = auth();
    let mut s = TcpStream::connect(srv.addr).unwrap();
    s.write_all(format!("POST /v1/vaults/v1/files/f2 HTTP/1.1\r\nHost: x\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{\"parent_seq\"").as_bytes()).unwrap();
    let r = read_resp(&mut s, cairn_server::BODY_TIMEOUT + Duration::from_secs(10));
    assert!(!r.timed_out, "connection with a stalled upload body still open: {r:?}");
    assert!(r.status.is_none() || is_4xx(&r), "{r:?}");
    assert_healthy(&srv, 1, 1);
}

/// The body timeout counts from the last bytes received, not from the start
/// of the request: a slow upload that keeps sending is not cut off.
#[test]
fn slow_upload_that_keeps_sending_is_not_cut_off() {
    let srv = fixture();
    let a = auth();
    let body = br#"{"parent_seq":null,"device":"d","deleted":false,"blob":"QUJD"}"#;
    let mut s = TcpStream::connect(srv.addr).unwrap();
    s.write_all(format!("POST /v1/vaults/v1/files/f2 HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: {a}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).unwrap();
    // Three pieces with two pauses: each pause is shorter than the body
    // timeout, together they are longer.
    let pause = cairn_server::BODY_TIMEOUT * 2 / 3;
    for (i, piece) in body.chunks(body.len() / 3 + 1).enumerate() {
        if i > 0 {
            std::thread::sleep(pause);
        }
        s.write_all(piece).unwrap();
    }
    let r = read_resp(&mut s, Duration::from_secs(5));
    assert_eq!(r.status, Some(200), "{r:?}");
    assert_healthy(&srv, 2, 1);
}

// ===================================================================
// FINDING-167: history of a vault that does not exist is 404, like every
// other vault route ("no such vault"), not 200 [].
// ===================================================================

#[test]
fn history_of_missing_vault_is_404() {
    let srv = fixture();
    assert_eq!(get(srv.addr, "/v1/vaults/nosuch").status, Some(404));
    assert_eq!(get(srv.addr, "/v1/vaults/nosuch/changes").status, Some(404));
    let r = get(srv.addr, "/v1/vaults/nosuch/files/f1/history");
    assert_eq!(r.status, Some(404), "{} {}", r.status.unwrap_or(0), r.body_str());
}

// ===================================================================
// FINDING-168: since >= 2^63 must not wrap to a negative number in SQL:
// after an enormous cursor the changes feed returns no heads, and the
// returned cursor does not jump backwards.
// ===================================================================

#[test]
fn huge_since_returns_nothing() {
    let srv = fixture();
    let r = get(srv.addr, "/v1/vaults/v1/changes?since=9223372036854775808");
    assert_eq!(r.status, Some(200));
    let b = r.body_str();
    assert!(b.contains(r#""heads":[]"#), "since=2^63 returned heads: {b}");
    // With nothing new, the cursor stays where the client was.
    assert!(b.contains(r#""cursor":9223372036854775808"#), "cursor went backwards: {b}");
    let b = get(srv.addr, "/v1/vaults/v1/changes?since=18446744073709551615").body_str();
    assert!(b.contains(r#""heads":[]"#) && b.contains(r#""cursor":18446744073709551615"#), "since=u64::MAX: {b}");
}

// ===================================================================
// FINDING-169: the Bearer scheme is matched case-insensitively; RFC 9110
// section 11.1 says auth schemes are case-insensitive ("bearer x" is valid).
// ===================================================================

#[test]
fn bearer_scheme_is_case_insensitive() {
    let srv = fixture();
    for scheme in ["bearer", "BEARER", "BeArEr"] {
        let v = format!("{scheme} {TOKEN}");
        let r = raw(srv.addr, &req("GET", "/v1/vaults/v1", &[("Authorization", &v)], b""));
        assert_eq!(r.status, Some(200), "{scheme}: {r:?}");
    }
}

// ===================================================================
// FINDING-170: error responses come in one format. Like the server's own
// errors, axum's extractor rejections (bad JSON, bad query, bad path, wrong
// content type) are JSON, not text/plain.
// ===================================================================

#[test]
fn all_errors_are_json() {
    let srv = fixture();
    let a = auth();
    let probes = vec![
        ("bad json", post_json(srv.addr, "POST", "/v1/vaults/v1/files/f1", "{")),
        ("missing field", post_json(srv.addr, "POST", "/v1/vaults/v1/files/f1", "{}")),
        ("bad query", get(srv.addr, "/v1/vaults/v1/changes?since=-1")),
        ("bad path", get(srv.addr, "/v1/vaults/v1/revisions/x")),
        ("bad utf8 path", get(srv.addr, "/v1/vaults/%FF")),
        ("no content type", raw(srv.addr, &req("POST", "/v1/vaults/v1/files/f1", &[("Authorization", &a)], b"{}"))),
    ];
    let mut bad = Vec::new();
    for (name, r) in probes {
        let ct = r.header("content-type").unwrap_or_default();
        if !ct.starts_with("application/json") {
            bad.push(format!("{name}: {} {ct:?} {:?}", r.status.unwrap_or(0), r.body_str()));
        }
    }
    assert!(bad.is_empty(), "non-JSON error responses:\n{}", bad.join("\n"));
}

// ===================================================================
// Logging: even at RUST_LOG=trace the server writes no token,
// blob or device name to its log.
// ===================================================================

#[test]
fn server_log_contains_no_tokens_or_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let p = spawn_bin(dir.path(), port, &[("RUST_LOG", "trace")]);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    create_vault(addr, "v1");
    let r = post_json(addr, "POST", "/v1/vaults/v1/files/f1", r#"{"parent_seq":null,"device":"SECRETDEVICE","deleted":false,"blob":"U0VDUkVUQkxPQg=="}"#);
    assert_eq!(r.status, Some(200), "{r:?}");
    let _ = raw(addr, &req("GET", "/v1/vaults/v1", &[("Authorization", "Bearer WRONGTOKENVALUE123")], b""));
    let _ = post_json(addr, "POST", "/v1/vaults/v1/files/f1", r#"{"parent_seq":null,"device":"SECRETDEVICE","blob":"U0VDUkVUQkxPQg=="}"#);
    let _ = raw(addr, b"GARBAGE U0VDUkVUQkxPQg==\r\n\r\n");
    drop(p);
    let log = std::fs::read_to_string(dir.path().join("server.log")).unwrap();
    for secret in [TOKEN, "WRONGTOKENVALUE123", "U0VDUkVUQkxPQg==", "SECRETBLOB", "SECRETDEVICE"] {
        assert!(!log.contains(secret), "log contains {secret}:\n{log}");
    }
}

// ===================================================================
// FINDING-171: failed authentication is logged, so an operator can see a
// misconfigured device or token guessing.
// ===================================================================

#[test]
fn failed_auth_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let p = spawn_bin(dir.path(), port, &[]);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    for _ in 0..5 {
        let r = raw(addr, &req("GET", "/v1/vaults/v1", &[("Authorization", "Bearer guess_guess_guess_1")], b""));
        assert_eq!(r.status, Some(401));
    }
    drop(p);
    let log = std::fs::read_to_string(dir.path().join("server.log")).unwrap();
    assert!(log.to_lowercase().contains("auth") || log.contains("401") || log.contains("token"), "log after 5 rejected requests:\n{log}");
    // One warning per refused request, with the client address and without
    // what the client presented.
    let warns = log.lines().filter(|l| l.contains("WARN")).collect::<Vec<_>>();
    assert_eq!(warns.len(), 5, "{log}");
    assert!(warns.iter().all(|l| l.contains("127.0.0.1:")), "{log}");
    assert!(!log.contains("guess_guess_guess_1"), "log contains the presented token:\n{log}");
}
