//! Windows ending the session: signing out, shutting down or restarting,
//! or Restart Manager closing Cairn so that an installer can replace it.
//!
//! Windows goes through the processes of the session in the order of their
//! shutdown level, highest first, and asks and ends each in turn. Every
//! process starts at level 0x280, WebView2's included, so Windows could ask
//! and end WebView2's processes before it asked Cairn, and the page, which
//! holds the text not saved yet, was gone by then. [`ask_cairn_first`] puts
//! Cairn at 0x3FF, the earliest level the range for apps allows.
//!
//! The page keeps the backend up to date with what it has not saved
//! (held.rs), so that the backend can save it, or name it, without the
//! page. Windows asks every top-level window with WM_QUERYENDSESSION
//! whether the session may end, then gives the outcome with WM_ENDSESSION.
//! tao, the window library under Tauri, answers yes, and on
//! WM_ENDSESSION(TRUE) to its hidden "Tao Thread Event Target" window it
//! ends the process, after a teardown that waits for WebView2. [`install`]
//! subclasses the main window and tao's window:
//!
//! - At a sign-out, shutdown or restart, the main window's question asks
//!   the page for what it holds once more, for 3 seconds at most, then
//!   writes what is held and answers no when something is left unsaved,
//!   with a reason for Windows' screen that names the notes. tao's window
//!   always answers yes: one no keeps the session from ending.
//! - Restart Manager (an installer, which closes Cairn anyway; Cairn's own
//!   asks the user first) and a critical end, which gives a question one
//!   second, get yes at once. The writing then happens on
//!   WM_ENDSESSION(TRUE) to tao's window, just before tao ends the process:
//!   what is held goes to disk first, then the page is asked and its answer
//!   written. Less than 2 seconds after a question that ran out with no
//!   answer, Cairn waits for that answer once more instead of asking again.
//! - When the session goes on (Cairn said no, or WM_ENDSESSION(FALSE)), the
//!   page hears what the backend wrote and, after a no, what is left
//!   unsaved.
//!
//! While it waits for the page, Cairn dispatches the messages that bring
//! the page's answer, never input, and holds back the keys that start
//! keyboard menu mode, so that no menu loop starts inside the wait. The
//! page answers in milliseconds when it is idle, but the question waits
//! behind the keys still to be handled, and in a paragraph of a megabyte
//! each takes about a quarter of a second, in one of five megabytes more
//! than a second. A page that WebView2 reports gone is not waited for.
//!
//! This relies on two details of tao 0.37 that an update can change: the
//! class name of its window, and that it ends the process on
//! WM_ENDSESSION(TRUE). Without the window, Restart Manager and critical
//! ends write before their yes. The decisions are plain code, built and
//! tested on every platform; the Win32 part is Windows only. This file uses
//! no Tauri types: a Windows check can build it on its own.

/// Cairn's shutdown level: the highest of the range for apps (0x100 to
/// 0x3FF; Windows keeps the levels above for its own processes).
pub const SHUTDOWN_LEVEL: u32 = 0x3FF;

// Before the default level every process starts at, within the apps' range.
const _: () = assert!(SHUTDOWN_LEVEL > 0x280 && SHUTDOWN_LEVEL <= 0x3FF);

/// Flags of WM_QUERYENDSESSION and WM_ENDSESSION, in lParam: Restart
/// Manager closes the app, the ending is forced, the user signs out.
pub const ENDSESSION_CLOSEAPP: u32 = 0x0000_0001;
pub const ENDSESSION_CRITICAL: u32 = 0x4000_0000;
pub const ENDSESSION_LOGOFF: u32 = 0x8000_0000;

/// The longest reason Windows shows, in UTF-16 units (MAX_STR_BLOCKREASON
/// counts the final NUL too).
pub const MAX_REASON: usize = 255;

/// Whether a question with `flags` may be answered no: not from Restart
/// Manager (ENDSESSION_CLOSEAPP), which closes the app anyway, and not when
/// the ending is critical, which Windows does not wait for.
pub fn may_refuse(flags: u32) -> bool {
    flags & (ENDSESSION_CLOSEAPP | ENDSESSION_CRITICAL) == 0
}

/// What is ending the session, for the log.
pub fn kind(flags: u32) -> &'static str {
    if flags & ENDSESSION_CLOSEAPP != 0 {
        "Restart Manager closing Cairn (an installer or update)"
    } else if flags & ENDSESSION_CRITICAL != 0 {
        "a forced end of the session"
    } else if flags & ENDSESSION_LOGOFF != 0 {
        "signing out"
    } else {
        "shutting down or restarting"
    }
}

/// The reason Windows shows for Cairn on its screen of apps that keep the
/// session from ending. Names that do not fit are counted instead, and a
/// single name too long for the screen is cut, so the text stays within
/// [`MAX_REASON`].
pub fn block_reason(unsaved: &[String]) -> String {
    const START: &str = "Unsaved changes to ";
    const END: &str = ". Switch to Cairn to save or discard them.";
    let more = |n: usize| if n == 0 { String::new() } else { format!(" and {n} more") };
    for listed in (1..=unsaved.len()).rev() {
        let text = format!("{START}{}{}{END}", unsaved[..listed].join(", "), more(unsaved.len() - listed));
        if utf16_len(&text) <= MAX_REASON {
            return text;
        }
    }
    let Some(first) = unsaved.first() else { return format!("Unsaved changes{END}") };
    // A title in quotes keeps its closing quote after the cut.
    let (name, quote) = match first.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        Some(inner) => (inner, "\""),
        None => (first.as_str(), ""),
    };
    let rest = format!("…{quote}{}{END}", more(unsaved.len() - 1));
    let room = MAX_REASON.saturating_sub(utf16_len(START) + utf16_len(quote) + utf16_len(&rest));
    format!("{START}{quote}{}{rest}", cut(name, room).trim_end())
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The longest start of `s` that is at most `max` UTF-16 units, cut
/// between characters.
fn cut(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &s[..i];
        }
    }
    s
}

#[cfg(windows)]
pub use win::{Host, ask_cairn_first, install, page_gone, page_heard, settled, waiting_for};

