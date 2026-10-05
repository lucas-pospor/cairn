//! How the engine talks to the server. [`HttpTransport`] is the real one.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::time::Duration as Wait;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport as Wire};
use ureq::Timeout;

use crate::protocol::*;
use crate::SyncError;

/// Largest request the client sends and largest record it reads, in bytes:
/// the server's default upload limit (CAIRN_MAX_BODY_MB=200). Files travel
/// base64-encoded in JSON, so files up to about 150 MB fit. Larger uploads
/// are refused here even if the server would take them, so that every
/// device can download what was uploaded.
pub const MAX_BODY: u64 = 200 << 20;
/// Room for the JSON around one record in a response.
const RECORD_SLACK: u64 = 64 << 10;
/// For finding the server and opening the connection (TLS included).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// For the server to start answering once it has the whole request.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
/// A transfer that moves less than SLOWEST bytes per second over STALL_TIME
/// has stalled and is given up. There is no limit on the total time, so
/// large files can be sent over slow links.
const SLOWEST: usize = 1 << 10;
const STALL_TIME: Duration = Duration::from_secs(30);
const STALL_BYTES: usize = SLOWEST * STALL_TIME.as_secs() as usize;
/// Most of an upload that can still be in network buffers (socket send
/// buffers are a few MB) after its last byte has been handed over.
const IN_FLIGHT: usize = 8 << 20;

pub enum PutOutcome {
    Stored(u64),
    /// Someone else changed the file first; pull and merge, then retry.
    Conflict(Option<u64>),
}

pub trait Transport: Send + Sync {
    fn get_vault(&self, vault: &str) -> Result<Option<VaultInfo>, SyncError>;
    fn create_vault(&self, vault: &str, keys: &KeyEnvelope) -> Result<(), SyncError>;
    fn changes(&self, vault: &str, since: u64, limit: u32) -> Result<ChangesResponse, SyncError>;
    fn put(&self, vault: &str, file_id: &str, rev: &PutRevision) -> Result<PutOutcome, SyncError>;
    fn history(&self, vault: &str, file_id: &str) -> Result<Vec<HistoryEntry>, SyncError>;
    fn revision(&self, vault: &str, seq: u64) -> Result<RevisionBlob, SyncError>;
    /// Largest file that [`Self::put`] can send, if it has a limit.
    fn max_file_size(&self) -> u64 {
        u64::MAX
    }
}

pub struct HttpTransport {
    base: String,
    token: String,
    agent: ureq::Agent,
    max_body: u64,
    /// Records per changes page that fit last time (see [`Self::changes`]).
    page: AtomicU32,
}

impl HttpTransport {
    pub fn new(server: &str, token: &str) -> HttpTransport {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_resolve(Some(CONNECT_TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(ANSWER_TIMEOUT))
            .build();
        let agent = ureq::Agent::with_parts(config, DefaultConnector::new().chain(StallGuard), DefaultResolver::default());
        HttpTransport {
            base: server.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent,
            max_body: MAX_BODY,
            page: AtomicU32::new(u32::MAX),
        }
    }

    /// Use another size limit than [`MAX_BODY`].
    pub fn with_max_body(mut self, bytes: u64) -> HttpTransport {
        self.max_body = bytes;
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}{}", self.base, API_PREFIX, path)
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// An upload stalled or was cut off while being sent. If the server
    /// still answers, the problem is this upload (too large for a proxy, too
    /// slow a link) and the other files can go on; otherwise the connection
    /// is down and `e` stands.
    fn upload_failed(&self, vault: &str, reason: String, e: SyncError) -> SyncError {
        match self.get_vault(vault) {
            Ok(Some(_)) => SyncError::Upload(reason),
            _ => e,
        }
    }
}

/// A failed request in words the user can act on, not the HTTP client's
/// ("io: Connection refused (os error 111)", "http: invalid format").
fn net(e: ureq::Error) -> SyncError {
    use std::io::ErrorKind as K;
    use ureq::Error as E;
    SyncError::Network(match e {
        E::Http(_) | E::BadUri(_) => "the URL must start with https:// or http://".into(),
        E::HostNotFound => "no server with that name was found".into(),
        E::Timeout(_) => "it did not answer in time".into(),
        E::Json(_) => "unexpected answer; is this a Cairn server?".into(),
        E::Io(io) => match io.kind() {
            K::ConnectionRefused => "connection refused; is the server running?".into(),
            K::TimedOut => "it did not answer in time".into(),
            K::NetworkUnreachable | K::HostUnreachable | K::NetworkDown => "no network connection".into(),
            _ => cairn_core::error::os_text(&io),
        },
        e => e.to_string(),
    })
}

/// A response that arrived but does not parse is the server's fault, not
/// the connection's.
fn bad_body(e: ureq::Error) -> SyncError {
    match e {
        ureq::Error::Json(e) if !e.is_io() => unexpected(e),
        e => net(e),
    }
}

/// A body that does not parse, in the words [`net`] has for one. The
/// parser's own text goes to the log.
fn unexpected(e: serde_json::Error) -> SyncError {
    log::warn!("sync: the server's answer does not parse: {e}");
    SyncError::Server("unexpected answer; is this a Cairn server?".into())
}

fn read_error(mut resp: ureq::http::Response<ureq::Body>) -> SyncError {
    let status = resp.status().as_u16();
    if status == 401 {
        return SyncError::Unauthorized;
    }
    // A Cairn server explains errors as {"error": "..."}; show that text, not
    // the raw body (which from another web server is often an HTML page).
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    let reason = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("error")?.as_str().map(str::to_string));
    SyncError::Server(match reason {
        Some(r) => format!("{r} (HTTP {status})"),
        None => format!("HTTP {status}"),
    })
}

