//! Which sync servers are on the local network, and what to tell the user
//! when Android does not let Cairn reach them.
//!
//! Android 17 blocks apps that target SDK 37 from local network addresses
//! unless they hold the runtime permission ACCESS_LOCAL_NETWORK, which
//! Android settings show as "Nearby devices". A blocked connection just
//! times out. `android::allow_server` asks for the permission before sync
//! connects to a server that [`local_addrs`] finds on the local network,
//! unless it [`answers`] all the same: Android does not count a connection
//! through a VPN as the local network.

use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long [`answers`] waits for a server. A blocked connection never
/// answers; one on the local network or through a VPN answers well within
/// this.
pub const PROBE: Duration = Duration::from_secs(2);

/// Whether `ip` is in a range that Android counts as the local network on
/// Wi-Fi or Ethernet: a private IPv4 address (10/8, 172.16/12, 192.168/16),
/// shared address space (100.64/10, carrier-grade NAT, which VPNs such as
/// Tailscale also use), a link-local address (169.254/16, fe80::/10) or an
/// IPv6 unique local address (fc00::/7). Loopback is not.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private() || v4.is_link_local() || (a == 100 && b & 0xc0 == 64)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
            }
        },
    }
}

/// The scheme and the authority (`host:port`, without user info) of a
/// server URL (`https://host:port/path`).
fn split(server: &str) -> (&str, &str) {
    let server = server.trim();
    let (scheme, rest) = server.split_once("://").unwrap_or(("", server));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    (scheme, authority.rsplit_once('@').map_or(authority, |(_, a)| a))
}

/// The host of a server URL, without the port and without the brackets of
/// an IPv6 address.
pub fn host(server: &str) -> Option<&str> {
    let (_, authority) = split(server);
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once(']')?.0,
        None => authority.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

/// The port of a server URL: the one it names, or 443 for https and 80
/// otherwise.
pub fn port(server: &str) -> u16 {
    let (scheme, authority) = split(server);
    let after_host = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map_or("", |(_, r)| r),
        None => authority.find(':').map_or("", |i| &authority[i..]),
    };
    let default = if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 };
    after_host.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(default)
}

/// The local network addresses of the server at URL `server` (see
/// [`is_local`]), or None when it is not on the local network. A name that
/// does not resolve is not: the connection then says that no server with
/// that name was found. A `.local` name (mDNS) is on it, with no address:
/// without the permission it does not resolve.
pub fn local_addrs(server: &str) -> Option<Vec<SocketAddr>> {
    let host = host(server)?;
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "local" || lower.ends_with(".local") {
        return Some(Vec::new());
    }
    let local: Vec<SocketAddr> = (host, port(server)).to_socket_addrs().ok()?.filter(|a| is_local(a.ip())).collect();
    (!local.is_empty()).then_some(local)
}

/// Whether one of `addrs` accepts a connection within `wait`, all tried at
/// once. Without the permission, Android drops a connection to the local
/// network, so it does not; through a VPN it does.
pub fn answers(addrs: &[SocketAddr], wait: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    for &a in addrs {
        let tx = tx.clone();
        std::thread::spawn(move || tx.send(TcpStream::connect_timeout(&a, wait).is_ok()));
    }
    drop(tx);
    let end = Instant::now() + wait;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(true) => return true,
            Ok(false) => continue,
            // All refused, or the time is up.
            Err(_) => return false,
        }
    }
    false
}

/// Why sync does not connect to `server`, which is on the local network,
/// while Cairn lacks the permission. `can_ask`: Android can still show its
/// prompt, so Sync now asks again.
pub fn denied_text(server: &str, can_ask: bool) -> String {
    let host = host(server).unwrap_or(server);
    if can_ask {
        format!("{host} is on your local network, and Cairn needs the \"Nearby devices\" permission to connect to it. Press Sync now to allow it")
    } else {
        format!(
            "{host} is on your local network, and Cairn is not allowed to connect to devices there. Allow \"Nearby devices\" for Cairn in Android settings (Apps > Cairn > Permissions), then sync again"
        )
    }
}