#[cfg(windows)]
mod win {
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
    use windows::Win32::System::Threading::{GetCurrentThreadId, SetProcessShutdownParameters};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EndMenu, EnumThreadWindows, GUI_INMENUMODE, GUI_POPUPMENUMODE, GUI_SYSTEMMENUMODE,
        GUITHREADINFO, GetClassNameW, GetGUIThreadInfo, MSG, MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS,
        MsgWaitForMultipleObjectsEx, PEEK_MESSAGE_REMOVE_TYPE, PM_QS_PAINT, PM_QS_POSTMESSAGE, PM_QS_SENDMESSAGE,
        PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, QS_PAINT, QS_POSTMESSAGE, QS_SENDMESSAGE, QS_TIMER,
        TranslateMessage, WM_APP, WM_ENDSESSION, WM_NCDESTROY, WM_QUERYENDSESSION, WM_QUIT,
    };
    use windows::core::{BOOL, HSTRING};

    use super::{ENDSESSION_CLOSEAPP, ENDSESSION_CRITICAL, ENDSESSION_LOGOFF, SHUTDOWN_LEVEL, block_reason, kind, may_refuse};

    // The flags and the limit as Windows defines them.
    const _: () = {
        use windows::Win32::UI::WindowsAndMessaging as wm;
        assert!(ENDSESSION_CLOSEAPP == wm::ENDSESSION_CLOSEAPP);
        assert!(ENDSESSION_CRITICAL == wm::ENDSESSION_CRITICAL);
        assert!(ENDSESSION_LOGOFF == wm::ENDSESSION_LOGOFF);
        assert!(super::MAX_REASON + 1 == wm::MAX_STR_BLOCKREASON as usize);
    };

    /// What the end of the session needs from the app.
    pub trait Host {
        /// Whether the page has said what it holds since it loaded.
        fn listening(&self) -> bool;
        /// Asks the page to send what it holds, as its answer to `round`.
        /// False when the question could not be sent.
        fn ask(&self, round: u64) -> bool;
        /// Whether the page answered `round`.
        fn answered(&self, round: u64) -> bool;
        /// Writes what is held, and names what is left unsaved.
        fn write(&self) -> Vec<String>;
        /// Tells the page what was written and, after a no, what was not
        /// saved. Runs outside any message of the end of the session.
        fn tell(&self, refused: bool);
    }

    /// Shown while Cairn saves.
    const SAVING: &str = "Saving your notes…";
    /// How long the page has to answer: long enough for a page busy with
    /// the last key in a very large paragraph, and short of the 5 seconds
    /// after which Windows shows its screen of apps that keep the session
    /// from ending.
    const ASK: Duration = Duration::from_secs(3);
    /// One question to the page per attempt of Windows: a question this
    /// soon after the last one ended asks no more.
    const ASK_AGAIN: Duration = Duration::from_secs(2);
    /// How often a wait looks for an answer that came without a message.
    const TICK: Duration = Duration::from_millis(50);
    /// Posted to the main window to tell the page, outside the message
    /// that decided (wParam 1 after a no).
    const TELL: u32 = WM_APP + 0x3e5;

    /// The class of tao's hidden window (tao 0.37, create_event_target_window).
    const TAO_CLASS: &str = "Tao Thread Event Target";
    /// Our subclass of both windows; its data tells them apart.
    const SUBCLASS_ID: usize = 0x5345;
    const MAIN: usize = 0;
    const EVENT_TARGET: usize = 1;

    struct Installed {
        host: Box<dyn Host>,
        main: HWND,
        /// tao's window was found, so WM_ENDSESSION(TRUE) reaches us before
        /// the process ends.
        end_hook: bool,
    }

    thread_local! {
        /// Set by install on the main thread, where both windows get their
        /// messages. Cloned out before use: a wait dispatches messages, and
        /// those can reach the windows again.
        static INSTALLED: RefCell<Option<Rc<Installed>>> = const { RefCell::new(None) };
        /// Cairn is waiting for the page's answer.
        static ASKING: Cell<bool> = const { Cell::new(false) };
        /// The round asked last and when its wait ends, while it has no
        /// answer and its wait has not run out.
        static DUE: Cell<Option<(u64, Instant)>> = const { Cell::new(None) };
        /// When the last question ended: answered, or not in time.
        static LAST_DONE: Cell<Option<Instant>> = const { Cell::new(None) };
        static ROUND: Cell<u64> = const { Cell::new(0) };
        /// A no's reason is shown.
        static REFUSED: Cell<bool> = const { Cell::new(false) };
        /// The page heard of a no and holds something unsaved since.
        static TOLD: Cell<bool> = const { Cell::new(false) };
        /// The round whose wait ran out with no answer, until a newer
        /// question or its answer.
        static RAN_OUT: Cell<Option<u64>> = const { Cell::new(None) };
        /// WM_ENDSESSION(TRUE) is being handled: the session ends.
        static CLOSING: Cell<bool> = const { Cell::new(false) };
    }

    /// Marks WM_ENDSESSION(TRUE) as being handled until dropped, also when
    /// the handling panics.
    struct Closing;

    impl Closing {
        fn start() -> Closing {
            CLOSING.set(true);
            Closing
        }
    }

    impl Drop for Closing {
        fn drop(&mut self) {
            CLOSING.set(false);
        }
    }

    /// WebView2 reported the page's process or its browser process gone:
    /// the page answers nothing until it loads again. WebView2 reports it
    /// on the main thread; an atomic keeps that from mattering.
    static PAGE_GONE: AtomicBool = AtomicBool::new(false);

    fn installed() -> Option<Rc<Installed>> {
        INSTALLED.with(|i| i.borrow().clone())
    }

    /// The page is gone (webview2.rs): the end of the session does not wait
    /// for its answer.
    pub fn page_gone() {
        PAGE_GONE.store(true, Ordering::Relaxed);
    }

    /// The page sent a request, so it is there again.
    pub fn page_heard() {
        PAGE_GONE.store(false, Ordering::Relaxed);
    }

    /// The round whose answer Cairn is waiting for right now, if any.
    pub fn waiting_for() -> Option<u64> {
        if ASKING.get() { DUE.get().map(|(round, _)| round) } else { None }
    }

    /// Makes Windows ask Cairn before the processes it started, WebView2's,
    /// when the session ends. With no flags Windows still shows its screen
    /// of apps that keep the session from ending.
    pub fn ask_cairn_first() {
        match unsafe { SetProcessShutdownParameters(SHUTDOWN_LEVEL, 0) } {
            Ok(()) => log::info!("session end: Windows asks Cairn at shutdown level {SHUTDOWN_LEVEL:#x}, before WebView2"),
            Err(e) => log::warn!("session end: cannot set the shutdown level, so WebView2 may end first: {e}"),
        }
    }

    /// Watches for the end of the session on `main`, the app's window, and
    /// on tao's event-target window. Call it on the main thread once both
    /// exist (Tauri's setup).
    pub fn install(main: HWND, host: impl Host + 'static) {
        install_with(main, Box::new(host));
    }

    fn install_with(main: HWND, host: Box<dyn Host>) -> bool {
        let target = event_target();
        INSTALLED.with(|i| *i.borrow_mut() = Some(Rc::new(Installed { host, main, end_hook: target.is_some() })));
        for (hwnd, which) in [(Some(main), MAIN), (target, EVENT_TARGET)] {
            let Some(hwnd) = hwnd else { continue };
            if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, which) }.as_bool() {
                log::warn!("session end: cannot watch window {hwnd:?}");
            }
        }
        match target {
            Some(_) => log::info!("session end: watching the main window and tao's event-target window"),
            None => log::warn!(
                "session end: tao's \"{TAO_CLASS}\" window was not found, so Restart Manager and forced ends save before \
                 they are answered"
            ),
        }
        target.is_some()
    }

    /// tao's hidden window on this thread.
    fn event_target() -> Option<HWND> {
        unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
            let mut class = [0u16; 64];
            let len = unsafe { GetClassNameW(hwnd, &mut class) }.clamp(0, 64) as usize;
            if String::from_utf16_lossy(&class[..len]) != TAO_CLASS {
                return BOOL(1);
            }
            // `found` points to the Option in event_target, which outlives the enumeration.
            unsafe { *(found.0 as *mut Option<HWND>) = Some(hwnd) };
            BOOL(0)
        }
        let mut found: Option<HWND> = None;
        let _ = unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(each), LPARAM(&raw mut found as isize)) };
        found
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        id: usize,
        which: usize,
    ) -> LRESULT {
        let flags = lparam.0 as u32;
        // A message that a wait dispatched: the wait goes on after it.
        let inside_wait = ASKING.get();
        // A panic must not leave a window procedure: the process would abort.
        let handled = catch_unwind(AssertUnwindSafe(|| match msg {
            WM_QUERYENDSESSION if which == MAIN => Some(LRESULT(question(flags) as isize)),
            WM_QUERYENDSESSION => {
                // Only the main window says no; one no is enough.
                Some(LRESULT(1))
            }
            WM_ENDSESSION if wparam.0 == 0 => {
                cancelled(which);
                None
            }
            WM_ENDSESSION if which == EVENT_TARGET => {
                closing(flags);
                None
            }
            TELL if which == MAIN => {
                // While the session ends, the page need not hear of a no, and
                // sync need not start.
                if !CLOSING.get()
                    && let Some(inst) = installed()
                {
                    inst.host.tell(wparam.0 != 0);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }));
        if msg == WM_NCDESTROY {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), id) };
        }
        match handled {
            Ok(Some(result)) => result,
            Ok(None) => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
            Err(_) => {
                log::error!("session end: panicked on message {msg:#x}");
                if inside_wait {
                    // The wait goes on: it still holds the menu keys back.
                    ASKING.set(true);
                    crate::menu_mode::hold_menu_keys(true);
                } else {
                    ASKING.set(false);
                    crate::menu_mode::hold_menu_keys(false);
                    if let Some(inst) = installed() {
                        set_reason(inst.main, None);
                    }
                    REFUSED.set(false);
                }
                // Never keep the session from ending by mistake.
                if msg == WM_QUERYENDSESSION { LRESULT(1) } else { unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) } }
            }
        }
    }

    /// WM_QUERYENDSESSION to the main window: whether the session may end.
    fn question(flags: u32) -> bool {
        let Some(inst) = installed() else { return true };
        let what = kind(flags);
        if !may_refuse(flags) {
            if !inst.end_hook {
                let unsaved = inst.host.write();
                log::info!("session end: {what} (flags {flags:#x}): saved what it could, not saved: {}", list(&unsaved));
            }
            log::info!("session end: {what} (flags {flags:#x}): yes at once, the saving comes before the end");
            return true;
        }
        log::info!("session end: {what} (flags {flags:#x})");
        leave_menu_mode();
        set_reason(inst.main, Some(SAVING));
        ask(&inst);
        let unsaved = inst.host.write();
        if unsaved.is_empty() {
            set_reason(inst.main, None);
            REFUSED.set(false);
            log::info!("session end: {what}: everything is saved: yes");
            true
        } else {
            set_reason(inst.main, Some(&block_reason(&unsaved)));
            REFUSED.set(true);
            TOLD.set(true);
            post_tell(inst.main, true);
            log::info!("session end: {what}: not saved: {}: no", list(&unsaved));
            false
        }
    }

    /// WM_ENDSESSION(TRUE) to tao's window, after which tao ends the process
    /// (its teardown waits for WebView2): the last chance to save. A forced
    /// end can end Cairn sooner,
    /// so what is held goes to disk before the page is asked; its answer,
    /// with what was typed since the page last handed its text over, is
    /// written after.
    fn closing(flags: u32) {
        let Some(inst) = installed() else { return };
        let _closing = Closing::start();
        inst.host.write();
        leave_menu_mode();
        // Whether a question had run out before this message came, not one
        // that runs out while this message waits.
        let ran_out = RAN_OUT.get();
        if !ask(&inst) {
            wait_once_more(&inst, ran_out);
        }
        // Each end waits once more at most.
        RAN_OUT.set(None);
        let unsaved = inst.host.write();
        set_reason(inst.main, None);
        REFUSED.set(false);
        TOLD.set(false);
        if unsaved.is_empty() {
            log::info!("session end: {} (flags {flags:#x}) goes ahead, everything is saved", kind(flags));
        } else {
            log::warn!("session end: {} (flags {flags:#x}) goes ahead, not saved: {}", kind(flags), list(&unsaved));
        }
    }

    /// WM_ENDSESSION(FALSE) to `which` window: the session goes on. Windows
    /// sends it to each window it asked.
    fn cancelled(which: usize) {
        let Some(inst) = installed() else { return };
        if REFUSED.replace(false) {
            set_reason(inst.main, None);
        }
        log::info!("session end: cancelled ({})", if which == MAIN { "main window" } else { "tao's window" });
        post_tell(inst.main, false);
    }

    /// A no is no longer needed: the page holds nothing unsaved.
    pub fn settled() {
        let Some(inst) = installed() else { return };
        if REFUSED.replace(false) {
            set_reason(inst.main, None);
        }
        if TOLD.replace(false) {
            #[cfg(test)]
            tests::note("settled".into());
            log::info!("session end: nothing is left unsaved");
        }
    }

    /// Asks the page once per attempt of Windows, and waits for its answer
    /// for [`ASK`] at most. An answer still due, to a question that this
    /// message cut into, is waited for without a new question. True when
    /// the page answered.
    fn ask(inst: &Installed) -> bool {
        let now = Instant::now();
        if PAGE_GONE.load(Ordering::Relaxed) {
            log::info!("session end: WebView2 reported the page gone, so Cairn does not ask it");
            return false;
        }
        if !inst.host.listening() {
            return false;
        }
        if let Some((round, until)) = DUE.get() {
            if now < until {
                log::info!("session end: waiting for the page's answer to round {round}");
                return wait(inst, round, until);
            }
            DUE.set(None);
        }
        if let Some(done) = LAST_DONE.get().filter(|t| now.duration_since(*t) < ASK_AGAIN) {
            log::info!("session end: the page was asked {} ms ago, not again", now.duration_since(done).as_millis());
            return false;
        }
        let round = ROUND.get() + 1;
        ROUND.set(round);
        RAN_OUT.set(None);
        if !catch_unwind(AssertUnwindSafe(|| inst.host.ask(round))).unwrap_or(false) {
            log::warn!("session end: could not ask the page (round {round})");
            LAST_DONE.set(Some(now));
            return false;
        }
        let until = now + ASK;
        DUE.set(Some((round, until)));
        wait(inst, round, until)
    }

    /// Waits for the page's answer to `round` until `until`, [`ASK`] after
    /// it was asked, with the menu keys held back. True when it answered.
    fn wait(inst: &Installed, round: u64, until: Instant) -> bool {
        let since = until - ASK;
        let gone = || PAGE_GONE.load(Ordering::Relaxed);
        let outer = ASKING.replace(true);
        crate::menu_mode::hold_menu_keys(true);
        // WebView2 can report the page gone during the wait: nothing answers then.
        pump(until, || inst.host.answered(round) || gone());
        let answered = inst.host.answered(round);
        crate::menu_mode::hold_menu_keys(outer);
        ASKING.set(outer);
        // A wait inside this one (a message it dispatched) may have ended
        // the question already.
        if DUE.get().is_some_and(|(r, _)| r == round) && (answered || gone() || Instant::now() >= until) {
            DUE.set(None);
            LAST_DONE.set(Some(Instant::now()));
            let ms = since.elapsed().as_millis();
            if answered {
                RAN_OUT.set(None);
                log::info!("session end: the page answered round {round} after {ms} ms");
            } else if gone() {
                log::info!("session end: WebView2 reported the page gone {ms} ms into round {round}, so Cairn stops waiting");
            } else {
                RAN_OUT.set(Some(round));
                log::info!("session end: no answer from the page to round {round} after {ms} ms");
            }
        }
        answered
    }

    /// At WM_ENDSESSION(TRUE) less than [`ASK_AGAIN`] after a question whose
    /// wait had run out with no answer (`ran_out`, the round, as it was when
    /// the message came), so that ask() did not ask again: the page may still
    /// be busy with the keys before it (in a very large paragraph, a second
    /// or more each), so Cairn waits for that same answer once more, for
    /// [`ASK`] at most. Not for a question that was still waiting when the
    /// message came, nor for one that the message asked itself.
    fn wait_once_more(inst: &Installed, ran_out: Option<u64>) {
        let round = ROUND.get();
        if ran_out != Some(round) || PAGE_GONE.load(Ordering::Relaxed) || inst.host.answered(round) {
            return;
        }
        // A message inside this wait does not wait once more again.
        RAN_OUT.set(None);
        log::info!("session end: waiting once more for the page's answer to round {round}");
        let until = Instant::now() + ASK;
        DUE.set(Some((round, until)));
        wait(inst, round, until);
    }

    /// Takes sent, posted, timer and paint messages from the queue, never
    /// input.
    fn no_input() -> PEEK_MESSAGE_REMOVE_TYPE {
        PEEK_MESSAGE_REMOVE_TYPE(PM_REMOVE.0 | PM_QS_SENDMESSAGE.0 | PM_QS_POSTMESSAGE.0 | PM_QS_PAINT.0)
    }

    /// Dispatches this thread's sent, posted, timer and paint messages, but
    /// no input, until `done` or `deadline`; true when done. The page's
    /// answer comes as such messages from WebView2.
    fn pump(deadline: Instant, done: impl Fn() -> bool) -> bool {
        let mut msg = MSG::default();
        let filter = no_input();
        loop {
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, filter) }.as_bool() {
                if msg.message == WM_QUIT {
                    // For the main loop, which ends on it.
                    unsafe { PostQuitMessage(msg.wParam.0 as i32) };
                    return done();
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if done() || Instant::now() >= deadline {
                    return done();
                }
            }
            if done() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let ms = (deadline - now).min(TICK).as_millis() as u32;
            let wake = QS_SENDMESSAGE | QS_POSTMESSAGE | QS_TIMER | QS_PAINT;
            let _ = unsafe { MsgWaitForMultipleObjectsEx(None, ms, wake, MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(0)) };
        }
    }

    /// Ends keyboard menu mode, which an Alt tap or F10 leaves the window
    /// in, so that the page and its keys work again once the answer is
    /// given (the menu loop ends when this message returns).
    fn leave_menu_mode() {
        let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut info) }.is_ok()
            && (info.flags & (GUI_INMENUMODE | GUI_POPUPMENUMODE | GUI_SYSTEMMENUMODE)).0 != 0
        {
            log::info!("session end: leaving keyboard menu mode");
            let _ = unsafe { EndMenu() };
        }
    }

    fn post_tell(main: HWND, refused: bool) {
        if let Err(e) = unsafe { PostMessageW(Some(main), TELL, WPARAM(refused as usize), LPARAM(0)) } {
            log::warn!("session end: cannot tell the page: {e}");
        }
    }

    /// Shows `reason` for `main` on Windows' screen of apps that keep the
    /// session from ending, or removes it. Only the window's own thread may.
    fn set_reason(main: HWND, reason: Option<&str>) {
        #[cfg(test)]
        tests::note(format!("reason: {}", reason.unwrap_or("none")));
        match reason {
            Some(text) => {
                if let Err(e) = unsafe { ShutdownBlockReasonCreate(main, &HSTRING::from(text)) } {
                    log::warn!("session end: cannot show the reason {text:?}: {e}");
                }
            }
            None => {
                // Fails when there was none.
                let _ = unsafe { ShutdownBlockReasonDestroy(main) };
            }
        }
    }

    fn list(names: &[String]) -> String {
        if names.is_empty() { "nothing".into() } else { names.join(", ") }
    }

    // These need Win32 windows and a thread message queue, so they run on
    // Windows only (CI's windows job runs cargo test -p cairn --lib).
    #[cfg(test)]
    pub(super) mod tests {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

        use parking_lot::Mutex;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::System::Shutdown::ShutdownBlockReasonQuery;
        use windows::Win32::System::Threading::GetProcessShutdownParameters;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, SetFocus, VK_F13,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, MWMO_INPUTAVAILABLE, QS_ALLINPUT,
            RegisterClassExW, SMTO_NORMAL, SW_SHOW, SWP_NOZORDER, SendMessageTimeoutW, SetForegroundWindow, SetWindowPos,
            ShowWindow, WM_APP, WM_KEYDOWN, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
        };
        use windows::core::{PCWSTR, PWSTR, w};

        use super::*;

        /// The tests share the thread's state: one at a time.
        static SERIAL: Mutex<()> = Mutex::new(());

        thread_local! {
            /// What happened, in order, on the test's thread.
            static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        }

        pub fn note(s: String) {
            LOG.with(|l| l.borrow_mut().push(s));
        }

        fn take_log() -> Vec<String> {
            LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
        }

        /// The test windows' own procedure: what reaches it is logged.
        unsafe extern "system" fn original(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            match msg {
                WM_QUERYENDSESSION => {
                    note("question reached the window".into());
                    LRESULT(1)
                }
                WM_ENDSESSION => {
                    note(format!("window got WM_ENDSESSION({})", wparam.0));
                    LRESULT(0)
                }
                WM_APP => {
                    note("posted message".into());
                    LRESULT(0)
                }
                ANSWER => {
                    POSTED_ANSWER.store(wparam.0 as u64, Ordering::SeqCst);
                    LRESULT(0)
                }
                WM_KEYDOWN => {
                    note("key".into());
                    LRESULT(0)
                }
                _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }

        /// The page's answer as a message posted to the main window, as
        /// WebView2 hands it over: wParam is the round.
        const ANSWER: u32 = WM_APP + 1;
        static POSTED_ANSWER: AtomicU64 = AtomicU64::new(0);

        /// A hidden top-level window of class `class` on this thread.
        fn window(class: PCWSTR) -> HWND {
            unsafe {
                let instance = GetModuleHandleW(PCWSTR::null()).unwrap().into();
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(original),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                // Fails with ERROR_CLASS_ALREADY_EXISTS after the first test: the same class.
                RegisterClassExW(&wc);
                CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("Cairn test"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None)
                    .unwrap()
            }
        }

        /// The app: what it holds and how the page answers.
        #[derive(Default)]
        struct FakeHost {
            /// The page listens.
            listening: bool,
            /// The page answers a question after 30 ms (from another thread).
            answers: bool,
            /// How long it takes to answer, if not 30 ms.
            answer_after: Option<Duration>,
            answered: Arc<AtomicU64>,
            /// Gets WM_ENDSESSION(TRUE) from another thread 50 ms after the
            /// question, as when Windows goes ahead while Cairn waits.
            end_to: Option<isize>,
            /// The page answers 100 ms after the write with this number (1
            /// for the first) instead of after a time, whatever the threads'
            /// timing.
            answer_on_write: Option<usize>,
            /// WebView2 reports the page gone this long after the question.
            gone_after: Option<Duration>,
            /// The answer to round 1 is posted to this window during the
            /// write with this number (1 for the first), as messages on
            /// their way when Windows goes ahead.
            post_answer: Option<(isize, usize)>,
            /// Cairn's own news of a no is posted to this window during the
            /// write with this number, as a message still queued then.
            tell_on_write: Option<(isize, usize)>,
            writes: Arc<AtomicUsize>,
            /// The first write panics.
            panic_once: Cell<bool>,
            /// Notes the first time the page's answer is checked while
            /// Cairn does not count itself as waiting.
            watch_waiting: bool,
            noted: Cell<bool>,
            /// What `write` names, before and after the page's answer.
            unsaved: Vec<String>,
            unsaved_after_answer: Option<Vec<String>>,
            /// Gets a posted message with the question.
            post_to: Option<isize>,
            /// Types a key with the question, as the user might.
            types: bool,
            panics: bool,
        }

        impl Host for FakeHost {
            fn listening(&self) -> bool {
                self.listening
            }
            fn ask(&self, round: u64) -> bool {
                note(format!("ask {round}"));
                if let Some(hwnd) = self.post_to {
                    unsafe { PostMessageW(Some(HWND(hwnd as _)), WM_APP, WPARAM(0), LPARAM(0)) }.unwrap();
                }
                if self.types {
                    let key = |flags| INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_F13, dwFlags: flags, ..Default::default() } },
                    };
                    let keys = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
                    assert_eq!(unsafe { SendInput(&keys, size_of::<INPUT>() as i32) }, 2);
                }
                if let Some(hwnd) = self.end_to {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(50));
                        let lparam = LPARAM(ENDSESSION_LOGOFF as isize);
                        unsafe { SendMessageTimeoutW(HWND(hwnd as _), WM_ENDSESSION, WPARAM(1), lparam, SMTO_NORMAL, 20_000, None) };
                    });
                }
                if let Some(after) = self.gone_after {
                    std::thread::spawn(move || {
                        std::thread::sleep(after);
                        page_gone();
                    });
                }
                if self.answers {
                    let answered = self.answered.clone();
                    let after = self.answer_after.unwrap_or(Duration::from_millis(30));
                    let on_write = self.answer_on_write.map(|n| (n, self.writes.clone()));
                    std::thread::spawn(move || {
                        if let Some((n, writes)) = on_write {
                            let give_up = Instant::now() + Duration::from_secs(10);
                            while writes.load(Ordering::SeqCst) < n && Instant::now() < give_up {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        } else {
                            std::thread::sleep(after);
                        }
                        answered.store(round, Ordering::SeqCst);
                    });
                }
                true
            }
            fn answered(&self, round: u64) -> bool {
                if self.watch_waiting && waiting_for().is_none() && !self.noted.replace(true) {
                    note("not waiting".into());
                }
                self.answered.load(Ordering::SeqCst) == round || POSTED_ANSWER.load(Ordering::SeqCst) == round
            }
            fn write(&self) -> Vec<String> {
                assert!(!self.panics, "a write that panics");
                let nth = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
                assert!(!self.panic_once.replace(false), "a first write that panics");
                if let Some((hwnd, n)) = self.post_answer
                    && n == nth
                {
                    unsafe { PostMessageW(Some(HWND(hwnd as _)), ANSWER, WPARAM(1), LPARAM(0)) }.unwrap();
                }
                if let Some((hwnd, n)) = self.tell_on_write
                    && n == nth
                {
                    unsafe { PostMessageW(Some(HWND(hwnd as _)), TELL, WPARAM(1), LPARAM(0)) }.unwrap();
                }
                let answered = self.answered.load(Ordering::SeqCst) != 0 || POSTED_ANSWER.load(Ordering::SeqCst) != 0;
                note(format!("write{}", if answered { " after the answer" } else { "" }));
                match (&self.unsaved_after_answer, answered) {
                    (Some(after), true) => after.clone(),
                    _ => self.unsaved.clone(),
                }
            }
            fn tell(&self, refused: bool) {
                note(format!("tell{}", if refused { " refused" } else { "" }));
            }
        }

        fn listening() -> FakeHost {
            FakeHost { listening: true, answers: true, ..FakeHost::default() }
        }

        /// The main window and tao's, destroyed when dropped.
        struct Windows {
            main: HWND,
            target: HWND,
        }

        impl Drop for Windows {
            fn drop(&mut self) {
                unsafe {
                    let _ = ShutdownBlockReasonDestroy(self.main);
                    let _ = DestroyWindow(self.main);
                    let _ = DestroyWindow(self.target);
                }
            }
        }

        /// Forgets what earlier tests on this thread left.
        fn fresh() {
            take_log();
            for c in [&ASKING, &REFUSED, &TOLD, &CLOSING] {
                c.set(false);
            }
            DUE.set(None);
            LAST_DONE.set(None);
            RAN_OUT.set(None);
            ROUND.set(0);
            POSTED_ANSWER.store(0, Ordering::SeqCst);
            page_heard();
        }

        fn watched(host: impl FnOnce(&Windows) -> FakeHost) -> Windows {
            fresh();
            let w = Windows { main: window(w!("Cairn session end test")), target: window(w!("Tao Thread Event Target")) };
            assert_eq!(event_target(), Some(w.target));
            assert!(install_with(w.main, Box::new(host(&w))));
            w
        }

        /// Sends `msg` to `hwnd` from another thread, as Windows does, while
        /// this thread dispatches messages. Returns the result and how long
        /// the sender waited.
        fn send(hwnd: HWND, msg: u32, wparam: usize, flags: u32) -> (usize, Duration) {
            let hwnd = hwnd.0 as isize;
            let sender = std::thread::spawn(move || {
                let mut result = 0;
                let start = Instant::now();
                let lparam = LPARAM(flags as isize);
                let sent = unsafe {
                    SendMessageTimeoutW(HWND(hwnd as _), msg, WPARAM(wparam), lparam, SMTO_NORMAL, 20_000, Some(&raw mut result))
                };
                assert_ne!(sent.0, 0, "SendMessageTimeoutW failed");
                (result, start.elapsed())
            });
            drain(|| sender.is_finished());
            sender.join().unwrap()
        }

        /// Dispatches this thread's messages until `done`.
        fn drain(done: impl Fn() -> bool) {
            let mut m = MSG::default();
            let end = Instant::now() + Duration::from_secs(25);
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

        /// The reason Windows has for `main`, if any.
        fn reason(main: HWND) -> Option<String> {
            let mut buf = [0u16; 300];
            let mut len = buf.len() as u32;
            unsafe { ShutdownBlockReasonQuery(main, Some(PWSTR(buf.as_mut_ptr())), &mut len) }.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]).trim_end_matches('\0').to_string())
        }

        #[test]
        fn cairn_is_asked_before_the_processes_it_starts() {
            ask_cairn_first();
            let (mut level, mut flags) = (0, 0);
            unsafe { GetProcessShutdownParameters(&mut level, &mut flags) }.unwrap();
            assert_eq!((level, flags), (SHUTDOWN_LEVEL, 0));
        }

        #[test]
        fn says_yes_once_everything_is_written() {
            let _serial = SERIAL.lock();
            let w = watched(|_| listening());
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write after the answer", "reason: none"]);
            assert_eq!(reason(w.main), None);
        }

        #[test]
        fn says_no_with_a_reason_that_names_what_is_left_unsaved() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { unsaved: vec!["\"T\"".into()], ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 0);
            assert_eq!(reason(w.main).as_deref(), Some("Unsaved changes to \"T\". Switch to Cairn to save or discard them."));
            // Windows asks tao's window next: yes, with nothing more done.
            assert_eq!(send(w.target, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            // The page hears of it once the question is answered.
            drain(|| LOG.with(|l| l.borrow().iter().any(|s| s == "tell refused")));
            let reason_text = "reason: Unsaved changes to \"T\". Switch to Cairn to save or discard them.";
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write after the answer", reason_text, "tell refused"]);
            // The user cancels the sign-out: the reason goes, and the page hears again.
            assert_eq!(send(w.main, WM_ENDSESSION, 0, ENDSESSION_LOGOFF).0, 0);
            drain(|| LOG.with(|l| l.borrow().iter().any(|s| s == "tell")));
            assert_eq!(take_log(), ["reason: none", "window got WM_ENDSESSION(0)", "tell"]);
            assert_eq!(reason(w.main), None);
        }

        #[test]
        fn the_pages_answer_counts() {
            let _serial = SERIAL.lock();
            // Held text the page then saves, or a note it can no longer save: what
            // the write finds after the answer decides.
            let w = watched(|_| FakeHost { unsaved: vec!["\"T\"".into()], unsaved_after_answer: Some(vec![]), ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 1);
            drop(w);
            let w = watched(|_| FakeHost { unsaved_after_answer: Some(vec!["\"T\"".into()]), ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 0);
            drop(w);
        }

        #[test]
        fn restart_manager_and_a_forced_end_get_yes_at_once_and_save_before_the_end() {
            let _serial = SERIAL.lock();
            for flags in [ENDSESSION_CLOSEAPP, ENDSESSION_CRITICAL | ENDSESSION_LOGOFF] {
                let w = watched(|_| FakeHost { unsaved: vec!["\"T\"".into()], ..listening() });
                for hwnd in [w.main, w.target] {
                    let (result, took) = send(hwnd, WM_QUERYENDSESSION, 0, flags);
                    assert_eq!(result, 1, "flags {flags:#x}");
                    assert!(took < Duration::from_millis(200), "flags {flags:#x}: {took:?}");
                }
                assert_eq!(take_log(), Vec::<String>::new(), "flags {flags:#x}");
                // WM_ENDSESSION(TRUE) to tao's window writes before tao ends the process:
                // what is held first, then the page's answer.
                assert_eq!(send(w.target, WM_ENDSESSION, 1, flags).0, 0, "flags {flags:#x}");
                let want = ["write", "ask 1", "write after the answer", "reason: none", "window got WM_ENDSESSION(1)"];
                assert_eq!(take_log(), want, "flags {flags:#x}");
            }
        }

        #[test]
        fn a_page_that_does_not_answer_holds_nothing_up_for_long() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { answers: false, ..listening() });
            let (result, took) = send(w.main, WM_QUERYENDSESSION, 0, 0);
            assert_eq!(result, 1);
            assert!(took >= ASK && took < ASK + Duration::from_secs(2), "{took:?}");
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write", "reason: none"]);
            // Windows goes ahead: the page is not asked again, but Cairn waits once
            // more for the answer, and no longer.
            let (_, took) = send(w.target, WM_ENDSESSION, 1, 0);
            assert!(took >= ASK && took < ASK + Duration::from_secs(2), "{took:?}");
            assert_eq!(take_log(), ["write", "write", "reason: none", "window got WM_ENDSESSION(1)"]);
            // A later end does not wait again.
            let (_, took) = send(w.target, WM_ENDSESSION, 1, 0);
            assert!(took < Duration::from_millis(500), "{took:?}");
        }

        #[test]
        fn an_answer_later_than_the_wait_is_waited_for_once_more_when_windows_goes_ahead() {
            let _serial = SERIAL.lock();
            // The page answers 3.6 s after the question: too late for it, so Cairn
            // says no; Windows goes ahead at once, as after Sign out anyway.
            // The answer comes once Windows has gone ahead (100 ms after its first
            // write), when news of the no is still queued.
            let w = watched(|w| FakeHost {
                answer_on_write: Some(2),
                tell_on_write: Some((w.main.0 as isize, 2)),
                unsaved: vec!["\"Big\"".into()],
                unsaved_after_answer: Some(vec![]),
                ..listening()
            });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 0);
            take_log();
            let (result, took) = send(w.target, WM_ENDSESSION, 1, ENDSESSION_LOGOFF);
            assert_eq!(result, 0);
            assert!(took < ASK, "{took:?}");
            // The news is not told: the session ends.
            assert_eq!(take_log(), ["write", "write after the answer", "reason: none", "window got WM_ENDSESSION(1)"]);
        }

        #[test]
        fn an_end_during_a_wait_that_runs_out_does_not_wait_twice() {
            let _serial = SERIAL.lock();
            // Windows goes ahead 50 ms into the question's wait, and the page never
            // answers: the end waits out that question only.
            let w = watched(|w| FakeHost { answers: false, end_to: Some(w.target.0 as isize), ..listening() });
            let (result, took) = send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF);
            assert_eq!(result, 1);
            assert!(took < ASK + Duration::from_secs(1), "{took:?}");
            assert_eq!(
                take_log(),
                [
                    "reason: Saving your notes…",
                    "ask 1",
                    "write",
                    "write",
                    "reason: none",
                    "window got WM_ENDSESSION(1)",
                    "write",
                    "reason: none"
                ]
            );
        }

        #[test]
        fn a_forced_end_waits_once_for_a_page_that_does_not_answer() {
            let _serial = SERIAL.lock();
            // The question at the end itself runs out: no second wait.
            let w = watched(|_| FakeHost { answers: false, ..listening() });
            let flags = ENDSESSION_CRITICAL | ENDSESSION_LOGOFF;
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, flags).0, 1);
            let (_, took) = send(w.target, WM_ENDSESSION, 1, flags);
            assert!(took >= ASK && took < ASK + Duration::from_secs(2), "{took:?}");
            assert_eq!(take_log(), ["write", "ask 1", "write", "reason: none", "window got WM_ENDSESSION(1)"]);
        }

        #[test]
        fn a_late_answer_counts() {
            let _serial = SERIAL.lock();
            // A note whose text only the page has: the page, busy for a while,
            // answers within the wait, and the note is written.
            let w = watched(|_| FakeHost {
                answer_after: Some(Duration::from_millis(900)),
                unsaved: vec!["\"Big\"".into()],
                unsaved_after_answer: Some(vec![]),
                ..listening()
            });
            let (result, took) = send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF);
            assert_eq!(result, 1);
            assert!(took >= Duration::from_millis(900) && took < ASK, "{took:?}");
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write after the answer", "reason: none"]);
        }

        #[test]
        fn an_end_during_the_wait_waits_for_the_same_answer() {
            let _serial = SERIAL.lock();
            // Windows goes ahead 50 ms into the wait; the page answers once that has
            // written what is held.
            let w = watched(|w| FakeHost { end_to: Some(w.target.0 as isize), answer_on_write: Some(1), ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(
                take_log(),
                [
                    "reason: Saving your notes…",
                    "ask 1",
                    // WM_ENDSESSION(TRUE) inside the wait: what is held, then the answer, with no
                    // second question.
                    "write",
                    "write after the answer",
                    "reason: none",
                    "window got WM_ENDSESSION(1)",
                    // The question, once the answer is in.
                    "write after the answer",
                    "reason: none",
                ]
            );
        }

        #[test]
        fn a_page_that_goes_during_the_wait_is_waited_for_no_more() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { answers: false, gone_after: Some(Duration::from_millis(100)), ..listening() });
            let (result, took) = send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF);
            assert_eq!(result, 1);
            assert!(took < Duration::from_secs(1), "{took:?}");
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write", "reason: none"]);
        }

        #[test]
        fn a_panic_inside_the_wait_leaves_the_wait_as_it_was() {
            let _serial = SERIAL.lock();
            // Windows goes ahead during the wait and that end's write panics: the
            // question still waits, with the menu keys held back, and takes the answer.
            let w = watched(|w| FakeHost {
                end_to: Some(w.target.0 as isize),
                answer_on_write: Some(1),
                panic_once: Cell::new(true),
                watch_waiting: true,
                ..listening()
            });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(
                take_log(),
                ["reason: Saving your notes…", "ask 1", "window got WM_ENDSESSION(1)", "write after the answer", "reason: none"]
            );
            // The end's own flag went with its panic.
            assert!(!CLOSING.get());
        }

        #[test]
        fn an_answer_on_its_way_when_windows_goes_ahead_is_taken() {
            let _serial = SERIAL.lock();
            // The page answers too late for the question, and its answer is
            // already in the queue when WM_ENDSESSION(TRUE) comes: waiting once
            // more takes it at once.
            let w = watched(|w| FakeHost { answers: false, post_answer: Some((w.main.0 as isize, 2)), ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write", "reason: none"]);
            assert_eq!(send(w.target, WM_ENDSESSION, 1, ENDSESSION_LOGOFF).0, 0);
            assert_eq!(take_log(), ["write", "write after the answer", "reason: none", "window got WM_ENDSESSION(1)"]);
        }

        #[test]
        fn a_page_that_is_gone_is_not_waited_for() {
            let _serial = SERIAL.lock();
            let w = watched(|_| listening());
            page_gone();
            let (result, took) = send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF);
            assert_eq!(result, 1);
            assert!(took < Duration::from_millis(500), "{took:?}");
            assert_eq!(take_log(), ["reason: Saving your notes…", "write", "reason: none"]);
            // The page loads again and sends what it holds: it is asked again.
            page_heard();
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write after the answer", "reason: none"]);
        }

        #[test]
        fn a_page_that_does_not_listen_is_not_asked() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost::default());
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(take_log(), ["reason: Saving your notes…", "write", "reason: none"]);
        }

        #[test]
        fn asks_once_per_attempt() {
            let _serial = SERIAL.lock();
            let w = watched(|_| listening());
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 1);
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 1);
            assert_eq!(
                take_log(),
                ["reason: Saving your notes…", "ask 1", "write after the answer", "reason: none", "reason: Saving your notes…", "write after the answer", "reason: none"]
            );
        }

        #[test]
        fn dispatches_posted_messages_while_it_waits_for_the_page() {
            let _serial = SERIAL.lock();
            let w = watched(|w| FakeHost { post_to: Some(w.main.0 as isize), ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 1);
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "posted message", "write after the answer", "reason: none"]);
        }

        /// Real input reaches only the window in the foreground: on a machine
        /// where the test cannot have it, the test says so and checks nothing.
        #[test]
        fn no_input_is_dispatched_while_it_waits_for_the_page() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { types: true, ..listening() });
            unsafe {
                let _ = SetWindowPos(w.main, None, 0, 0, 300, 200, SWP_NOZORDER);
                let _ = ShowWindow(w.main, SW_SHOW);
                let _ = SetForegroundWindow(w.main);
                let _ = SetFocus(Some(w.main));
            }
            let end = Instant::now() + Duration::from_millis(300);
            drain(|| Instant::now() >= end);
            if unsafe { GetForegroundWindow() } != w.main {
                eprintln!("skipped: the test window cannot come to the foreground here, so it gets no input");
                return;
            }
            take_log();
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 1);
            // The key typed during the wait reaches the window once the question is answered.
            let end = Instant::now() + Duration::from_millis(500);
            drain(|| Instant::now() >= end || LOG.with(|l| l.borrow().iter().any(|s| s == "key")));
            assert_eq!(take_log(), ["reason: Saving your notes…", "ask 1", "write after the answer", "reason: none", "key"]);
        }

        #[test]
        fn a_refusal_ends_once_nothing_is_held() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { unsaved: vec!["the settings".into()], ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, 0).0, 0);
            assert!(reason(w.main).is_some());
            take_log();
            settled();
            assert_eq!(reason(w.main), None);
            assert_eq!(take_log(), ["reason: none", "settled"]);
            // Once only.
            settled();
            assert_eq!(take_log(), Vec::<String>::new());
        }

        #[test]
        fn a_refusal_settles_after_a_cancel_too() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { unsaved: vec!["\"RO\"".into()], ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 0);
            // The cancel removes the reason; the page's notice stays until the note is saved.
            for hwnd in [w.main, w.target] {
                assert_eq!(send(hwnd, WM_ENDSESSION, 0, ENDSESSION_LOGOFF).0, 0);
            }
            assert_eq!(reason(w.main), None);
            take_log();
            settled();
            assert_eq!(take_log(), ["settled"]);
        }

        #[test]
        fn a_panic_says_yes_and_removes_the_reason() {
            let _serial = SERIAL.lock();
            let w = watched(|_| FakeHost { panics: true, ..listening() });
            assert_eq!(send(w.main, WM_QUERYENDSESSION, 0, ENDSESSION_LOGOFF).0, 1);
            assert_eq!(reason(w.main), None);
            assert!(!ASKING.get());
        }

        #[test]
        fn without_taos_window_restart_manager_saves_before_its_yes() {
            let _serial = SERIAL.lock();
            fresh();
            let main = window(w!("Cairn session end test"));
            assert!(!install_with(main, Box::new(FakeHost { unsaved: vec!["\"T\"".into()], ..listening() })));
            assert_eq!(send(main, WM_QUERYENDSESSION, 0, ENDSESSION_CLOSEAPP).0, 1);
            assert_eq!(take_log(), ["write"]);
            unsafe { DestroyWindow(main) }.unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_FLAGS: [u32; 6] = [
        0,
        ENDSESSION_LOGOFF,
        ENDSESSION_CLOSEAPP,
        ENDSESSION_CRITICAL,
        ENDSESSION_CRITICAL | ENDSESSION_LOGOFF,
        ENDSESSION_CLOSEAPP | ENDSESSION_LOGOFF,
    ];

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn says_no_only_at_a_sign_out_shutdown_or_restart() {
        for flags in ALL_FLAGS {
            let refuses = flags & (ENDSESSION_CLOSEAPP | ENDSESSION_CRITICAL) == 0;
            assert_eq!(may_refuse(flags), refuses, "flags {flags:#x}");
        }
        assert_eq!(kind(ENDSESSION_CLOSEAPP | ENDSESSION_LOGOFF), "Restart Manager closing Cairn (an installer or update)");
        assert_eq!(kind(ENDSESSION_CRITICAL | ENDSESSION_LOGOFF), "a forced end of the session");
        assert_eq!(kind(ENDSESSION_LOGOFF), "signing out");
        assert_eq!(kind(0), "shutting down or restarting");
    }

    #[test]
    fn the_reason_names_the_notes_in_plain_words() {
        assert_eq!(block_reason(&names(&["\"T\""])), "Unsaved changes to \"T\". Switch to Cairn to save or discard them.");
        assert_eq!(
            block_reason(&names(&["\"A\"", "the settings"])),
            "Unsaved changes to \"A\", the settings. Switch to Cairn to save or discard them."
        );
    }

    #[test]
    fn the_reason_fits_on_windows_screen() {
        let many: Vec<String> = (0..40).map(|i| format!("\"A rather long title of a note, number {i}\"")).collect();
        let text = block_reason(&many);
        assert!(utf16_len(&text) <= MAX_REASON, "{text}");
        assert!(text.starts_with("Unsaved changes to \"A rather long title of a note, number 0\", "), "{text}");
        assert!(text.ends_with(" more. Switch to Cairn to save or discard them."), "{text}");
        // As many names as fit: one more would not.
        let listed = text.matches("number").count();
        let more = format!(
            "Unsaved changes to {} and {} more. Switch to Cairn to save or discard them.",
            many[..=listed].join(", "),
            40 - listed - 1
        );
        assert!(utf16_len(&more) > MAX_REASON);
        // One name too long for the screen is cut, keeps its closing quote,
        // and the text still says what to do.
        let long = format!("\"{}\"", "Long ".repeat(80));
        let text = block_reason(&[long, "\"B\"".to_string()]);
        assert!(utf16_len(&text) <= MAX_REASON, "{text}");
        assert!(text.starts_with("Unsaved changes to \"Long Long "), "{text}");
        assert!(text.ends_with("Long…\" and 1 more. Switch to Cairn to save or discard them."), "{text}");
        // A name without quotes is cut the same way.
        let text = block_reason(&["x".repeat(400)]);
        assert!(utf16_len(&text) <= MAX_REASON && text.ends_with("x…. Switch to Cairn to save or discard them."), "{text}");
    }

    #[test]
    fn the_reason_is_cut_between_characters() {
        // An emoji is two UTF-16 units: the cut must not split it.
        for pad in 0..4 {
            let title = format!("\"{}{}\"", "x".repeat(pad), "😀".repeat(150));
            let text = block_reason(&[title]);
            assert!(utf16_len(&text) <= MAX_REASON, "{pad}: {text}");
            assert!(utf16_len(&text) >= MAX_REASON - 1, "{pad}: as much as fits");
            assert!(text.ends_with("😀…\". Switch to Cairn to save or discard them."), "{pad}: {text}");
        }
        assert_eq!(cut("a😀b", 2), "a");
        assert_eq!(cut("a😀b", 3), "a😀");
        assert_eq!(cut("abc", 5), "abc");
    }
}
