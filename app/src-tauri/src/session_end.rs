//! Windows ending the session: signing out, shutting down or restarting.
//!
//! Windows goes through the processes of the session in the order of their
//! shutdown level, highest first, and asks and ends each in turn. Every
//! process starts at level 0x280, WebView2's included, so Windows could ask
//! and end WebView2's processes before it asked Cairn, and the page, which
//! holds the text not saved yet, was gone by then. [`ask_cairn_first`] puts
//! Cairn at 0x3FF, the earliest level the range for apps allows.

/// Cairn's shutdown level: the highest of the range for apps (0x100 to
/// 0x3FF; Windows keeps the levels above for its own processes).
pub const SHUTDOWN_LEVEL: u32 = 0x3FF;

// Before the default level every process starts at, within the apps' range.
const _: () = assert!(SHUTDOWN_LEVEL > 0x280 && SHUTDOWN_LEVEL <= 0x3FF);

#[cfg(windows)]
pub use win::ask_cairn_first;

#[cfg(windows)]
mod win {
    use windows::Win32::System::Threading::SetProcessShutdownParameters;

    use super::SHUTDOWN_LEVEL;

    /// Makes Windows ask Cairn before the processes it started, WebView2's,
    /// when the session ends. With no flags Windows still shows its screen
    /// of apps that keep the session from ending.
    pub fn ask_cairn_first() {
        match unsafe { SetProcessShutdownParameters(SHUTDOWN_LEVEL, 0) } {
            Ok(()) => log::info!("session end: Windows asks Cairn at shutdown level {SHUTDOWN_LEVEL:#x}, before WebView2"),
            Err(e) => log::warn!("session end: cannot set the shutdown level, so WebView2 may end first: {e}"),
        }
    }

    #[cfg(test)]
    mod tests {
        use windows::Win32::System::Threading::GetProcessShutdownParameters;

        use super::*;

        #[test]
        fn cairn_is_asked_before_the_processes_it_starts() {
            ask_cairn_first();
            let (mut level, mut flags) = (0, 0);
            unsafe { GetProcessShutdownParameters(&mut level, &mut flags) }.unwrap();
            assert_eq!((level, flags), (SHUTDOWN_LEVEL, 0));
        }
    }
}
