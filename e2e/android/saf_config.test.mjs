// CSS snippets and plugins (the files in .cairn/snippets and .cairn/plugins)
// in a vault in a shared folder picked with the system picker (Storage Access
// Framework), and the same checks in a vault in the app's own storage as the
// control. Settings lists the files, a snippet turned on changes the page, a
// snippet made in Settings is saved in the vault and listed again after a
// restart, and a plugin turned on runs. The file tree still hides .cairn.
//
//   scripts/adv-android-run-all.sh e2e/android/saf_config.test.mjs
//
// Needs one emulator or device in `adb devices` and the debug APK. Clears the
// app's data and uses /sdcard/Documents/SafConfigT on the device. Screenshots
// go to e2e/.tmp/AN/.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import {
  Device, adb, devSh, sleep, eventually, key, kill9, appPid, closeSettings, fillSharedFolder, writeAppFile, readAppFile, runAs, q,
  APK, PKG, APP_DATA,
} from "./adv_helpers.mjs";

const LABEL = "SafConfigT";
const F = `/sdcard/Documents/${LABEL}`;
const APP_VAULT = "AppConfigT";
const V = `${APP_DATA}/vaults/${APP_VAULT}`;
const d = new Device();

const BIG = ":root { --bg: rgb(255, 236, 160); --accent: rgb(200, 10, 10); }\n";
const MADE = ":root { --text: rgb(20, 90, 20); }\n";
const HELLO = '// @name Hello\n// @description Says hello.\ncairn.commands.register("hello", "Say hello", () => cairn.ui.toast("hello from the plugin"));\n';

/**
 * What each test starts from; `settings` goes into .cairn/settings.json. The light theme
 * keeps the snippets' colours measurable: in dark mode the dark theme's variables win.
 */
function files(settings = {}) {
  return {
    "Seed.md": "seed\n",
    ".cairn/settings.json": JSON.stringify({ theme: "light", ...settings }),
    ".cairn/snippets/big.css": BIG,
    // Neither is a snippet: a hidden file, and a file in a subfolder.
    ".cairn/snippets/.hidden.css": ":root { --bg: rgb(1, 2, 3); }\n",
    ".cairn/snippets/sub/nested.css": ":root { --bg: rgb(4, 5, 6); }\n",
    ".cairn/plugins/hello.js": HELLO,
  };
}

const KINDS = [
  {
    name: "shared folder",
    slug: "saf",
    label: LABEL,
    fill: (fs) => fillSharedFolder(F, fs),
    read: (p) => devSh(`cat ${q(`${F}/${p}`)} 2>/dev/null || true`),
  },
  {
    name: "app storage",
    slug: "app",
    label: APP_VAULT,
    fill: (fs) => {
      runAs(`mkdir -p ${q(V)} && cd ${q(V)} && rm -rf ./* ./.trash ./.cairn`);
      for (const [name, content] of Object.entries(fs)) writeAppFile(`${V}/${name}`, content);
    },
    read: (p) => readAppFile(`${V}/${p}`),
  },
];

before(async () => {
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  fillSharedFolder(F, { "Seed.md": "seed\n" });
  await d.fresh();
  try {
    await d.pickSafFolder(LABEL);
  } catch (e) {
    await d.shot("saf-config-pick-failed.png").catch(() => {});
    console.log("toasts:", await d.toasts().catch(() => null));
    throw e;
  }
  await d.toWelcome();
  await d.createAppVault(APP_VAULT);
});

