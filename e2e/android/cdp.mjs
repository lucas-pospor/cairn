// Drive the Cairn WebView on an Android device/emulator through the Chrome
// DevTools protocol (debug builds enable WebView debugging).
//   import { connect } from "./cdp.mjs"; const page = await connect(); await page.eval("1+1")

import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

export const adb = (...args) => execFileSync("adb", args, { encoding: "utf8" });

export async function connect(port = 9222) {
  let socket = "";
  for (let i = 0; i < 50 && !socket; i++) {
    let pid = "";
    try {
      pid = adb("shell", "pidof", "app.cairn.notes").trim();
    } catch {}
    if (pid && adb("shell", "cat", "/proc/net/unix").includes(`@webview_devtools_remote_${pid}`)) socket = `webview_devtools_remote_${pid}`;
    else await sleep(200);
  }
  if (!socket) throw new Error("no debuggable WebView found");
  adb("forward", `tcp:${port}`, `localabstract:${socket}`);
  let pages = [];
  for (let i = 0; i < 50 && !pages.length; i++) {
    try {
      pages = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).filter((p) => p.type === "page");
    } catch {}
    if (!pages.length) await sleep(200);
  }
  const ws = new WebSocket(pages[0].webSocketDebuggerUrl);
  await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)));
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve) => {
      const i = ++id;
      pending.set(i, resolve);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  const page = {
    async eval(expr) {
      const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
      if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails.exception?.description ?? r.result.exceptionDetails));
      return r.result?.result?.value;
    },
    async waitFor(expr, timeout = 10000) {
      const end = Date.now() + timeout;
      let last;
      while (Date.now() < end) {
        try {
          last = await page.eval(expr);
          if (last) return last;
        } catch (e) {
          last = e.message;
        }
        await sleep(150);
      }
      throw new Error(`timed out waiting for ${expr} (last: ${last})`);
    },
    close: () => ws.close(),
  };
  return page;
}

/** Tap at CSS-pixel coordinates of an element (real touch through adb). */
export async function tapElement(page, selector) {
  const measure = () =>
    page.eval(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); if (!e) return null; const b = e.getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2, dpr: devicePixelRatio, h: innerHeight }; })()`);
  // Wait until the element stops moving (the on-screen keyboard resizes the view).
  let r = await measure();
  for (let i = 0; i < 20; i++) {
    await sleep(150);
    const r2 = await measure();
    if (r && r2 && r.x === r2.x && r.y === r2.y && r.h === r2.h) break;
    r = r2;
  }
  if (!r) throw new Error(`no element ${selector}`);
  const top = webViewTop();
  adb("shell", "input", "tap", String(Math.round(r.x * r.dpr)), String(Math.round(r.y * r.dpr + top)));
}

/** Device-pixel y of the web view's top edge, from the UI hierarchy. */
export function webViewTop() {
  adb("shell", "uiautomator", "dump", "/sdcard/ui.xml");
  const xml = adb("shell", "cat", "/sdcard/ui.xml");
  const m = xml.match(/class="android\.webkit\.WebView"[^>]*?bounds="\[\d+,(\d+)\]/);
  return m ? Number(m[1]) : 0;
}

export function screenshot(file) {
  const png = execFileSync("adb", ["exec-out", "screencap", "-p"], { maxBuffer: 64 << 20 });
  return import("node:fs").then((fs) => fs.writeFileSync(file, png));
}
