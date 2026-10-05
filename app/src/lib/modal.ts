// Keyboard behaviour shared by the modal overlays (quick switcher, command
// palette, Settings, Version history, dialogs, the sidebar drawers on a small
// screen): Tab and Shift+Tab stay inside the top overlay, Escape closes only
// the top one wherever focus is, and when an overlay (or the context menu)
// closes focus goes back to the element that had it before (or, when that
// cannot take it, to the overlay underneath).

import { untrack } from "svelte";
import type { Attachment } from "svelte/attachments";
import { app } from "./app.svelte";
import { recordingHotkey } from "./hotkeyRecorder";

interface Layer {
  node: HTMLElement;
  close: () => void;
  opener: Element | null;
  escapeLast: boolean;
}

export interface ModalOptions {
  /** Closes the overlay (Escape). */
  close: () => void;
  /**
   * The element that had focus before the overlay opened; it gets focus back
   * when the overlay closes. Take it when the overlay is asked for
   * (app.takeOpener()) or, for a dialog, in a pre-effect: an overlay this one
   * replaces is removed first, and the editor drops its focus in an effect
   * once a modal covers it.
   */
  opener: Element | null;
  /**
   * The controls inside take Escape first (the rename box, the context
   * menu): it closes the overlay only if none of them used it.
   */
  escapeLast?: boolean;
}

const layers: Layer[] = [];
/** Overlays that have closed, with the element focused before each opened. */
const closed = new WeakMap<Element, Element | null>();

const FOCUSABLE = "button, input, select, textarea, a[href], [tabindex]:not([tabindex='-1'])";

/** The top overlay, unless the key is not for it. */
function topFor(e: KeyboardEvent): Layer | undefined {
  // While a hotkey is being recorded, Settings takes every key itself.
  if (recordingHotkey() || e.isComposing || e.ctrlKey || e.altKey || e.metaKey) return;
  return layers[layers.length - 1];
}

function onKey(e: KeyboardEvent) {
  const top = topFor(e);
  if (!top) return;
  if (e.key === "Escape") {
    if (top.escapeLast) return; // see onLateKey
    e.preventDefault();
    top.close();
  } else if (e.key === "Tab" || e.code === "Tab") {
    // WebKitGTK gives Shift+Tab the key "Unidentified" (GTK sends it as ISO_Left_Tab).
    trapTab(top.node, e);
  }
}

/** Escape for an overlay with `escapeLast`, once the event has been through the page. */
function onLateKey(e: KeyboardEvent) {
  const top = topFor(e);
  if (e.key !== "Escape" || !top?.escapeLast || e.defaultPrevented) return;
  e.preventDefault();
  top.close();
}

/** The controls Tab can reach inside `node`. */
function tabbable(node: HTMLElement): HTMLElement[] {
  return [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) => !(el as HTMLButtonElement).disabled && el.getClientRects().length > 0,
  );
}

/** Keep Tab inside the overlay, wrapping at either end. */
function trapTab(node: HTMLElement, e: KeyboardEvent) {
  const items = tabbable(node);
  if (!items.length) {
    e.preventDefault();
    return;
  }
  const i = items.indexOf(document.activeElement as HTMLElement);
  const first = items[0];
  const last = items[items.length - 1];
  let next: HTMLElement | null = null;
  if (i < 0) next = e.shiftKey ? last : first;
  else if (!e.shiftKey && i === items.length - 1) next = first;
  else if (e.shiftKey && i === 0) next = last;
  if (!next) return; // the browser moves focus within the overlay
  e.preventDefault();
  next.focus();
}

/**
 * Give focus back to `el`. If `el` was inside an overlay that has closed since
 * (one overlay replaced another), it goes to where that overlay came from.
 * The editor is focused through its own focus(), which keeps its cursor, and
 * only while it is shown and nothing covers it.
 */
export function restoreFocus(el: Element | null) {
  for (let hops = 0; el && !el.isConnected && hops < 8; hops++) {
    let a: Element | null = el;
    while (a && !closed.has(a)) a = a.parentElement;
    if (!a) return;
    el = closed.get(a) ?? null;
  }
  if (!el || !el.isConnected || el === document.body) return;
  if (app.view?.contentDOM.contains(el)) {
    const shown = app.active?.kind === "note" && app.active.mode !== "preview";
    if (shown && !app.settingsOpen && !app.historyFor && !app.dialog) app.view.focus();
  } else if (el instanceof HTMLElement) el.focus();
}

/** Svelte action for the overlay's dialog element. */
export function modal(node: HTMLElement, opts: ModalOptions) {
  const layer: Layer = { node, close: opts.close, opener: opts.opener, escapeLast: !!opts.escapeLast };
  layers.push(layer);
  if (layers.length === 1) {
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keydown", onLateKey);
  }
  return {
    update(o: ModalOptions) {
      layer.close = o.close;
    },
    destroy() {
      layers.splice(layers.indexOf(layer), 1);
      if (!layers.length) {
        window.removeEventListener("keydown", onKey, true);
        window.removeEventListener("keydown", onLateKey);
      }
      handBack(node, layer.opener);
    },
  };
}

/**
 * Svelte attachment for a sidebar shown as a drawer over the editor (small
 * screens), while it is open: it is a modal overlay that takes focus on its
 * first control, and the panels inside take Escape first.
 */
export function drawer(close: () => void): Attachment<HTMLElement> {
  return (node) =>
    untrack(() => {
      const m = modal(node, { close, opener: document.activeElement, escapeLast: true });
      if (!node.contains(document.activeElement)) tabbable(node)[0]?.focus();
      return () => {
        m.destroy();
        // Its toggle has gone if the window was widened (with this sidebar
        // closed in the wide layout): the editor then.
        queueMicrotask(() => {
          if (lost(node)) restoreFocus(app.view?.contentDOM ?? null);
        });
      };
    });
}

/**
 * Svelte action for a popup that takes focus but is not modal (the context
 * menu): when it closes, focus goes back as it does for an overlay.
 */
export function popup(node: HTMLElement, opener: Element | null) {
  return { destroy: () => handBack(node, opener) };
}

/** `node` has closed: give focus back to `opener` if it went down with it. */
function handBack(node: HTMLElement, opener: Element | null) {
  closed.set(node, opener);
  // After the update that closed the overlay has finished: until then the
  // app state still reads as before it. Only when focus went down with the
  // overlay (removed, or hidden like a closed drawer); never take it from
  // something that has claimed it since (another overlay, the rename box),
  // or from a drawer that stays in view as a sidebar (window widened).
  queueMicrotask(() => {
    if (!lost(node)) return;
    restoreFocus(opener);
    // The opener has gone or cannot take focus (the Connect button in
    // Settings is disabled while it runs, so a question it asks opens from
    // <body>): then to the overlay still open underneath, if any.
    if (lost(node)) focusTop();
  });
}

/** Focus the top overlay itself, or its first control if it cannot take focus. */
function focusTop() {
  const top = layers[layers.length - 1];
  if (!top?.node.isConnected) return;
  (top.node.hasAttribute("tabindex") ? top.node : tabbable(top.node)[0])?.focus();
}

/** Focus is nowhere, or inside `node` while that is removed or hidden. */
function lost(node: HTMLElement) {
  const now = document.activeElement;
  if (!now || now === document.body || !now.isConnected) return true;
  return node.contains(now) && !node.getClientRects().length;
}