after(async () => {
  await d.shot("saf-config-final.png").catch(() => {});
  d.close();
  try {
    devSh(`rm -rf ${F}`);
  } catch {}
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

const invoke = (cmd, args = {}) =>
  d.eval(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).catch((e) => ({ ERR: String(e) }))`);
const cssVar = (name) => d.eval(`getComputedStyle(document.documentElement).getPropertyValue(${JSON.stringify(name)}).trim()`);
const applied = () => d.eval(`[...document.querySelectorAll('style[data-cairn-snippet]')].map((s) => s.dataset.cairnSnippet)`);
const snippetRows = () => d.eval(`[...document.querySelectorAll('[data-testid=settings] .row.snippet label')].map((l) => l.textContent.trim())`);
const pluginRows = () => d.eval(`[...document.querySelectorAll('[data-testid=plugin-row]')].map((r) => r.dataset.file)`);
const vaultSettings = (kind) => JSON.parse(kind.read(".cairn/settings.json") || "{}");
/** Every toast text shown since the last `open` (toasts go away after a few seconds). */
const toastLog = () => d.eval(`window.__toastLog ?? []`);

/** Show the vault of `kind` with exactly `fs` in it: refilled from the welcome screen, then opened from Recent. */
async function open(kind, fs) {
  await d.launch(); // also brings the app back to the front
  if (!(await d.isWelcome())) {
    await closeSettings(d);
    await d.toWelcome();
  }
  kind.fill(fs);
  await d.eval(`(() => {
    window.__toastLog = [];
    if (window.__toastObserver) return;
    window.__toastObserver = new MutationObserver(() => {
      for (const t of document.querySelectorAll('.toast .msg')) {
        const s = t.textContent.trim();
        if (s && !window.__toastLog.includes(s)) window.__toastLog.push(s);
      }
    });
    window.__toastObserver.observe(document.body, { childList: true, subtree: true, characterData: true });
  })()`);
  await d.openRecent(kind.label);
}

/** Kill the app and start it again; start-up reopens the last vault. */
async function restart(kind) {
  key("KEYCODE_HOME");
  await sleep(800);
  kill9();
  await eventually(() => !appPid(), { message: "app process gone" });
  await d.launch();
  await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 30000).catch(() => {});
  if (await d.isWelcome()) await d.openRecent(kind.label);
  assert.equal(await d.eval(`document.querySelector('.status .vault-name')?.textContent.trim()`), kind.label);
}

/** Tap `selector` once it is in view and has stopped moving. */
async function tap(selector) {
  await d.eval(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({ block: 'center', inline: 'center' })`);
  let last = null;
  for (let i = 0; i < 40; i++) {
    const r = await d.rect(selector);
    if (r && last && r.cx === last.cx && r.cy === last.cy && r.ih === last.ih) break;
    last = r;
    await sleep(250);
  }
  try {
    await d.tap(selector);
  } catch (e) {
    // The tap finds the web view with `uiautomator dump`, which is sometimes killed right after a restart.
    if (!String(e.message).includes("uiautomator")) throw e;
    await sleep(1000);
    await d.tap(selector);
  }
}

