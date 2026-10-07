//! What WebView2, the web view Cairn runs in on Windows, may do on its own
//! as a browser. Left alone it reloads the window on F5 or Ctrl+R (which
//! drops edits that could not be saved), prints on Ctrl+P, opens its find
//! bar on Ctrl+F, asks about caret browsing on F7, and offers Back, Refresh,
//! Save as and Print on a right-click. Cairn keeps its zoom keys and the
//! editing items of the right-click menu.
//!
//! The rules are plain functions, so that they build and are tested on every
//! platform. `attach` (Windows only) installs them on the web view. This file
//! uses no Tauri types: a Windows check can build it on its own.

// Windows virtual-key codes (winuser.h), as plain numbers so that the rules
// build on every platform.
const VK_0: u32 = 0x30;
const VK_I: u32 = 0x49;
const VK_R: u32 = 0x52;
const VK_NUMPAD0: u32 = 0x60;
const VK_ADD: u32 = 0x6B;
const VK_SUBTRACT: u32 = 0x6D;
const VK_F4: u32 = 0x73;
const VK_F5: u32 = 0x74;
const VK_F12: u32 = 0x7B;
const VK_SPACE: u32 = 0x20;
const VK_BROWSER_REFRESH: u32 = 0xA8;
const VK_OEM_PLUS: u32 = 0xBB;
const VK_OEM_MINUS: u32 = 0xBD;

/// A key pressed down while the web view has the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    /// The Windows virtual-key code.
    pub vk: u32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Whether WebView2 may act on a key press as a browser does. A key it may
/// not act on still reaches the page, so Cairn's own hotkeys and the
/// editor's keys work as before.
///
/// Its zoom keys stay: Ctrl with + or - or 0, on the main keys or the number
/// pad, also with Shift (Ctrl+Shift+= is Ctrl++ on many layouts) but not
/// with Alt, as AltGr is Ctrl+Alt on Windows and AltGr+0 types a character.
/// Alt+F4 and Alt+Space stay too, in case WebView2 counts them among its
/// keys: closing the window and its window menu must keep working. With
/// `devtools` (debug builds) F12 and Ctrl+Shift+I still open the developer
/// tools. Everything else, such as F5 and Ctrl+R (reload), Ctrl+P (print),
/// Ctrl+F and F3 (find), F7 (caret browsing) or Alt+Left (back), only
/// reaches the page.
pub fn browser_key_allowed(k: KeyPress, devtools: bool) -> bool {
    let zoom = k.ctrl && !k.alt && matches!(k.vk, VK_OEM_PLUS | VK_OEM_MINUS | VK_ADD | VK_SUBTRACT | VK_0 | VK_NUMPAD0);
    let window = k.alt && !k.ctrl && matches!(k.vk, VK_F4 | VK_SPACE);
    let tools = devtools && !k.alt && ((k.vk == VK_F12 && !k.ctrl && !k.shift) || (k.vk == VK_I && k.ctrl && k.shift));
    zoom || window || tools
}

/// The keys that reload the page: F5 (also with Shift or Ctrl), Ctrl+R (also
/// with Shift) and the keyboard's Refresh key. A WebView2 runtime older than
/// 120.0.2210 (December 2023) cannot keep a key from the browser and still
/// give it to the page, so there Cairn takes these keys from both, and
/// leaves the others.
pub fn is_reload_key(k: KeyPress) -> bool {
    match k.vk {
        VK_F5 => !k.alt,
        VK_R => k.ctrl && !k.alt,
        VK_BROWSER_REFRESH => true,
        _ => false,
    }
}

/// The right-click menu items Cairn keeps, by WebView2's name for them:
/// editing, the spelling suggestions that Settings > Editor > Spell check
/// offers, and copying a link or an image. The names are not always the
/// label: runtime 154.0.4258.62 sends "pasteAndMatchStyle" for Paste as
/// plain text, "copyLinkLocation" for Copy link and "spellcheck", all lower
/// case, for each spelling item. The label in lower camel case is listed as
/// well where it differs, in case another runtime sends that.
const KEPT_MENU_ITEMS: &[&str] = &[
    "emoji",
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "pasteAsPlainText",
    "pasteAndMatchStyle",
    "selectAll",
    "spellcheck",
    "spellCheck",
    "copyLink",
    "copyLinkLocation",
    "copyImage",
];

