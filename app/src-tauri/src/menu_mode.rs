//! Keyboard menu mode on Windows, without the web view waiting for it.
//!
//! A plain Alt tap, F10, or Alt+Space then Esc puts a window in keyboard
//! menu mode. The page does not use these keys, so WebView2's browser
//! process hands them to `DefWindowProc` on its own window, which sends
//! them on to Cairn's windows: Alt and F10 as WM_SYSCOMMAND(SC_KEYMENU) to
//! the Tauri window, Alt+Space as WM_SYSCHAR to the web view's parent
//! window (WRY_WEBVIEW), which passes it up to the Tauri window. Those
//! sends cross into Cairn's process, and Windows runs the menu loop inside
//! them, on Cairn's main thread, until menu mode ends. All that time the
//! browser thread waited for the send to return: autosave, the IPC and any
//! script Cairn ran waited until the next key or click.
//!
//! [`install`] subclasses both windows. When one of these messages comes
//! from another thread (the browser's), Cairn answers it with
//! `ReplyMessage` first, which lets the browser go on, and then passes it
//! on unchanged, so the menu and its keys behave as before.
//!
//! The choice of messages is plain code, tested on every platform; the
//! Win32 part is Windows only.

/// WM_SYSCHAR, WM_SYSCOMMAND and SC_KEYMENU as Windows defines them.
pub const WM_SYSCHAR: u32 = 0x0106;
pub const WM_SYSCOMMAND: u32 = 0x0112;
pub const SC_KEYMENU: usize = 0xF100;

/// Whether a message with `msg` and `wparam` can start keyboard menu mode:
/// an Alt key combination's character, or the menu command itself. Windows
/// uses the four low bits of a WM_SYSCOMMAND's wParam for itself.
pub fn starts_menu_mode(msg: u32, wparam: usize) -> bool {
    msg == WM_SYSCHAR || (msg == WM_SYSCOMMAND && wparam & 0xFFF0 == SC_KEYMENU)
}

#[cfg(windows)]
pub use win::{hold_menu_keys, install};