/** Settings, opened with taps from the Files drawer, on `section`. */
async function openSettings(section) {
  if (await d.eval(`document.querySelector('aside.left').classList.contains('hidden')`)) await tap("[data-testid=mobile-files]");
  await d.waitFor(`!document.querySelector('aside.left').classList.contains('hidden')`);
  await sleep(300);
  await tap("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings]')`);
  await tap(`[data-testid=settings-${section}]`);
  await d.waitFor(`document.querySelector('[data-testid=settings-${section}]').getAttribute('aria-current') === 'page'`);
}

/** Scroll Appearance to its CSS snippets part and wait a little for the rows (there may be none). */
async function showSnippets() {
  await d.eval(`[...document.querySelectorAll('[data-testid=settings] h4')].find((h) => h.textContent === 'CSS snippets').scrollIntoView({ block: 'start' })`);
  await eventually(async () => (await snippetRows()).length > 0, { timeout: 4000 }).catch(() => {});
  await sleep(300);
}

/** Tap the checkbox of snippet `name` in Settings > Appearance. */
async function tapSnippet(name) {
  await d.eval(
    `[...document.querySelectorAll('[data-testid=settings] .row.snippet label')].find((l) => l.textContent.trim() === ${JSON.stringify(name)}).querySelector('input').dataset.snippet = ${JSON.stringify(name)}`,
  );
  await tap(`[data-testid=settings] input[data-snippet="${name}"]`);
}

for (const kind of KINDS) {
  test(`${kind.name}: list_config lists the files in .cairn/snippets and .cairn/plugins, and the tree still hides .cairn`, async () => {
    await open(kind, files());
    assert.deepEqual(await invoke("list_config", { dir: "snippets" }), ["big.css"]);
    assert.deepEqual(await invoke("list_config", { dir: "plugins" }), ["hello.js"]);
    assert.deepEqual((await invoke("list_entries")).map((e) => e.path), ["Seed.md"]);
  });

  test(`${kind.name}: a snippet turned on in the vault's settings is applied when the vault opens`, async () => {
    await open(kind, files({ snippets: ["big.css"] }));
    await eventually(async () => (await applied()).includes("big.css"), { timeout: 10000, message: "big.css applied" });
    assert.equal(await cssVar("--accent"), "rgb(200, 10, 10)");
    assert.equal(await cssVar("--bg"), "rgb(255, 236, 160)");
  });

  test(`${kind.name}: Settings > Appearance lists the snippet, and turning it on changes the page`, async () => {
    await open(kind, files());
    assert.notEqual(await cssVar("--accent"), "rgb(200, 10, 10)");
    await openSettings("appearance");
    await showSnippets();
    await d.shot(`saf-config-${kind.slug}-appearance.png`);
    assert.deepEqual(await snippetRows(), ["big.css"]);
    await tapSnippet("big.css");
    await eventually(async () => (await cssVar("--accent")) === "rgb(200, 10, 10)", { message: "big.css applied" });
    assert.equal(await cssVar("--bg"), "rgb(255, 236, 160)");
    await eventually(() => vaultSettings(kind).snippets?.includes("big.css"), { message: "settings.json lists big.css" });
    await d.shot(`saf-config-${kind.slug}-snippet-on.png`);
    await closeSettings(d);
  });

  test(`${kind.name}: a snippet made in Settings is saved in the vault and listed again after a restart`, async () => {
    await open(kind, files());
    await openSettings("appearance");
    await showSnippets();
    await d.eval(`document.querySelector('[data-testid=snippet-new]').scrollIntoView({ block: 'center' })`);
    await d.click("[data-testid=snippet-new]");
    await d.waitFor(`!!document.querySelector('[data-testid=snippet-css]')`);
    assert.equal(await d.eval(`document.querySelector('[data-testid=snippet-name]').value`), "custom.css");
    await d.setValue("[data-testid=snippet-css]", MADE);
    await d.click("[data-testid=snippet-save]");
    await eventually(() => kind.read(".cairn/snippets/custom.css") === MADE, { message: "custom.css saved in the vault" });
    await eventually(() => vaultSettings(kind).snippets?.includes("custom.css"), { message: "settings.json lists custom.css" });
    await eventually(async () => (await snippetRows()).includes("custom.css"), { timeout: 5000, message: "custom.css listed" });
    await eventually(async () => (await cssVar("--text")) === "rgb(20, 90, 20)", { timeout: 5000, message: "custom.css applied" });
    await closeSettings(d);

    await restart(kind);
    assert.deepEqual(await invoke("list_config", { dir: "snippets" }), ["big.css", "custom.css"]);
    await eventually(async () => (await cssVar("--text")) === "rgb(20, 90, 20)", { timeout: 10000, message: "custom.css applied after the restart" });
    await openSettings("appearance");
    await showSnippets();
    assert.deepEqual(await snippetRows(), ["big.css", "custom.css"]);
    await closeSettings(d);
  });

  test(`${kind.name}: Settings > Plugins lists the plugin, and once turned on it runs`, async () => {
    // On in the vault's settings, but not approved on this device yet: it stays off, with a toast.
    await open(kind, files({ plugins: ["hello.js"] }));
    await eventually(async () => (await toastLog()).some((t) => t.includes("1 plugin in this notebook is off until you turn it on")), {
      timeout: 10000,
      message: "toast about the plugin that is off",
    });
    await openSettings("plugins");
    await eventually(async () => (await pluginRows()).length > 0, { timeout: 4000 }).catch(() => {});
    await d.shot(`saf-config-${kind.slug}-plugins.png`);
    assert.deepEqual(await pluginRows(), ["hello.js"]);
    const row = '[data-testid=plugin-row][data-file="hello.js"]';
    assert.equal(await d.eval(`!!document.querySelector('${row} [data-testid=plugin-not-approved]')`), true);
    await tap(`${row} [data-testid=plugin-toggle]`);
    await eventually(async () => (await invoke("plugin_approvals"))["hello.js"], { message: "plugin approved on this device" });
    await d.waitFor(`document.querySelector('${row} [data-testid=plugin-toggle]').checked && !document.querySelector('${row} [data-testid=plugin-not-approved]')`);
    await eventually(() => vaultSettings(kind).plugins?.includes("hello.js"), { message: "settings.json lists hello.js" });
    await closeSettings(d);
    // Its command is in the palette, and it runs. The palette lists the commands there are
    // when it opens, and the plugin registers its command a moment after it starts.
    const item = `[...document.querySelectorAll('[data-testid=palette-item]')].find((e) => e.textContent.includes('Hello: Say hello'))`;
    await eventually(
      async () => {
        await d.click(".mobile-bar [title='Commands']");
        await d.waitFor(`!!document.querySelector('[data-testid=palette-input]')`);
        await d.setValue("[data-testid=palette-input]", "Say hello");
        if (await d.eval(`!!${item}`)) return true;
        await d.eval(`document.querySelector('.backdrop').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))`);
        await d.waitFor(`!document.querySelector('[data-testid=palette-input]')`);
        await sleep(500);
        return false;
      },
      { message: "plugin command in the palette" },
    );
    await d.eval(`${item}.click()`);
    await eventually(async () => (await toastLog()).includes("Hello: hello from the plugin"), { message: "toast from the plugin" });
  });
}