/// A 404 that is not a Cairn server's own answer: the URL points somewhere
/// else (a wrong path, another web server). Its body is that server's page,
/// so it is not shown. Kept to one line in the setup form, like the texts
/// in [`net`], so its Connect button stays in view.
fn not_cairn() -> SyncError {
    SyncError::Server("there is no Cairn sync server at this address. Check the URL".into())
}

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1 << 20) as f64;
    if mb >= 10.0 { format!("{mb:.0} MB") } else { format!("{mb:.1} MB") }
}

/// Why a file over the client's limit is not uploaded.
pub(crate) fn too_large(max_file: u64) -> String {
    format!("too large to sync (over {})", megabytes(max_file))
}

/// Adds stall detection to ureq's connections. ureq's own timeouts limit
/// the total time of each step of a request, which would also limit the
/// size of a file that can be sent over a slow link. A connection here only
/// gives up when a read or write makes no progress for [`STALL_TIME`], or
/// moves less than [`STALL_BYTES`] in that time. Waiting for the server to
/// answer is left to ureq's [`ANSWER_TIMEOUT`].
#[derive(Debug)]
struct StallGuard;

impl<In: Wire> Connector<In> for StallGuard {
    type Out = Guarded<In>;

    fn connect(&self, _: &ConnectionDetails, chained: Option<In>) -> Result<Option<Guarded<In>>, ureq::Error> {
        Ok(chained.map(|inner| Guarded { inner, doing: None, since: Instant::now(), moved: 0 }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Doing {
    Sending,
    /// Waiting for the response headers.
    Waiting,
    Receiving,
}

#[derive(Debug)]
struct Guarded<T> {
    inner: T,
    /// What the connection did last; a change starts a new transfer.
    doing: Option<Doing>,
    since: Instant,
    /// Bytes moved since `since`.
    moved: usize,
}

impl<T: Wire> Guarded<T> {
    /// The timeout for the next read or write.
    fn begin(&mut self, doing: Doing, t: NextTimeout) -> NextTimeout {
        if self.doing != Some(doing) {
            self.doing = Some(doing);
            self.since = Instant::now();
            self.moved = 0;
        }
        if doing == Doing::Waiting {
            return t;
        }
        // Steps without a timeout of their own are reported as global.
        let reason = match (t.reason, doing) {
            (Timeout::Global, Doing::Sending) => Timeout::SendBody,
            (Timeout::Global, _) => Timeout::RecvBody,
            (r, _) => r,
        };
        let after = if t.after.is_not_happening() { STALL_TIME } else { (*t.after).min(STALL_TIME) };
        NextTimeout { after: Wait::Exact(after), reason }
    }

    fn progress(&mut self, n: usize, reason: Timeout) -> Result<(), ureq::Error> {
        if self.doing == Some(Doing::Waiting) {
            return Ok(());
        }
        self.moved += n;
        if self.since.elapsed() >= STALL_TIME {
            if self.moved < STALL_BYTES {
                return Err(ureq::Error::Timeout(reason));
            }
            self.since = Instant::now();
            self.moved = 0;
        }
        Ok(())
    }
}

impl<T: Wire> Wire for Guarded<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let t = self.begin(Doing::Sending, timeout);
        self.inner.transmit_output(amount, t)?;
        self.progress(amount, t.reason)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let doing = if timeout.reason == Timeout::RecvResponse { Doing::Waiting } else { Doing::Receiving };
        let t = self.begin(doing, timeout);
        let before = self.inner.buffers().input().len();
        let progressed = self.inner.await_input(t)?;
        let n = self.inner.buffers().input().len().saturating_sub(before);
        self.progress(n, t.reason)?;
        Ok(progressed)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

impl Transport for HttpTransport {
    fn get_vault(&self, vault: &str) -> Result<Option<VaultInfo>, SyncError> {
        let mut r = self.agent.get(&self.url(&format!("/vaults/{vault}"))).header("Authorization", &self.auth()).call().map_err(net)?;
        match r.status().as_u16() {
            200 => Ok(Some(r.body_mut().read_json().map_err(net)?)),
            // Only the server's own answer means it has no such vault. Any
            // other 404 (a wrong path, another web server) is an error, so
            // that setup never offers to create a vault there.
            404 => {
                let body = r.body_mut().read_to_string().unwrap_or_default();
                if body.contains("no such vault") {
                    Ok(None)
                } else {
                    Err(not_cairn())
                }
            }
            _ => Err(read_error(r)),
        }
    }

    fn create_vault(&self, vault: &str, keys: &KeyEnvelope) -> Result<(), SyncError> {
        let r = self
            .agent
            .put(&self.url(&format!("/vaults/{vault}")))
            .header("Authorization", &self.auth())
            .send_json(keys)
            .map_err(net)?;
        match r.status().as_u16() {
            201 => Ok(()),
            // Every Cairn server has this route: the URL points somewhere else.
            404 => Err(not_cairn()),
            _ => Err(read_error(r)),
        }
    }

    /// A page holds whole records, so it can be much larger than one upload.
    /// When it is over the limit, fewer records are asked for until it fits;
    /// one record always does. The next call starts from what fitted.
    fn changes(&self, vault: &str, since: u64, limit: u32) -> Result<ChangesResponse, SyncError> {
        let max = self.max_body + RECORD_SLACK;
        let mut limit = limit.min(self.page.load(Ordering::Relaxed).saturating_mul(2)).max(1);
        loop {
            let mut r = self
                .agent
                .get(&self.url(&format!("/vaults/{vault}/changes?since={since}&limit={limit}")))
                .header("Authorization", &self.auth())
                .call()
                .map_err(net)?;
            match r.status().as_u16() {
                200 => {}
                404 => return Err(SyncError::NoSuchVault(vault.to_string())),
                _ => return Err(read_error(r)),
            }
            let oversized = || SyncError::Server(format!("a change on the server is too large to download (over {})", megabytes(self.max_file_size())));
            let page = match r.body().content_length() {
                Some(len) if len > max && limit == 1 => return Err(oversized()),
                Some(len) if len > max => {
                    limit = ((limit as u64 * max / len) as u32).clamp(1, limit / 2);
                    continue;
                }
                Some(_) => r.body_mut().with_config().limit(max).read_json().map_err(net)?,
                // Compressed on the way (by a proxy): the size shows only
                // while reading.
                None => match r.body_mut().with_config().limit(max).read_to_vec() {
                    Ok(body) => serde_json::from_slice(&body).map_err(unexpected)?,
                    Err(ureq::Error::BodyExceedsLimit(_)) if limit == 1 => return Err(oversized()),
                    Err(ureq::Error::BodyExceedsLimit(_)) => {
                        limit = (limit / 4).max(1);
                        continue;
                    }
                    Err(e) => return Err(net(e)),
                },
            };
            self.page.store(limit, Ordering::Relaxed);
            return Ok(page);
        }
    }

    fn put(&self, vault: &str, file_id: &str, rev: &PutRevision) -> Result<PutOutcome, SyncError> {
        use std::io::ErrorKind::*;
        let body = serde_json::to_vec(rev).map_err(|e| SyncError::Local(e.to_string()))?;
        if body.len() as u64 > self.max_body {
            return Err(SyncError::Upload(too_large(self.max_file_size())));
        }
        // Part of a large upload can still be in network buffers when its
        // last byte has been handed over, so the wait for the answer grows
        // with what can be in them, at the slowest rate allowed.
        let wait = ANSWER_TIMEOUT + Duration::from_secs((body.len().min(IN_FLIGHT) / SLOWEST) as u64);
        let sent = self
            .agent
            .post(&self.url(&format!("/vaults/{vault}/files/{file_id}")))
            .header("Authorization", &self.auth())
            .header("Content-Type", "application/json")
            .config()
            .timeout_recv_response(Some(wait))
            .build()
            .send(&body[..]);
        // Only a failure of this upload itself lets the other files go on:
        // too large, or stalled or cut off while being sent. A server that
        // takes the whole request and does not answer, or answers with
        // another error, would fail the other uploads too: sync stops.
        let mut r = match sent {
            Ok(r) => r,
            Err(e @ ureq::Error::Timeout(Timeout::SendRequest | Timeout::SendBody)) => {
                return Err(self.upload_failed(vault, "the upload timed out".into(), net(e)));
            }
            Err(ureq::Error::Io(e)) if matches!(e.kind(), BrokenPipe | ConnectionReset | ConnectionAborted | UnexpectedEof) => {
                let reason = "the server closed the connection during the upload".into();
                return Err(self.upload_failed(vault, reason, net(ureq::Error::Io(e))));
            }
            Err(e) => return Err(net(e)),
        };
        match r.status().as_u16() {
            200 => Ok(PutOutcome::Stored(r.body_mut().read_json::<PutResult>().map_err(net)?.seq)),
            409 => Ok(PutOutcome::Conflict(r.body_mut().read_json::<Conflict>().map_err(net)?.current_seq)),
            413 => Err(SyncError::Upload("the server does not accept files this large".into())),
            _ => Err(read_error(r)),
        }
    }

    /// Files travel base64-encoded, so a larger one cannot fit in
    /// `max_body` (and a file just under it may still not, with the JSON).
    fn max_file_size(&self) -> u64 {
        self.max_body / 4 * 3
    }

    fn history(&self, vault: &str, file_id: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        let mut r = self
            .agent
            .get(&self.url(&format!("/vaults/{vault}/files/{file_id}/history")))
            .header("Authorization", &self.auth())
            .call()
            .map_err(net)?;
        match r.status().as_u16() {
            200 => Ok(r.body_mut().read_json().map_err(net)?),
            _ => Err(read_error(r)),
        }
    }

    fn revision(&self, vault: &str, seq: u64) -> Result<RevisionBlob, SyncError> {
        let mut r = self
            .agent
            .get(&self.url(&format!("/vaults/{vault}/revisions/{seq}")))
            .header("Authorization", &self.auth())
            .call()
            .map_err(net)?;
        match r.status().as_u16() {
            200 => Ok(r.body_mut().with_config().limit(self.max_body + RECORD_SLACK).read_json().map_err(bad_body)?),
            _ => Err(read_error(r)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A proxy that compresses responses sends no Content-Length, so a page
    /// that is too large shows only while it is read.
    #[test]
    fn a_page_of_unknown_size_over_the_limit_is_asked_for_in_fewer_records() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                let mut line = String::new();
                let mut r = BufReader::new(c.try_clone().unwrap());
                r.read_line(&mut line).unwrap();
                while r.read_line(&mut String::new()).unwrap() > 2 {}
                let limit: u32 = line.split("limit=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
                seen2.lock().unwrap().push(limit);
                let body = if limit == 1 {
                    r#"{"heads":[],"cursor":7,"more":true}"#.to_string()
                } else {
                    format!(r#"{{"heads":[],"cursor":7,"more":true,"pad":"{}"}}"#, "x".repeat(200 << 10))
                };
                let _ = write!(c, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
            }
        });
        let t = HttpTransport::new(&url, "token").with_max_body(64 << 10);
        let page = t.changes("v", 0, 500).unwrap();
        assert_eq!(page.cursor, 7);
        assert_eq!(*seen.lock().unwrap(), vec![500, 125, 31, 7, 1]);
        // the next page starts near the size that fitted
        t.changes("v", 7, 500).unwrap();
        assert_eq!(seen.lock().unwrap()[5..], [2, 1]);
    }

    /// An answer that does not parse (a proxy's page, say) is named in the
    /// same words as at setup, not in the JSON parser's.
    #[test]
    fn an_answer_that_does_not_parse_is_named_plainly() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                let mut r = BufReader::new(c.try_clone().unwrap());
                while r.read_line(&mut String::new()).unwrap() > 2 {}
                let body = "<html>Sign in first</html>";
                let _ = write!(c, "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
            }
        });
        let t = HttpTransport::new(&url, "token");
        let plain = "server error: unexpected answer; is this a Cairn server?";
        assert_eq!(t.changes("v", 0, 500).unwrap_err().to_string(), plain);
        assert_eq!(t.revision("v", 1).unwrap_err().to_string(), plain);
    }
}