#[cfg(windows)]
mod win {
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        ISMEX_REPLIED, ISMEX_SEND, InSendMessageEx, ReplyMessage, WM_NCDESTROY,
    };

    use super::starts_menu_mode;

    // The constants as Windows defines them.
    const _: () = {
        use windows::Win32::UI::WindowsAndMessaging as wm;
        assert!(super::WM_SYSCHAR == wm::WM_SYSCHAR);
        assert!(super::WM_SYSCOMMAND == wm::WM_SYSCOMMAND);
        assert!(super::SC_KEYMENU == wm::SC_KEYMENU as usize);
    };

    /// Our subclass of the windows.
    const SUBCLASS_ID: usize = 0x4d4d;

    thread_local! {
        /// Set while the end of the session waits for the page: a menu loop
        /// started then would run inside that wait and outlast it. The
        /// windows and that wait are on the main thread.
        static HOLD: Cell<bool> = const { Cell::new(false) };
    }

    /// Holds back the keys that start keyboard menu mode (`on`), or lets
    /// them through again, for the windows of this thread. Their sender is
    /// let go either way.
    pub fn hold_menu_keys(on: bool) {
        HOLD.set(on);
    }

    /// Answers menu keys sent to `windows` (the Tauri window and the web
    /// view's parent) from another thread before menu mode starts. Call it
    /// on the main thread, which owns them.
    pub fn install(windows: &[HWND]) {
        for &hwnd in windows {
            if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) }.as_bool() {
                log::warn!("menu keys: cannot watch window {hwnd:?}, so Alt and F10 can still hold up the web view");
            }
        }
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        _data: usize,
    ) -> LRESULT {
        if starts_menu_mode(msg, wparam.0) {
            // A panic must not leave a window procedure: the process would abort.
            let _ = catch_unwind(AssertUnwindSafe(release_sender));
            if HOLD.get() {
                log::info!("menu keys: held back while Cairn saves at the end of the session");
                return LRESULT(0);
            }
        }
        if msg == WM_NCDESTROY {
            let _removed = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), id) };
            #[cfg(test)]
            if _removed.as_bool() {
                tests::REMOVED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    /// Lets the thread that sent the message this thread is handling go on,
    /// when it is another thread's send that has no answer yet. Inside a
    /// message that a window of this thread passes on to another (the web
    /// view's parent to the Tauri window, `DefWindowProc` to itself), that
    /// is still the other thread's send. True when it replied.
    pub fn release_sender() -> bool {
        let flags = unsafe { InSendMessageEx(None) };
        if flags & (ISMEX_SEND | ISMEX_REPLIED) != ISMEX_SEND {
            return false;
        }
        #[cfg(test)]
        tests::REPLIES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // The sender (the browser's DefWindowProc) ignores the result.
        unsafe { ReplyMessage(LRESULT(0)) }.as_bool()
    }

    // These need Win32 windows and a second thread, so they run on Windows
    // only (CI's windows job runs cargo test -p cairn --lib).
    #[cfg(test)]
    pub(super) mod tests {
        use std::cell::{Cell, RefCell};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        use parking_lot::Mutex;
        use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EndMenu, GetForegroundWindow, KillTimer, MSG,
            MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RegisterClassExW,
            SMTO_NORMAL, SW_SHOW, SendMessageTimeoutW, SendMessageW, SetForegroundWindow, SetTimer, ShowWindow,
            TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ENTERMENULOOP, WM_EXITMENULOOP, WM_SYSCOMMAND, WM_TIMER,
            WNDCLASSEXW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_POPUP,
        };
        use windows::core::{PCWSTR, w};

        use super::{hold_menu_keys, install};
        use super::super::{SC_KEYMENU, WM_SYSCHAR};

        /// How often `release_sender` replied.
        pub static REPLIES: AtomicUsize = AtomicUsize::new(0);
        /// How often the subclass removed itself from a window going away.
        pub static REMOVED: AtomicUsize = AtomicUsize::new(0);
        /// The tests share REPLIES and window classes: one at a time.
        static SERIAL: Mutex<()> = Mutex::new(());
        /// Alt+Space as WM_SYSCHAR: the character, and the Alt bit in lParam.
        const ALT_SPACE: (usize, isize) = (b' ' as usize, 0x2000_0001);
        /// How long the stand-in for the menu loop runs.
        const MENU_LOOP: Duration = Duration::from_millis(1500);

        thread_local! {
            static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
            static IN_MENU_LOOP: Cell<bool> = const { Cell::new(false) };
        }

        fn note(s: String) {
            LOG.with(|l| l.borrow_mut().push(s));
        }

        fn take_log() -> Vec<String> {
            LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
        }

        /// Dispatches this thread's messages until `done` or `max`.
        fn pump_until(max: Duration, done: impl Fn() -> bool) {
            let end = Instant::now() + max;
            let mut m = MSG::default();
            while !done() && Instant::now() < end {
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(None, 10, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                    while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&m);
                        DispatchMessageW(&m);
                    }
                }
            }
        }

        /// The Tauri window's stand-in: on WM_SYSCOMMAND it runs a modal
        /// loop for `MENU_LOOP`, as Windows' menu loop would, and passes the
        /// rest to DefWindowProc, which turns Alt+Space into SC_KEYMENU.
        unsafe extern "system" fn top_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            if msg == WM_SYSCOMMAND {
                note(format!("menu loop for {:#x}", wparam.0));
                IN_MENU_LOOP.set(true);
                pump_until(MENU_LOOP, || false);
                IN_MENU_LOOP.set(false);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        /// WRY_WEBVIEW's stand-in: DefWindowProc only, which passes Alt+Space
        /// on to the parent.
        unsafe extern "system" fn child_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        /// A window that leaves menu keys to Windows' own menu loop, ended
        /// by a timer, and notes when the loop starts and ends.
        unsafe extern "system" fn real_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            match msg {
                WM_ENTERMENULOOP => note("entered the menu loop".into()),
                WM_EXITMENULOOP => note("left the menu loop".into()),
                WM_TIMER => unsafe {
                    let _ = KillTimer(Some(hwnd), wparam.0);
                    let _ = EndMenu();
                },
                _ => {}
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        fn window(
            class: PCWSTR,
            proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
            style: WINDOW_STYLE,
            parent: Option<HWND>,
        ) -> HWND {
            unsafe {
                let instance = GetModuleHandleW(PCWSTR::null()).unwrap().into();
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(proc),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                // Fails with ERROR_CLASS_ALREADY_EXISTS after the first test: the same class.
                RegisterClassExW(&wc);
                let (w, h) = if parent.is_some() { (0, 0) } else { (300, 200) };
                CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Cairn menu test"), style, 0, 0, w, h, parent, None, Some(instance), None)
                    .unwrap()
            }
        }

        /// The Tauri window and the web view's parent, as their stand-ins.
        struct Windows {
            top: HWND,
            child: HWND,
        }

        impl Drop for Windows {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.top);
                }
            }
        }

        fn stand_ins() -> Windows {
            take_log();
            REPLIES.store(0, Ordering::SeqCst);
            let top = window(w!("Cairn menu test top"), top_proc, WS_POPUP, None);
            let child = window(w!("Cairn menu test child"), child_proc, WS_CHILD, Some(top));
            Windows { top, child }
        }

        /// Sends `msg` to `hwnd` from another thread, as the browser does,
        /// while this thread dispatches messages. Returns how long the
        /// sender waited.
        fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> Duration {
            let hwnd = hwnd.0 as isize;
            let sender = std::thread::spawn(move || {
                let start = Instant::now();
                let sent =
                    unsafe { SendMessageTimeoutW(HWND(hwnd as _), msg, WPARAM(wparam), LPARAM(lparam), SMTO_NORMAL, 20_000, None) };
                assert_ne!(sent.0, 0, "SendMessageTimeoutW failed");
                start.elapsed()
            });
            pump_until(Duration::from_secs(25), || sender.is_finished());
            sender.join().unwrap()
        }

        /// The sender was let go before the menu loop ended.
        fn released(took: Duration) -> bool {
            took < Duration::from_millis(700)
        }

        #[test]
        fn alt_and_f10_let_the_sender_go_before_the_menu_loop() {
            let _serial = SERIAL.lock();
            let w = stand_ins();
            install(&[w.top, w.child]);
            let took = send(w.top, WM_SYSCOMMAND, SC_KEYMENU, 0);
            assert!(released(took), "the sender waited {took:?}");
            assert_eq!(take_log(), ["menu loop for 0xf100"]);
            assert_eq!(REPLIES.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn without_the_subclass_the_sender_waits_for_the_menu_loop() {
            let _serial = SERIAL.lock();
            let w = stand_ins();
            let took = send(w.top, WM_SYSCOMMAND, SC_KEYMENU, 0);
            assert!(took >= MENU_LOOP - Duration::from_millis(100), "the sender waited only {took:?}");
            assert_eq!(take_log(), ["menu loop for 0xf100"]);
        }

        #[test]
        fn alt_space_lets_the_sender_go_from_either_window() {
            let _serial = SERIAL.lock();
            // As WebView2 sends it: WM_SYSCHAR to the web view's parent, which
            // DefWindowProc passes up to the Tauri window, whose DefWindowProc
            // turns it into SC_KEYMENU with the space.
            let w = stand_ins();
            install(&[w.top, w.child]);
            let took = send(w.child, WM_SYSCHAR, ALT_SPACE.0, ALT_SPACE.1);
            assert!(released(took), "the sender waited {took:?}");
            assert_eq!(take_log(), ["menu loop for 0xf100"]);
            // Replied once, at the first window: the Tauri window then finds the send answered.
            assert_eq!(REPLIES.load(Ordering::SeqCst), 1);
            drop(w);
            // With the Tauri window watched alone, the message it gets from
            // the web view's parent on the same thread still belongs to the
            // browser's send.
            let w = stand_ins();
            install(&[w.top]);
            let took = send(w.child, WM_SYSCHAR, ALT_SPACE.0, ALT_SPACE.1);
            assert!(released(took), "the sender waited {took:?}");
            assert_eq!(take_log(), ["menu loop for 0xf100"]);
            assert_eq!(REPLIES.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn other_system_commands_and_sends_from_this_thread_are_left_alone() {
            let _serial = SERIAL.lock();
            let w = stand_ins();
            install(&[w.top, w.child]);
            // SC_MINIMIZE from another thread: the sender waits, as before.
            let took = send(w.top, WM_SYSCOMMAND, 0xF020, 0);
            assert!(took >= MENU_LOOP - Duration::from_millis(100), "the sender waited only {took:?}");
            // SC_KEYMENU sent on this thread: nothing to answer.
            unsafe { SendMessageW(w.top, WM_SYSCOMMAND, Some(WPARAM(SC_KEYMENU)), Some(LPARAM(0))) };
            assert_eq!(take_log(), ["menu loop for 0xf020", "menu loop for 0xf100"]);
            assert_eq!(REPLIES.load(Ordering::SeqCst), 0);
        }

        #[test]
        fn menu_keys_are_held_back_while_cairn_saves() {
            let _serial = SERIAL.lock();
            let w = stand_ins();
            install(&[w.top, w.child]);
            hold_menu_keys(true);
            let took = send(w.top, WM_SYSCOMMAND, SC_KEYMENU, 0);
            let took_space = send(w.child, WM_SYSCHAR, ALT_SPACE.0, ALT_SPACE.1);
            hold_menu_keys(false);
            assert!(released(took) && released(took_space), "the sender waited {took:?}, {took_space:?}");
            // No menu loop started, and the senders were let go.
            assert_eq!(take_log(), Vec::<String>::new());
            assert_eq!(REPLIES.load(Ordering::SeqCst), 2);
        }

        #[test]
        fn the_subclass_goes_with_the_window() {
            let _serial = SERIAL.lock();
            let w = stand_ins();
            install(&[w.top, w.child]);
            REMOVED.store(0, Ordering::SeqCst);
            // Destroying the top window destroys its child too.
            drop(w);
            assert_eq!(REMOVED.load(Ordering::SeqCst), 2);
        }

        #[test]
        fn alt_lets_the_sender_go_before_windows_menu_loop() {
            let _serial = SERIAL.lock();
            take_log();
            let top = window(w!("Cairn menu test real"), real_proc, WS_OVERLAPPEDWINDOW, None);
            unsafe {
                let _ = ShowWindow(top, SW_SHOW);
                let _ = SetForegroundWindow(top);
            }
            pump_until(Duration::from_millis(300), || false);
            if unsafe { GetForegroundWindow() } != top {
                eprintln!("skipped: the test window cannot come to the foreground here, so Windows starts no menu loop");
                unsafe { DestroyWindow(top) }.unwrap();
                return;
            }
            install(&[top]);
            REPLIES.store(0, Ordering::SeqCst);
            // EndMenu once the loop has run a while.
            unsafe { SetTimer(Some(top), 1, MENU_LOOP.as_millis() as u32, None) };
            let took = send(top, WM_SYSCOMMAND, SC_KEYMENU, 0);
            pump_until(MENU_LOOP * 2, || LOG.with(|l| l.borrow().iter().any(|s| s == "left the menu loop")));
            unsafe { DestroyWindow(top) }.unwrap();
            assert_eq!(take_log(), ["entered the menu loop", "left the menu loop"]);
            assert!(released(took), "the sender waited {took:?}");
            assert_eq!(REPLIES.load(Ordering::SeqCst), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_keys_are_alt_characters_and_the_menu_command() {
        assert!(starts_menu_mode(WM_SYSCHAR, b' ' as usize));
        assert!(starts_menu_mode(WM_SYSCHAR, b'f' as usize));
        assert!(starts_menu_mode(WM_SYSCOMMAND, SC_KEYMENU));
        // Windows keeps the low four bits of the command for itself.
        assert!(starts_menu_mode(WM_SYSCOMMAND, SC_KEYMENU | 0x3));
        // Close (Alt+F4), minimize and the mouse's menu are not.
        for other in [0xF060, 0xF020, 0xF090] {
            assert!(!starts_menu_mode(WM_SYSCOMMAND, other), "{other:#x}");
        }
        // A key press itself is not: it never reaches Cairn's windows by a send.
        assert!(!starts_menu_mode(0x0104, 0x12));
    }
}
