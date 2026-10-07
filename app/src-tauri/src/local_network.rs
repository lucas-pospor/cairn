//! Which sync servers are on the local network, and what to tell the user
//! when Android does not let Cairn reach them.
//!
//! Android 17 blocks apps that target SDK 37 from local network addresses
//! unless they hold the runtime permission ACCESS_LOCAL_NETWORK, which
//! Android settings show as "Nearby devices". A blocked connection just
//! times out. `android::allow_server` asks for the permission before sync
//! connects to a server that [`server_is_local`] finds on the local network.

use std::net::{IpAddr, ToSocketAddrs};

/// Whether `ip` is a local network address: a private IPv4 address
/// (10/8, 172.16/12, 192.168/16), a link-local address (169.254/16,
/// fe80::/10) or an IPv6 unique local address (fc00::/7). Loopback and
/// carrier-grade NAT (100.64/10, also used by VPNs such as Tailscale) are
/// not the local network.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
            }
        },
    }
}

/// The host of a server URL (`https://host:port/path`), without the port
/// and without the brackets of an IPv6 address.
pub fn host(server: &str) -> Option<&str> {
    let rest = server.trim().split_once("://").map_or(server.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once(']')?.0,
        None => authority.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

/// Whether the server at URL `server` is on the local network: a `.local`
/// name (mDNS), or a name or address that resolves to a local one (see
/// [`is_local`]). A name that does not resolve is not: the connection then
/// says that no server with that name was found.
pub fn server_is_local(server: &str) -> bool {
    let Some(host) = host(server) else { return false };
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "local" || lower.ends_with(".local") {
        return true;
    }
    match (host, 0).to_socket_addrs() {
        Ok(mut addrs) => addrs.any(|a| is_local(a.ip())),
        Err(_) => false,
    }
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
/// an address as local that [`is_local`] does not (a public address on the
/// same network as the phone).
pub const TIMEOUT_HINT: &str = "If the server is on your local network, allow \"Nearby devices\" for Cairn in Android settings (Apps > Cairn > Permissions)";

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn local_addresses() {
        for a in ["192.168.10.10", "10.0.0.2", "172.16.0.1", "172.31.255.254", "169.254.1.1", "fd12:3456::1", "fc00::1", "fe80::1", "::ffff:192.168.1.5"] {
            assert!(is_local(ip(a)), "{a}");
        }
        for a in ["8.8.8.8", "172.32.0.1", "100.64.0.1", "127.0.0.1", "::1", "2001:db8::1", "::ffff:8.8.8.8", "0.0.0.0"] {
            assert!(!is_local(ip(a)), "{a}");
        }
    }

    #[test]
    fn host_of_a_server_url() {
        assert_eq!(host("https://notes.example.com"), Some("notes.example.com"));
        assert_eq!(host("https://notes.example.com/"), Some("notes.example.com"));
        assert_eq!(host(" http://192.168.1.5:8080/cairn "), Some("192.168.1.5"));
        assert_eq!(host("https://[fd00::5]:8443/x"), Some("fd00::5"));
        assert_eq!(host("https://user@nas.local:443"), Some("nas.local"));
        assert_eq!(host("nas.local:8080"), Some("nas.local"));
        assert_eq!(host("https://"), None);
        assert_eq!(host(""), None);
    }

    #[test]
    fn local_servers() {
        assert!(server_is_local("https://192.168.10.10"));
        assert!(server_is_local("http://[fe80::1]:8080"));
        assert!(server_is_local("https://nas.local"));
        assert!(server_is_local("https://NAS.Local./"));
        assert!(!server_is_local("https://8.8.8.8"));
        assert!(!server_is_local("http://127.0.0.1:9"));
        assert!(!server_is_local("https://"));
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
