// Shared harness for the accessibility audit (adv_a11y_keyboard.test.mjs,
// adv_a11y_semantics.test.mjs). This file defines no tests; node --test
// loads it as an empty test file.
//
// Each test file starts one app on a throwaway vault with temp XDG dirs and
// calls `reset()` at the start of every test: that drops the saved session
// and reloads the web view, so every test starts from a clean UI (no tabs,
// no overlays) and a failing test cannot leave state behind for the next one.
//
// Run through scripts/e2e-headless.sh (private headless display and ports).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { setTimeout as sleep } from "node:timers/promises";
import { startDriver, Session } from "./webdriver.mjs";

export { sleep };
export const ROOT = path.resolve(import.meta.dirname, "..");
export const APP = path.join(ROOT, "target/debug/cairn");
export const SERVER = path.join(ROOT, "target/debug/cairn-server");
export const EVIDENCE = path.join(import.meta.dirname, ".tmp", "AX");

/** WebDriver key codes. */
export const K = {
  tab: "",
  enter: "",
  esc: "",
  space: " ",
  left: "",
  up: "",
  right: "",
  down: "",
  del: "",
  home: "",
  end: "",
  f2: "",
  f10: "",
  ctrl: "",
  shift: "",
};

export async function eventually(fn, { timeout = 5000, message = "condition" } = {}) {
  const end = Date.now() + timeout;
  let err;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      err = e;
    }
    await sleep(80);
  }
  throw new Error(`timed out: ${message}${err ? ` (${err.message})` : ""} (last: ${JSON.stringify(last)})`);
}

export function seedVault(vault) {
  const w = (rel, c) => {
    const p = path.join(vault, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, c);
  };
  w("Welcome.md", "# Welcome\n\nLinks: [[Ideas]], [[Projects/Garden plan|the garden]] and [[Not yet written]].\n\nA #tag1 and some `code` here.\n\n> a quote\n\nlast line\n");
  w("Ideas.md", "---\ntags: [brainstorm]\n---\n# Ideas\n\nSee [[Welcome]].\n\n## Sub heading\n");
  w("Projects/Garden plan.md", "# Garden plan\n\nTomatoes and basil. Back to [[Welcome]].\n");
  w("Journal/2026-10-03.md", "Daily note about [[Ideas]].\n");
  fs.copyFileSync(path.join(ROOT, "app/src-tauri/icons/32x32.png"), path.join(vault, "pic.png"));
}

export class AxApp {
  constructor(prefix) {
    this.tmp = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
    this.vault = path.join(this.tmp, "vault");
    fs.mkdirSync(this.vault, { recursive: true });
    seedVault(this.vault);
  }

  /** `env`: extra environment for the app (e.g. a PATH with a stand-in xdg-open). */
  async start(env = {}) {
    fs.mkdirSync(EVIDENCE, { recursive: true });
    this.drv = await startDriver(4444, {
      XDG_CONFIG_HOME: path.join(this.tmp, "config"),
      XDG_DATA_HOME: path.join(this.tmp, "data"),
      XDG_CACHE_HOME: path.join(this.tmp, "cache"),
      ...env,
    });
    this.s = await Session.create(this.drv.port, APP, [this.vault]);
    await this.s.waitFor(`return document.querySelectorAll('[data-testid=tree-row]').length >= 4`, { timeout: 20000 });
    await this.install();
  }

