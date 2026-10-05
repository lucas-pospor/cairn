//! Asking a server whether it has a vault (FINDING-164, FINDING-083): sync
//! setup asks the user before it creates a vault the server does not have,
//! so only a Cairn server's own "no such vault" may count as missing. Any
//! other 404 says in a plain sentence that the address is not a Cairn
//! server, not with that server's page (FINDING-120).
//!
//! Run with:
//!   cargo test -p cairn-sync --test vault_lookup

#[path = "adv_sync_semantics_common.rs"]
mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use cairn_sync::crypto::VaultKey;
use cairn_sync::engine::SyncEngine;
use cairn_sync::transport::{HttpTransport, Transport};
use cairn_sync::SyncError;
use common::*;

/// The page another web server sends for a path it does not have.
const PAGE_404: &str = "<html><body>404 Not Found</body></html>";

/// A web server that is not a Cairn server: every request gets its 404 page.
fn another_web_server() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { return };
            let mut r = BufReader::new(c.try_clone().unwrap());
            // The whole request, body too: closing with an unread body
            // resets the connection before the client reads the answer.
            let mut len = 0;
            let mut line = String::new();
            while r.read_line(&mut line).unwrap_or(0) > 2 {
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                line.clear();
            }
            let _ = std::io::copy(&mut r.take(len), &mut std::io::sink());
            let _ = write!(c, "HTTP/1.1 404 Not Found\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE_404}", PAGE_404.len());
        }
    });
    url
}

/// A 404 that is not a Cairn server's own answer reads as a plain sentence
/// (FINDING-120), not as that server's body: a JSON error or an HTML page.
fn assert_not_a_cairn_server(e: &SyncError) {
    let text = e.to_string();
    assert!(text.contains("no Cairn sync server at this address") && text.contains("Check the URL"), "{text:?}");
    assert!(!text.contains('{') && !text.contains('<') && !text.contains("not found"), "the 404 body is shown: {text:?}");
}

#[test]
fn a_vault_the_server_does_not_have_is_missing_and_looking_does_not_create_it() {
    let srv = server();
    let t = HttpTransport::new(&srv.url, TOKEN);
    assert!(t.get_vault("notes").unwrap().is_none());
    assert!(t.get_vault("notes").unwrap().is_none(), "looking created the vault");
    t.create_vault("notes", &VaultKey::generate().wrap(PASS, FAST_KDF).unwrap()).unwrap();
    assert!(t.get_vault("notes").unwrap().is_some());
    // Vault names are case-sensitive.
    assert!(t.get_vault("Notes").unwrap().is_none());
}

#[test]
fn a_not_found_answer_from_a_wrong_path_is_an_error_not_a_missing_vault() {
    let srv = server();
    // A path prefix the server does not have (a reverse proxy set up for /notes).
    let t = HttpTransport::new(&format!("{}/notes", srv.url), TOKEN);
    let r = t.get_vault("notes");
    assert!(r.is_err(), "a 404 for an unknown path was taken for a missing vault: {:?}", r.map(|v| v.is_some()));
}

#[test]
fn a_not_found_answer_from_another_web_server_is_an_error_not_a_missing_vault() {
    let url = another_web_server();
    let r = HttpTransport::new(&url, TOKEN).get_vault("notes");
    assert!(r.is_err(), "another server's 404 was taken for a missing vault: {:?}", r.map(|v| v.is_some()));
}

#[test]
fn setup_against_an_address_that_is_not_a_cairn_server_says_so_plainly() {
    let srv = server();
    for url in [format!("{}/nothing-here", srv.url), another_web_server()] {
        let t = HttpTransport::new(&url, TOKEN);
        assert_not_a_cairn_server(&t.get_vault("notes").unwrap_err());
        assert_not_a_cairn_server(&t.create_vault("notes", &VaultKey::generate().wrap(PASS, FAST_KDF).unwrap()).unwrap_err());
    }
}

/// The check before each sync: a server without the vault still gets its
/// own message, an address that is not a Cairn server the plain sentence.
#[test]
fn a_synced_device_whose_address_is_not_a_cairn_server_says_so_plainly() {
    let srv = server();
    let mut a = Device::new(&srv, "laptop", &[("a.md", "a\n")]);
    a.sync();
    // What answers at the address set up here changes.
    let mut sync_with = |url: &str| {
        let settings = a.engine.settings().clone();
        a.engine = SyncEngine::load_with(a.vault.clone(), &a.state_dir, settings, Box::new(HttpTransport::new(url, TOKEN))).unwrap();
        a.try_sync().unwrap_err()
    };
    // A reverse proxy now serves the server under another path only, and
    // answers 404 for everything else.
    assert_not_a_cairn_server(&sync_with(&format!("{}/nothing-here", srv.url)));
    assert_not_a_cairn_server(&sync_with(&another_web_server()));
    // A Cairn server that does not have the vault (reset, or another one).
    let other = server();
    let e = sync_with(&other.url);
    assert!(matches!(e, SyncError::VaultGone), "{e}");
}