/// Added to a timeout while Cairn lacks the permission: Android may count
/// an address as local that [`is_local`] does not (a public IPv6 address on
/// the phone's own network).
pub const TIMEOUT_HINT: &str = "If the server is on your local network, allow \"Nearby devices\" for Cairn in Android settings (Apps > Cairn > Permissions)";

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn local_addresses() {
        for a in ["192.168.10.10", "10.0.0.2", "172.16.0.1", "172.31.255.254", "169.254.1.1", "100.64.0.1", "100.127.255.254", "fd12:3456::1", "fc00::1", "fe80::1", "::ffff:192.168.1.5"] {
            assert!(is_local(ip(a)), "{a}");
        }
        for a in ["8.8.8.8", "172.32.0.1", "100.63.255.255", "100.128.0.1", "127.0.0.1", "::1", "2001:db8::1", "::ffff:8.8.8.8", "0.0.0.0"] {
            assert!(!is_local(ip(a)), "{a}");
        }
    }

    #[test]
    fn host_and_port_of_a_server_url() {
        assert_eq!(host("https://notes.example.com"), Some("notes.example.com"));
        assert_eq!(host("https://notes.example.com/"), Some("notes.example.com"));
        assert_eq!(host(" http://192.168.1.5:8080/cairn "), Some("192.168.1.5"));
        assert_eq!(host("https://[fd00::5]:8443/x"), Some("fd00::5"));
        assert_eq!(host("https://user@nas.local:443"), Some("nas.local"));
        assert_eq!(host("nas.local:8080"), Some("nas.local"));
        assert_eq!(host("https://"), None);
        assert_eq!(host(""), None);
        assert_eq!(port("https://notes.example.com"), 443);
        assert_eq!(port("HTTPS://notes.example.com/x"), 443);
        assert_eq!(port("http://notes.example.com"), 80);
        assert_eq!(port(" http://192.168.1.5:8787/cairn "), 8787);
        assert_eq!(port("https://[fd00::5]:8443/x"), 8443);
        assert_eq!(port("https://[fd00::5]/x"), 443);
        assert_eq!(port("https://user:pw@nas.local:9000"), 9000);
        assert_eq!(port("http://host:notaport/"), 80);
    }

    #[test]
    fn local_servers() {
        let at = |s: &str| local_addrs(s).map(|v| v.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(at("http://192.168.10.10:8787"), Some(vec!["192.168.10.10:8787".to_string()]));
        assert_eq!(at("https://192.168.10.10"), Some(vec!["192.168.10.10:443".to_string()]));
        assert_eq!(at("http://[fe80::1]:8080"), Some(vec!["[fe80::1]:8080".to_string()]));
        assert_eq!(at("http://100.100.1.2"), Some(vec!["100.100.1.2:80".to_string()]));
        // mDNS names are local, and are not resolved.
        assert_eq!(at("https://nas.local"), Some(vec![]));
        assert_eq!(at("https://NAS.Local./"), Some(vec![]));
        assert_eq!(at("https://8.8.8.8"), None);
        assert_eq!(at("http://127.0.0.1:9"), None);
        assert_eq!(at("https://"), None);
    }

    #[test]
    fn a_server_that_answers_and_one_that_does_not() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let open = listener.local_addr().unwrap();
        assert!(answers(&[open], PROBE));
        drop(listener);
        // Nothing listens there now: refused at once.
        assert!(!answers(&[open], PROBE));
        assert!(!answers(&[], PROBE));
        // One that answers among several that do not.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(answers(&[open, listener.local_addr().unwrap(), open], PROBE));
    }

    #[test]
    fn texts_name_the_host_and_the_setting() {
        let t = denied_text("https://notes.example.com/", false);
        assert!(t.starts_with("notes.example.com is on your local network"), "{t}");
        assert!(t.contains("\"Nearby devices\"") && t.contains("Android settings"), "{t}");
        assert!(denied_text("https://nas.local", true).contains("Press Sync now"));
        assert!(TIMEOUT_HINT.contains("\"Nearby devices\""));
    }

    /// The manifest is in git and edited by hand (`tauri android init` would
    /// write a new one): without the permission there, Android neither asks
    /// for it nor grants it, and sync to a local server times out again.
    #[test]
    fn the_android_manifest_declares_the_permission() {
        let manifest = include_str!("../gen/android/app/src/main/AndroidManifest.xml");
        assert!(manifest.contains(r#"<uses-permission android:name="android.permission.ACCESS_LOCAL_NETWORK" />"#));
        let plugin = include_str!("../gen/android/app/src/main/java/app/cairn/notes/LocalNetworkPlugin.kt");
        assert!(plugin.contains(r#""android.permission.ACCESS_LOCAL_NETWORK""#));
        let keep = include_str!("../gen/android/app/proguard-rules.pro");
        assert!(keep.contains("-keep class app.cairn.notes.LocalNetworkPlugin { *; }"));
    }
}