  async stop(finalShot) {
    if (this.s && finalShot) {
      try {
        fs.writeFileSync(path.join(EVIDENCE, finalShot), await this.s.screenshot());
      } catch {}
    }
    await this.s?.close();
    this.drv?.proc.kill();
    // WebKit may still be writing its cache into the temp dir right after the
    // session closes; retry, and never fail a test over a leftover temp dir.
    try {
      fs.rmSync(this.tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    } catch {}
  }

  p(rel) {
    return path.join(this.vault, rel);
  }
  write(rel, c) {
    fs.mkdirSync(path.dirname(this.p(rel)), { recursive: true });
    fs.writeFileSync(this.p(rel), c);
  }
  read(rel) {
    return fs.readFileSync(this.p(rel), "utf8");
  }
  exists(rel) {
    return fs.existsSync(this.p(rel));
  }

  async install() {
    await this.s.exec(PAGE_HELPERS);
  }

  /** Clean UI: forget the saved session (tabs, expanded folders) and reload the page. */
  async reset({ minRows = 4 } = {}) {
    try {
      await this.s.cmd("POST", "/window/maximize", {});
    } catch {}
    await this.s.exec(`addEventListener("beforeunload", () => { try { localStorage.clear(); } catch {} }); return 1`);
    await this.s.cmd("POST", "/refresh", {});
    await this.s.waitFor(
      `return document.querySelectorAll('[data-testid=tree-row]').length >= ${minRows} && !document.querySelector('[data-testid=tab]') && !document.querySelector('.workspace.narrow')`,
      { timeout: 15000, message: "app reloaded with an empty session" },
    );
    await this.install();
  }

  exec(script, ...args) {
    return this.s.exec(script, ...args);
  }
  keys(...seq) {
    return this.s.keys(...seq);
  }
  chord(...k) {
    return this.s.keys({ chord: k });
  }
  async shot(name) {
    try {
      fs.writeFileSync(path.join(EVIDENCE, name), await this.s.screenshot());
    } catch {}
    return path.join("e2e/.tmp/AX", name);
  }
  /** Short description of the focused element ("BODY" when nothing has focus). */
  focus() {
    return this.s.exec(`return __ax.desc(document.activeElement)`);
  }
  activeTab() {
    return this.s.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path ?? null`);
  }
  tabs() {
    return this.s.exec(`return [...document.querySelectorAll('[data-testid=tab]')].map(t => t.dataset.path)`);
  }
  focusInEditor() {
    return this.s.exec(`return !!document.activeElement?.closest('.cm-content')`);
  }

  /** Open a note with the keyboard only: Ctrl+O, type, Enter. */
  async openNote(query, expectPath) {
    await this.chord(K.ctrl, "o");
    await this.s.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`, { message: "switcher focused" });
    await this.keys(query);
    await this.s.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.toLowerCase().includes(${JSON.stringify(query.toLowerCase())})`, { message: `switcher lists ${query}` });
    await this.keys(K.enter);
    await eventually(async () => (await this.activeTab()) === expectPath, { message: `tab ${expectPath} active` });
    await eventually(() => this.focusInEditor(), { message: "editor focused after opening" });
  }

  /** Run a palette command with the keyboard only. */
  async palette(query) {
    await this.chord(K.ctrl, "p");
    await this.s.waitFor(`return document.activeElement?.dataset.testid === 'palette-input'`, { message: "palette focused" });
    await this.keys(query);
    await this.s.waitFor(`return document.querySelector('[data-testid=palette-item]')?.textContent.toLowerCase().includes(${JSON.stringify(query.toLowerCase())})`, { message: `palette lists ${query}` });
    await this.keys(K.enter);
    await sleep(150);
  }

  /** `theme` "light" or "dark", in the light or dark theme `name` (Limestone or Slate when not given). */
  setTheme(theme, name) {
    // Same effect as Settings > Appearance > Theme and Light theme / Dark theme
    // (settings.svelte.ts sets data-theme, data-light-theme and data-dark-theme on <html>).
    return this.s.exec(
      `const r = document.documentElement; r.dataset.theme = arguments[0]; r.dataset.lightTheme = arguments[1] ?? 'limestone'; r.dataset.darkTheme = arguments[2] ?? 'slate'; return 1`,
      theme,
      theme === "light" ? (name ?? null) : null,
      theme === "dark" ? (name ?? null) : null,
    );
  }

  async rectOf(css) {
    return this.s.exec(`const r = document.querySelector(arguments[0]).getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }`, css);
  }
}

// ---------------------------------------------------------------------------
// In-page helpers (installed after every reload): element description,
// approximate accessible-name computation, contrast and focus-style audits.
export const PAGE_HELPERS = String.raw`
window.__ax = (() => {
  const NAME_FROM_CONTENT = new Set(["button", "link", "tab", "option", "menuitem", "menuitemcheckbox", "menuitemradio", "treeitem", "checkbox", "radio", "switch", "heading", "cell", "columnheader", "rowheader", "row", "tooltip", "gridcell"]);
  function role(el) {
    const r = el.getAttribute("role");
    if (r) return r.split(" ")[0];
    const t = el.tagName.toLowerCase();
    if (t === "button") return "button";
    if (t === "a" && el.hasAttribute("href")) return "link";
    if (t === "select") return el.multiple ? "listbox" : "combobox";
    if (t === "textarea") return "textbox";
    if (t === "input") {
      const ty = (el.type || "text").toLowerCase();
      return { checkbox: "checkbox", radio: "radio", range: "slider", button: "button", submit: "button", reset: "button", color: "color", search: "searchbox" }[ty] || "textbox";
    }
    if (/^h[1-6]$/.test(t)) return "heading";
    return "";
  }
  function hidden(el) {
    for (let e = el; e && e.nodeType === 1; e = e.parentElement) {
      if (e.getAttribute("aria-hidden") === "true") return true;
      const cs = getComputedStyle(e);
      if (cs.display === "none" || cs.visibility === "hidden") return true;
    }
    return false;
  }
  function textOf(node, top) {
    if (node.nodeType === 3) return node.textContent;
    if (node.nodeType !== 1) return "";
    const el = node;
    if (el.getAttribute("aria-hidden") === "true") return "";
    const cs = getComputedStyle(el);
    if (cs.display === "none") return "";
    if (!top) {
      if (el.getAttribute("aria-label")) return " " + el.getAttribute("aria-label") + " ";
      if (el.tagName === "IMG") return el.alt || "";
      const r = role(el);
      if (r === "textbox" || r === "searchbox") return el.value || "";
    }
    let s = "";
    for (const c of el.childNodes) s += textOf(c, false);
    if (!top && !s.trim() && el.title) s = el.title;
    return cs.display !== "inline" ? " " + s + " " : s;
  }
  // Approximation of the accname algorithm: labelledby, aria-label, <label>,
  // name from content for roles that allow it, then title, then placeholder
  // (reported with a "(placeholder)" prefix because it is only a fallback).
  function name(el) {
    const lb = el.getAttribute("aria-labelledby");
    if (lb) return lb.split(/\s+/).map((id) => document.getElementById(id)?.textContent ?? "").join(" ").replace(/\s+/g, " ").trim();
    const al = el.getAttribute("aria-label");
    if (al && al.trim()) return al.trim();
    const r = role(el);
    const t = el.tagName;
    if (t === "INPUT" || t === "SELECT" || t === "TEXTAREA") {
      const labs = el.labels ? [...el.labels].map((l) => textOf(l, true)).join(" ").replace(/\s+/g, " ").trim() : "";
      if (labs) return labs;
      if (el.title) return el.title;
      if (el.placeholder) return "(placeholder) " + el.placeholder;
      return "";
    }
    if (NAME_FROM_CONTENT.has(r)) {
      const s = textOf(el, true).replace(/\s+/g, " ").trim();
      if (s) return s;
    }
    if (el.title) return el.title;
    return "";
  }
  function desc(el) {
    if (!el || el === document.body || el === document.documentElement) return "BODY";
    let s = el.tagName.toLowerCase();
    if (el.getAttribute("role")) s += "[role=" + el.getAttribute("role") + "]";
    if (el.dataset && el.dataset.testid) s += "[data-testid=" + el.dataset.testid + "]";
    const cls = [...el.classList].filter((c) => !c.startsWith("svelte-") && c !== "__axf");
    if (cls.length) s += "." + cls.join(".");
    if (el.tagName === "INPUT") s += "[type=" + el.type + "]";
    const n = name(el);
    if (n) s += ' "' + n.slice(0, 40) + '"';
    return s;
  }
  const INTERACTIVE = "button, a[href], input:not([type=hidden]), select, textarea, [tabindex]:not([tabindex='-1']), [role=button], [role=tab], [role=treeitem], [role=option], [role=menuitem], [role=tree], [role=listbox], [role=dialog], [role=textbox], [role=checkbox], [role=switch], [role=slider], [role=menu], [role=tablist], [role=combobox]";
  function audit(root) {
    const out = [];
    for (const el of (root || document.body).querySelectorAll(INTERACTIVE)) {
      if (hidden(el)) continue;
      out.push({ el: desc(el), role: role(el), name: name(el) });
    }
    return out;
  }

  // ----- contrast -----
  function parse(c) {
    if (!c) return null;
    let m = c.match(/^rgba?\(([^)]+)\)$/);
    if (m) { const p = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number); return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 }; }
    m = c.match(/^color\(srgb ([^)]+)\)$/);
    if (m) { const p = m[1].split(/[ \/]+/).filter(Boolean).map(Number); return { r: p[0] * 255, g: p[1] * 255, b: p[2] * 255, a: p.length > 3 ? p[3] : 1 }; }
    if (c === "transparent") return { r: 0, g: 0, b: 0, a: 0 };
    return null;
  }
  const over = (top, bot) => { const a = top.a + bot.a * (1 - top.a); if (a === 0) return { r: 0, g: 0, b: 0, a: 0 }; return { r: (top.r * top.a + bot.r * bot.a * (1 - top.a)) / a, g: (top.g * top.a + bot.g * bot.a * (1 - top.a)) / a, b: (top.b * top.a + bot.b * bot.a * (1 - top.a)) / a, a }; };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }; return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const hex = (c) => "#" + [c.r, c.g, c.b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
  function bgOf(el) {
    const stack = [];
    for (let e = el; e && e.nodeType === 1; e = e.parentElement) stack.push(parse(getComputedStyle(e).backgroundColor) || { r: 0, g: 0, b: 0, a: 0 });
    let c = parse(getComputedStyle(document.documentElement).backgroundColor);
    if (!c || c.a === 0) c = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = stack.length - 1; i >= 0; i--) c = over(stack[i], c);
    return c;
  }
  function opacity(el) { let o = 1; for (let e = el; e && e.nodeType === 1; e = e.parentElement) o *= Number(getComputedStyle(e).opacity); return o; }
  function visible(el) { const r = el.getBoundingClientRect(); if (r.width < 1 || r.height < 1) return false; const cs = getComputedStyle(el); return cs.visibility !== "hidden" && cs.display !== "none"; }
  function label(el) {
    let s = el.tagName.toLowerCase();
    if (el.dataset.testid) s += "[data-testid=" + el.dataset.testid + "]";
    const cls = [...el.classList].filter((c) => !c.startsWith("svelte-"));
    if (cls.length) s += "." + cls.join(".");
    return s;
  }
  // Every visible element that directly holds text: effective colours after
  // alpha compositing, ratio and the WCAG AA threshold for its size.
  function contrast(root) {
    const out = [];
    const seen = new Set();
    const walker = document.createTreeWalker(root || document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (!n.textContent.trim()) continue;
      const el = n.parentElement;
      if (!el || seen.has(el) || !visible(el)) continue;
      if (el.closest("[aria-hidden=true], svg, style, script")) continue;
      seen.add(el);
      const cs = getComputedStyle(el);
      let fg = parse(cs.color);
      if (!fg) continue;
      const bg = bgOf(el);
      fg = { ...fg, a: fg.a * opacity(el) };
      const eff = over(fg, bg);
      const size = parseFloat(cs.fontSize), weight = Number(cs.fontWeight) || 400;
      const large = size >= 24 || (size >= 18.66 && weight >= 700);
      const need = large ? 3 : 4.5;
      const r = ratio(eff, bg);
      out.push({ el: label(el), text: n.textContent.trim().slice(0, 40), fg: hex(eff), bg: hex(bg), ratio: Math.round(r * 100) / 100, need, size, pass: r >= need });
    }
    return out;
  }
  // Non-text contrast (3:1) of icons inside buttons and of tree chevrons.
  function icons(root) {
    const out = [];
    for (const svg of (root || document.body).querySelectorAll("svg")) {
      const b = svg.closest("button, [role=button], .chev");
      if (!b || !visible(svg)) continue;
      let fg = parse(getComputedStyle(svg).color);
      if (!fg) continue;
      fg = { ...fg, a: fg.a * opacity(svg) };
      const bg = bgOf(svg);
      const r = ratio(over(fg, bg), bg);
      out.push({ el: label(b), name: b.getAttribute("title") || b.getAttribute("aria-label") || "", fg: hex(over(fg, bg)), bg: hex(bg), ratio: Math.round(r * 100) / 100, need: 3, pass: r >= 3 });
    }
    return out;
  }
  // Contrast of two theme variables (fg over bg over the page background).
  function pair(fgCss, bgCss) {
    const d = document.createElement("div");
    d.style.cssText = "position:fixed;left:-9999px;color:" + fgCss + ";background:" + bgCss;
    document.body.appendChild(d);
    const cs = getComputedStyle(d);
    const fg = parse(cs.color), bgRaw = parse(cs.backgroundColor);
    d.remove();
    const bg = over(bgRaw, parse(getComputedStyle(document.body).backgroundColor));
    const eff = over(fg, bg);
    return { fg: hex(eff), bg: hex(bg), ratio: Math.round(ratio(eff, bg) * 100) / 100 };
  }

  // ----- focus indicators -----
  // The headless window never has system focus, so :focus never matches and
  // focus rings cannot be screenshotted. Instead every author rule that uses
  // :focus / :focus-visible / :focus-within is copied with the pseudo-class
  // replaced by the class "__axf", and WebKit's UA rule
  // ":focus-visible { outline: auto }" is emulated with a zero-specificity
  // rule placed first (so any author "outline" declaration beats it, as an
  // author-origin rule beats the UA origin). Adding "__axf" to an element then
  // shows the styles it would get when focused with the keyboard.
  let installed = false;
  function installFocus() {
    if (installed) return;
    installed = true;
    const rules = [];
    const walk = (list) => { for (const r of list) { if (r instanceof CSSStyleRule) { if (/:focus/.test(r.selectorText)) rules.push(r); } else if (r.cssRules) walk(r.cssRules); } };
    for (const sh of document.styleSheets) { try { walk(sh.cssRules); } catch (e) {} }
    const ua = document.createElement("style");
    ua.textContent = ":where(.__axf){outline:auto 5px -webkit-focus-ring-color}";
    document.head.prepend(ua);
    const au = document.createElement("style");
    au.textContent = rules.map((r) => {
      const sel = r.selectorText.split(",").filter((s) => /:focus/.test(s)).map((s) => s.replace(/:focus-visible|:focus(?![-\w])/g, ".__axf").replace(/:focus-within/g, ":has(.__axf)")).join(",");
      return sel + "{" + r.style.cssText + "}";
    }).join("\n");
    document.head.append(au);
  }
  const PROPS = ["outlineStyle", "outlineWidth", "outlineColor", "boxShadow", "borderTopColor", "borderBottomColor", "backgroundColor", "color", "textDecorationLine"];
  function snap(el) { const cs = getComputedStyle(el); const o = {}; for (const p of PROPS) o[p] = cs[p]; return o; }
  function focusStyle(el) {
    installFocus();
    const before = snap(el);
    el.classList.add("__axf");
    const after = snap(el);
    el.classList.remove("__axf");
    const ring = after.outlineStyle !== "none" && parseFloat(after.outlineWidth) > 0;
    const changed = PROPS.filter((p) => before[p] !== after[p]).filter((p) => !(p.startsWith("outline") && !ring));
    return { indicator: changed.length > 0, changed, opacity: opacity(el) };
  }
  return { role, name, desc, audit, hidden, contrast, icons, pair, parse, ratio, bgOf, hex, over, opacity, focusStyle };
})();
return "ok";
`;

// ---------------------------------------------------------------------------
// Minimal PNG decoder (8-bit, non-interlaced RGB/RGBA/grey) for measuring
// colours that getComputedStyle cannot report, such as ::placeholder text.
export function decodePng(buf) {
  let pos = 8, w = 0, h = 0, bitDepth = 0, colorType = 0, interlace = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString("ascii", pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      w = data.readUInt32BE(0);
      h = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
      interlace = data[12];
    } else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    pos += 12 + len;
  }
  if (bitDepth !== 8 || interlace) throw new Error(`unsupported PNG (depth ${bitDepth}, interlace ${interlace})`);
  const bpp = { 2: 3, 6: 4, 0: 1, 4: 2 }[colorType];
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = w * bpp;
  const out = Buffer.alloc(w * h * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < h; y++) {
    const cur = Buffer.alloc(stride);
    const f = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? cur[x - bpp] : 0, b = prev[x], c = x >= bpp ? prev[x - bpp] : 0;
      let v = line[x];
      if (f === 1) v += a;
      else if (f === 2) v += b;
      else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      cur[x] = v & 255;
    }
    for (let x = 0; x < w; x++) {
      const o = (y * w + x) * 4;
      if (bpp >= 3) {
        out[o] = cur[x * bpp];
        out[o + 1] = cur[x * bpp + 1];
        out[o + 2] = cur[x * bpp + 2];
      } else out[o] = out[o + 1] = out[o + 2] = cur[x * bpp];
      out[o + 3] = 255;
    }
    prev = cur;
  }
  return { w, h, px: out };
}

const lumRgb = ([r, g, b]) => {
  const f = (v) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
};

/** Background = most common colour in the region; text = the colour furthest from it in luminance. */
export function regionContrast(img, x0, y0, x1, y1) {
  const counts = new Map();
  for (let y = Math.max(0, Math.round(y0)); y < Math.min(img.h, Math.round(y1)); y++)
    for (let x = Math.max(0, Math.round(x0)); x < Math.min(img.w, Math.round(x1)); x++) {
      const o = (y * img.w + x) * 4;
      const k = `${img.px[o]},${img.px[o + 1]},${img.px[o + 2]}`;
      counts.set(k, (counts.get(k) || 0) + 1);
    }
  const bg = [...counts.entries()].sort((a, b) => b[1] - a[1])[0][0].split(",").map(Number);
  const lb = lumRgb(bg);
  let fg = bg, best = -1;
  for (const k of counts.keys()) {
    const c = k.split(",").map(Number);
    const d = Math.abs(lumRgb(c) - lb);
    if (d > best) {
      best = d;
      fg = c;
    }
  }
  const a = lumRgb(fg);
  const hex = (c) => "#" + c.map((v) => v.toString(16).padStart(2, "0")).join("");
  return { bg: hex(bg), fg: hex(fg), ratio: Math.round(((Math.max(a, lb) + 0.05) / (Math.min(a, lb) + 0.05)) * 100) / 100 };
}