/// The items to remove from a right-click menu, as indexes in ascending
/// order. `items` holds each item's name and whether it is a separator.
/// Every item that is not in [`KEPT_MENU_ITEMS`] goes, and "inspectElement"
/// unless `devtools` (debug builds); so does a separator that would start
/// or end the menu or follow another one.
pub fn menu_items_to_remove(items: &[(&str, bool)], devtools: bool) -> Vec<usize> {
    let mut keep = vec![false; items.len()];
    // The last item kept so far.
    let mut last: Option<usize> = None;
    for (i, &(name, separator)) in items.iter().enumerate() {
        keep[i] = if separator {
            last.is_some_and(|l| !items[l].1)
        } else {
            KEPT_MENU_ITEMS.contains(&name) || (devtools && name == "inspectElement")
        };
        if keep[i] {
            last = Some(i);
        }
    }
    if let Some(l) = last.filter(|&l| items[l].1) {
        keep[l] = false;
    }
    (0..items.len()).filter(|&i| !keep[i]).collect()
}

#[cfg(windows)]
pub use glue::attach;

#[cfg(windows)]
mod glue {
    use super::{KeyPress, browser_key_allowed, is_reload_key, menu_items_to_remove};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR, COREWEBVIEW2_KEY_EVENT_KIND,
        COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, ICoreWebView2_11,
        ICoreWebView2AcceleratorKeyPressedEventArgs, ICoreWebView2AcceleratorKeyPressedEventArgs2,
        ICoreWebView2ContextMenuRequestedEventArgs, ICoreWebView2Controller,
    };
    use webview2_com::{AcceleratorKeyPressedEventHandler, ContextMenuRequestedEventHandler, take_pwstr};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT};
    use windows::core::{Interface, PWSTR, Result};

    /// Install the key and menu rules on the web view of `controller`. They
    /// apply at once, also to the page that is loading now.
    pub fn attach(controller: &ICoreWebView2Controller) -> Result<()> {
        let devtools = cfg!(debug_assertions);
        let mut token = 0i64;
        unsafe {
            controller.add_AcceleratorKeyPressed(
                &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| match args {
                    Some(args) => on_key(&args, devtools),
                    None => Ok(()),
                })),
                &mut token,
            )?;
            // ContextMenuRequested came with runtime 1.0.1185 (2022).
            match controller.CoreWebView2()?.cast::<ICoreWebView2_11>() {
                Ok(webview) => webview.add_ContextMenuRequested(
                    &ContextMenuRequestedEventHandler::create(Box::new(move |_, args| match args {
                        Some(args) => on_menu(&args, devtools),
                        None => Ok(()),
                    })),
                    &mut token,
                )?,
                Err(e) => log::warn!("this WebView2 runtime keeps its own right-click menu: {e}"),
            }
        }
        Ok(())
    }

    /// Whether `key` is held down, as Microsoft's sample checks it in this
    /// handler (the high bit, a negative value, says it is down).
    fn down(key: VIRTUAL_KEY) -> bool {
        unsafe { GetKeyState(i32::from(key.0)) < 0 }
    }

    /// Keep the browser from acting on a key press unless
    /// [`browser_key_allowed`]. The press is never marked handled, which
    /// would keep it from the page as well.
    fn on_key(args: &ICoreWebView2AcceleratorKeyPressedEventArgs, devtools: bool) -> Result<()> {
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut vk = 0;
        unsafe {
            args.KeyEventKind(&mut kind)?;
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                return Ok(());
            }
            args.VirtualKey(&mut vk)?;
        }
        let key = KeyPress { vk, ctrl: down(VK_CONTROL), alt: down(VK_MENU), shift: down(VK_SHIFT) };
        match args.cast::<ICoreWebView2AcceleratorKeyPressedEventArgs2>() {
            Ok(args2) => unsafe { args2.SetIsBrowserAcceleratorKeyEnabled(browser_key_allowed(key, devtools)) },
            // A runtime older than 120.0.2210: no reload, at the cost of the
            // page not seeing these keys either.
            Err(_) if is_reload_key(key) => unsafe { args.SetHandled(true) },
            Err(_) => Ok(()),
        }
    }

    /// Remove the items [`menu_items_to_remove`] names, and show no menu at
    /// all when none is left. The names go to the debug log with each item's
    /// command id, so that a run with RUST_LOG=cairn_app_lib=debug shows what
    /// WebView2 calls them and tells apart items that share a name.
    fn on_menu(args: &ICoreWebView2ContextMenuRequestedEventArgs, devtools: bool) -> Result<()> {
        let mut menu = Vec::new();
        unsafe {
            let items = args.MenuItems()?;
            let mut count = 0;
            items.Count(&mut count)?;
            for i in 0..count {
                let item = items.GetValueAtIndex(i)?;
                let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
                item.Kind(&mut kind)?;
                let mut name = PWSTR::null();
                item.Name(&mut name)?;
                let mut id = 0;
                item.CommandId(&mut id)?;
                menu.push((take_pwstr(name), kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR, id));
            }
            let named: Vec<(&str, bool)> = menu.iter().map(|(name, separator, _)| (name.as_str(), *separator)).collect();
            let remove = menu_items_to_remove(&named, devtools);
            // From the end, so that the indexes still to remove stay right.
            for &i in remove.iter().rev() {
                items.RemoveValueAtIndex(i as u32)?;
            }
            let label = |i: usize| match &menu[i] {
                (_, true, _) => String::from("-"),
                (name, false, id) => format!("{name} {id}"),
            };
            let kept: Vec<String> = (0..named.len()).filter(|i| !remove.contains(i)).map(label).collect();
            let removed: Vec<String> = remove.iter().map(|&i| label(i)).collect();
            log::debug!("right-click menu: kept {kept:?}, removed {removed:?}");
            if remove.len() == named.len() {
                args.SetHandled(true)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn plain(vk: u32) -> KeyPress {
        KeyPress { vk, ctrl: false, alt: false, shift: false }
    }
    const fn ctrl(vk: u32) -> KeyPress {
        KeyPress { ctrl: true, ..plain(vk) }
    }
    const fn alt(vk: u32) -> KeyPress {
        KeyPress { alt: true, ..plain(vk) }
    }
    const fn shift(k: KeyPress) -> KeyPress {
        KeyPress { shift: true, ..k }
    }
    /// Ctrl+Alt, which is also AltGr on Windows.
    const fn ctrl_alt(vk: u32) -> KeyPress {
        KeyPress { alt: true, ..ctrl(vk) }
    }

    #[test]
    fn the_browser_keeps_its_zoom_keys() {
        for vk in [VK_OEM_PLUS, VK_OEM_MINUS, VK_ADD, VK_SUBTRACT, VK_0, VK_NUMPAD0] {
            assert!(browser_key_allowed(ctrl(vk), false), "Ctrl+{vk:#x}");
            assert!(browser_key_allowed(shift(ctrl(vk)), false), "Ctrl+Shift+{vk:#x}");
            assert!(!browser_key_allowed(plain(vk), false), "{vk:#x} alone");
            // AltGr+0 types "}" on a German keyboard.
            assert!(!browser_key_allowed(ctrl_alt(vk), false), "AltGr+{vk:#x}");
        }
    }

    #[test]
    fn other_browser_keys_only_reach_the_page() {
        let (f3, f7, arrow_left) = (0x72, 0x76, 0x25);
        let keys = [
            ("F5", plain(VK_F5)),
            ("Shift+F5", shift(plain(VK_F5))),
            ("Ctrl+F5", ctrl(VK_F5)),
            ("Ctrl+R", ctrl(VK_R)),
            ("Ctrl+Shift+R", shift(ctrl(VK_R))),
            ("Refresh", plain(VK_BROWSER_REFRESH)),
            ("Ctrl+P", ctrl(0x50)),
            ("Ctrl+F", ctrl(0x46)),
            ("F3", plain(f3)),
            ("F7", plain(f7)),
            ("Ctrl+S", ctrl(0x53)),
            ("Ctrl+U", ctrl(0x55)),
            ("Alt+Left", alt(arrow_left)),
            ("Ctrl+Shift+C", shift(ctrl(0x43))),
            ("Escape", plain(0x1B)),
            ("Ctrl+C", ctrl(0x43)),
            ("Ctrl+V", ctrl(0x56)),
            ("Ctrl+Alt+F4", ctrl_alt(VK_F4)),
        ];
        for (name, k) in keys {
            assert!(!browser_key_allowed(k, false), "{name}");
        }
    }

    #[test]
    fn developer_tools_keys_in_debug_builds_only() {
        for (name, k) in [("F12", plain(VK_F12)), ("Ctrl+Shift+I", shift(ctrl(VK_I)))] {
            assert!(!browser_key_allowed(k, false), "{name}");
            assert!(browser_key_allowed(k, true), "{name} in a debug build");
        }
        let others = [("Ctrl+F5", ctrl(VK_F5)), ("Ctrl+I", ctrl(VK_I)), ("Alt+F12", alt(VK_F12)), ("Ctrl+Alt+Shift+I", shift(ctrl_alt(VK_I)))];
        for (name, k) in others {
            assert!(!browser_key_allowed(k, true), "{name} in a debug build");
        }
    }

    #[test]
    fn closing_the_window_and_its_window_menu_stay() {
        assert!(browser_key_allowed(alt(VK_F4), false));
        assert!(browser_key_allowed(shift(alt(VK_F4)), false));
        assert!(browser_key_allowed(alt(VK_SPACE), false));
        assert!(!browser_key_allowed(plain(VK_F4), false));
        assert!(!browser_key_allowed(ctrl(VK_F4), false));
        assert!(!browser_key_allowed(plain(VK_SPACE), false));
    }

    #[test]
    fn reload_keys_for_old_runtimes() {
        for k in [plain(VK_F5), shift(plain(VK_F5)), ctrl(VK_F5), ctrl(VK_R), shift(ctrl(VK_R)), plain(VK_BROWSER_REFRESH)] {
            assert!(is_reload_key(k), "{k:?}");
        }
        for k in [plain(VK_R), shift(plain(VK_R)), alt(VK_F5), ctrl_alt(VK_R), ctrl(0x50), ctrl(VK_OEM_PLUS), alt(VK_F4)] {
            assert!(!is_reload_key(k), "{k:?}");
        }
    }

    const SEP: (&str, bool) = ("other", true);
    const fn item(name: &str) -> (&str, bool) {
        (name, false)
    }

    /// The items left after removing what [`menu_items_to_remove`] says, separators as "-".
    fn left(items: &[(&str, bool)], devtools: bool) -> Vec<String> {
        let remove = menu_items_to_remove(items, devtools);
        assert!(remove.windows(2).all(|w| w[0] < w[1]), "ascending: {remove:?}");
        let kept = items.iter().enumerate().filter(|(i, _)| !remove.contains(i));
        kept.map(|(_, &(name, separator))| String::from(if separator { "-" } else { name })).collect()
    }

    #[test]
    fn the_page_menu_goes_entirely() {
        // The menu of the page itself, as runtime 154.0.4258.62 showed it on the status bar,
        // a tab, the right sidebar and the Welcome screen; debug builds add Inspect.
        let page = [
            item("back"),
            item("forward"),
            item("reload"),
            SEP,
            item("saveAs"),
            item("print"),
            SEP,
            item("moreTools"),
            item("inspectElement"),
        ];
        assert_eq!(menu_items_to_remove(&page, false), (0..page.len()).collect::<Vec<_>>());
        assert_eq!(left(&page, true), ["inspectElement"]);
        assert!(left(&[item("refresh"), item("share"), item("webCapture"), item("unknown")], false).is_empty());
        assert!(left(&[], false).is_empty());
    }

    #[test]
    fn the_editing_menu_keeps_its_editing_items() {
        // Runtime 154.0.4258.62 in the editor and in text fields, with nothing selected. Its
        // "other" item is not a separator.
        let editor = [
            item("emoji"),
            SEP,
            item("undo"),
            item("redo"),
            SEP,
            item("cut"),
            item("copy"),
            item("paste"),
            item("pasteAndMatchStyle"),
            item("selectAll"),
            SEP,
            item("other"),
            SEP,
            item("moreTools"),
        ];
        let kept = ["emoji", "-", "undo", "redo", "-", "cut", "copy", "paste", "pasteAndMatchStyle", "selectAll"];
        assert_eq!(left(&editor, false), kept);
        // The same with a selection, which brings Print.
        let selection = [
            item("emoji"),
            SEP,
            item("cut"),
            item("copy"),
            item("paste"),
            item("pasteAndMatchStyle"),
            item("selectAll"),
            SEP,
            item("print"),
            item("other"),
            SEP,
            item("moreTools"),
        ];
        assert_eq!(left(&selection, false), ["emoji", "-", "cut", "copy", "paste", "pasteAndMatchStyle", "selectAll"]);
        // The names other runtimes may send for Paste as plain text.
        assert_eq!(left(&[item("pasteAsPlainText"), item("writingDirection")], false), ["pasteAsPlainText"]);
    }

    #[test]
    fn the_spelling_suggestions_stay() {
        // Runtime 154.0.4258.62 on a misspelled word: three items named "spellcheck", then
        // the editing items.
        let spelling = [
            item("spellcheck"),
            item("spellcheck"),
            item("spellcheck"),
            SEP,
            item("cut"),
            item("copy"),
            item("paste"),
            item("pasteAndMatchStyle"),
            SEP,
            item("moreTools"),
        ];
        let kept = ["spellcheck", "spellcheck", "spellcheck", "-", "cut", "copy", "paste", "pasteAndMatchStyle"];
        assert_eq!(left(&spelling, false), kept);
        assert_eq!(left(&[item("spellCheck"), SEP, item("cut")], false), ["spellCheck", "-", "cut"]);
    }

    #[test]
    fn selections_links_and_images_keep_their_copy_items() {
        // Runtime 154.0.4258.62 in the reading view.
        let selection = [item("copy"), item("copyLinkToHighlight"), item("print"), SEP, item("moreTools")];
        assert_eq!(left(&selection, false), ["copy"]);
        let link = [item("openLinkInNewWindow"), SEP, item("saveLinkAs"), item("copyLinkLocation"), SEP, item("moreTools")];
        assert_eq!(left(&link, false), ["copyLinkLocation"]);
        let image = [item("saveImageAs"), item("copyImage"), item("copyImageLocation"), SEP, item("moreTools")];
        assert_eq!(left(&image, false), ["copyImage"]);
        // The name other runtimes may send for Copy link.
        assert_eq!(left(&[item("saveLinkAs"), item("copyLink"), item("copyImageLink")], false), ["copyLink"]);
    }

    #[test]
    fn separators_never_start_end_or_double() {
        let items = [SEP, item("back"), SEP, item("cut"), SEP, item("print"), SEP, SEP, item("copy"), SEP, item("saveAs"), SEP];
        assert_eq!(left(&items, false), ["cut", "-", "copy"]);
        assert!(left(&[SEP, SEP], false).is_empty());
        assert_eq!(left(&[item("paste"), SEP, SEP, item("print")], false), ["paste"]);
    }

    #[test]
    fn names_are_matched_exactly() {
        assert_eq!(left(&[item("Copy"), item("copy "), item("copyLinkAddress"), item("SpellCheck"), item("cut")], false), ["cut"]);
    }
}
