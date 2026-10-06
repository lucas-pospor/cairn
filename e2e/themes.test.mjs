// Light and dark themes in the real app: Settings > Appearance lists them by
// name, choosing one applies its colours and saves it in .cairn/settings.json,
// the choice comes back after a restart, "System" uses the chosen light or dark
// theme, and a theme this version does not know is kept in the file.
// Screenshots of each theme, wide and narrow, go to e2e/.tmp/themes/.
//
//   scripts/e2e-headless.sh e2e/themes.test.mjs
//
// The system's colour scheme: with no session bus the app cannot ask the
// desktop portal for it, so GTK_THEME decides what WebKit reports as
// prefers-color-scheme ("Adwaita:dark" is dark).

import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { launch, freshEnv, eventually, sleep } from "./adv_editor_lib.mjs";

const SHOTS = path.join(import.meta.dirname, ".tmp", "themes");
const SETTINGS = ".cairn/settings.json";

// Some colours of each theme, as app.css sets them.
const THEMES = {
  limestone: { scheme: "light", name: "Limestone", bg: "#fbfaf7", side: "#f3f1ec", text: "#24292b", accent: "#a84529" },
  marble: { scheme: "light", name: "Marble", bg: "#ffffff", side: "#f1f4f8", text: "#1b2738", accent: "#1f5bbf" },
  slate: { scheme: "dark", name: "Slate", bg: "#1d2022", side: "#181b1d", text: "#dfe3e0", accent: "#e5774f" },
  graphite: { scheme: "dark", name: "Graphite", bg: "#1e1e1e", side: "#191919", text: "#e0e0e0", accent: "#e5774f" },
};

const NOTES = {
  "Garden.md":
    "# Garden\n\nPlant [[Beans]] after the last frost, see [[Missing note]]. #spring\n\n> Water in the morning.\n\n```js\n// a comment\nclass Bed { size = 42; grow(n) { return \"tall\"; } }\n```\n\n- [ ] buy seeds\n",
  "Beans.md": "# Beans\n\nBack to [[Garden]].\n",
  "Notes/Ideas.md": "Some ideas.\n",
};

const envs = [];
function env(settings) {
  const e = freshEnv({ ...NOTES, ...(settings ? { [SETTINGS]: JSON.stringify(settings, null, 2) } : {}) });
  envs.push(e);
  return e;
}

function start(e, system = "light") {
  return launch({
    vault: e.vault.root,
    xdg: e.xdg,
    env: { DBUS_SESSION_BUS_ADDRESS: `unix:path=${path.join(e.xdg, "no-bus")}`, GTK_THEME: system === "dark" ? "Adwaita:dark" : "Adwaita" },
  });
}

after(async () => {
  for (const e of envs) await e.cleanup();
});

const hex = (rgb) => "#" + rgb.match(/\d+/g).slice(0, 3).map((v) => Number(v).toString(16).padStart(2, "0")).join("");

/** What the page shows: the attributes on <html>, some theme colours and the painted page background. */
function look(app) {
  return app
    .exec(
      `const r = document.documentElement, cs = getComputedStyle(r);
       // The built CSS may shorten #ffffff to #fff.
       const v = (n) => cs.getPropertyValue(n).trim().replace(/^#(\\w)(\\w)(\\w)$/, '#$1$1$2$2$3$3');
       return { theme: r.dataset.theme ?? null, light: r.dataset.lightTheme, dark: r.dataset.darkTheme, systemDark: matchMedia('(prefers-color-scheme: dark)').matches,
         bg: v('--bg'), side: v('--bg-side'), text: v('--text'), accent: v('--accent'), body: getComputedStyle(document.body).backgroundColor };`,
    )
    .then((l) => ({ ...l, body: hex(l.body) }));
}

/** The colours of `id` as the page should show them. */
const colours = (id) => ({ bg: THEMES[id].bg, side: THEMES[id].side, text: THEMES[id].text, accent: THEMES[id].accent, body: THEMES[id].bg });
const pick = (l) => ({ bg: l.bg, side: l.side, text: l.text, accent: l.accent, body: l.body });

async function openAppearance(app) {
  if (!(await app.exec(`return !!document.querySelector('[data-testid=settings]')`))) await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=settings-appearance]')`);
  await app.exec(`document.querySelector('[data-testid=settings-appearance]').click(); return 1`);
  await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-select]')`);
}

async function closeSettings(app) {
  await app.exec(`document.querySelector('[data-testid=settings] .close').click(); return 1`);
  await app.s.waitFor(`return !document.querySelector('[data-testid=settings]')`);
}

/** Settings > Appearance > Theme, as the select does it. */
const setMode = (app, mode) =>
  app.exec(`const s = document.querySelector('[data-testid=theme-select]'); s.value = arguments[0]; s.dispatchEvent(new Event('change', { bubbles: true })); return 1`, mode);

/** The light and dark theme choices: [[name, checked], ...] per group. */
const choices = (app) =>
  app.exec(
    `return [...document.querySelectorAll('[data-testid=settings] [role=radiogroup]')].map(g => ({
       label: document.getElementById(g.getAttribute('aria-labelledby')).textContent,
       options: [...g.querySelectorAll('label')].map(l => [l.textContent.trim(), l.querySelector('input').checked, l.querySelector('input').dataset.testid, !!l.querySelector('.swatch')]),
     }))`,
  );

const saved = (e) => JSON.parse(e.vault.read(SETTINGS));
/** The theme keys of the saved settings (a save writes every setting). */
const savedThemes = (e) => {
  const { theme, lightTheme, darkTheme } = saved(e);
  return { theme, lightTheme, darkTheme };
};

async function shot(app, name) {
  fs.mkdirSync(SHOTS, { recursive: true });
  fs.writeFileSync(path.join(SHOTS, `${name}.png`), await app.s.screenshot());
}

async function openNote(app, p) {
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="' + arguments[0] + '"]').click(); return 1`, p);
  await eventually(async () => (await app.exec(`return document.querySelector('[data-testid=tab][aria-selected=true]')?.dataset.path`)) === p, { message: `${p} open` });
}

test("Settings > Appearance lists the light and dark themes by name, with swatches", async () => {
  const e = env({ theme: "light" });
  const app = await start(e);
  try {
    await openAppearance(app);
    assert.deepEqual(await choices(app), [
      { label: "Light theme", options: [["Limestone", true, "theme-limestone", true], ["Marble", false, "theme-marble", true]] },
      { label: "Dark theme", options: [["Slate", true, "theme-slate", true], ["Graphite", false, "theme-graphite", true]] },
    ]);
    // The Theme select is unchanged.
    assert.deepEqual(await app.exec(`return [...document.querySelector('[data-testid=theme-select]').options].map(o => o.value)`), ["system", "light", "dark"]);
    // Nothing is written by opening Settings (a save would come 300 ms after a change).
    await sleep(600);
    assert.deepEqual(await app.invokes("write_config"), []);
    assert.deepEqual(saved(e), { theme: "light" });
  } finally {
    await app.stop();
  }
});

/** The graph's colours, and the page's colours they should match. */
const graphColours = (app) =>
  app.exec(
    `const r = document.querySelector('[data-testid=graph-view] .canvas').__sigma, cs = getComputedStyle(document.documentElement), v = (n) => cs.getPropertyValue(n).trim();
     return { node: r.getNodeDisplayData('Beans.md').color, edge: r.getEdgeDisplayData('Garden.md->Beans.md').color, label: r.getSetting('labelColor').color,
       want: { node: v('--text-muted'), edge: v('--border'), label: v('--text') } };`,
  );

/** Wait until the graph shows the page's node, edge and label colours (nothing hovered). */
async function graphFollows(app, what) {
  let g;
  await eventually(
    async () => {
      g = await graphColours(app);
      return g.node === g.want.node && g.edge === g.want.edge && g.label === g.want.label;
    },
    { message: `${what}: graph colours` },
  ).catch((err) => assert.fail(`${err.message}: ${JSON.stringify(g)}`));
  return g.want;
}

test("choosing each theme applies its colours and saves it; the graph takes the new colours", async () => {
  const e = env({ theme: "light" });
  const app = await start(e);
  try {
    await app.exec(`document.querySelector('[data-testid=open-graph]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=graph-view] .canvas').__sigma?.getNodeDisplayData('Garden.md')`, { timeout: 10000 });
    // Each click a change: Limestone and Slate are already chosen at the start.
    for (const id of ["marble", "graphite", "slate", "limestone"]) {
      const t = THEMES[id];
      await openAppearance(app);
      await setMode(app, t.scheme);
      await app.exec(`document.querySelector('[data-testid=theme-' + arguments[0] + ']').click(); return 1`, id);
      await eventually(async () => (await look(app)).bg === t.bg, { message: `${t.name} applied` });
      const l = await look(app);
      assert.deepEqual({ theme: l.theme, [t.scheme]: l[t.scheme], ...pick(l) }, { theme: t.scheme, [t.scheme]: id, ...colours(id) }, t.name);
      await eventually(() => saved(e)[`${t.scheme}Theme`] === id && saved(e).theme === t.scheme, { message: `${t.name} saved` });
      const checked = await app.exec(`return document.querySelector('[data-testid=theme-' + arguments[0] + ']').checked`, id);
      assert.equal(checked, true, `${t.name} checked`);
      // With no custom accent, the colour picker shows the theme's own.
      await eventually(async () => (await app.exec(`return document.querySelector('[data-testid=settings] input[type=color]').value`)) === t.accent, { message: `${t.name}: accent picker` });
      await closeSettings(app);
      // The graph is redrawn in the new colours without a hover or reload.
      await graphFollows(app, t.name);
    }
    assert.deepEqual(savedThemes(e), { theme: "light", lightTheme: "limestone", darkTheme: "slate" });

    // A snippet that sets the colours the graph uses, then turned off again.
    await openAppearance(app);
    await app.exec(`document.querySelector('[data-testid=snippet-new]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=snippet-css]')`);
    await app.exec(
      `const t = document.querySelector('[data-testid=snippet-css]'); t.value = ':root { --text-muted: #123456; --border: #345678; --text: #563412; }'; t.dispatchEvent(new Event('input', { bubbles: true }));
       document.querySelector('[data-testid=snippet-save]').click(); return 1`,
    );
    await eventually(() => e.vault.exists(".cairn/snippets/custom.css"), { message: "snippet saved" });
    await closeSettings(app);
    assert.deepEqual(await graphFollows(app, "snippet on"), { node: "#123456", edge: "#345678", label: "#563412" });
    await openAppearance(app);
    await app.exec(`[...document.querySelectorAll('[data-testid=settings] .row.snippet label')].find(l => l.textContent.trim() === 'custom.css').querySelector('input').click(); return 1`);
    await closeSettings(app);
    assert.deepEqual(await graphFollows(app, "snippet off"), { node: "#5c6260", edge: "#e2ded5", label: THEMES.limestone.text }, "Limestone again");

    // A custom accent: the note picked in the graph is drawn in it.
    await app.exec(
      `const i = document.querySelector('[data-testid=graph-view] input'); i.value = 'Beans'; i.dispatchEvent(new Event('input', { bubbles: true })); i.form.requestSubmit(); return 1`,
    );
    const picked = () =>
      app.exec(
        `const r = document.querySelector('[data-testid=graph-view] .canvas').__sigma; return { node: r.getNodeDisplayData('Beans.md').color, accent: getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() };`,
      );
    await eventually(async () => {
      const p = await picked();
      return p.node === p.accent;
    }, { message: "picked note in the theme's accent" });
    await openAppearance(app);
    await app.exec(`const i = document.querySelector('[data-testid=settings] input[type=color]'); i.value = '#3b82f6'; i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await closeSettings(app);
    let p;
    await eventually(async () => {
      p = await picked();
      return p.accent !== THEMES.limestone.accent && p.node === p.accent;
    }, { message: "picked note in the new accent" }).catch((err) => assert.fail(`${err.message}: ${JSON.stringify(p)}`));
  } finally {
    await app.stop();
  }
});

test("the chosen theme comes back after a restart", async () => {
  const e = env({ theme: "dark", lightTheme: "marble", darkTheme: "graphite" });
  let app = await start(e);
  try {
    assert.deepEqual(pick(await look(app)), colours("graphite"));
    await openAppearance(app);
    await setMode(app, "light");
    await eventually(async () => (await look(app)).bg === THEMES.marble.bg, { message: "Marble applied" });
    await eventually(() => saved(e).theme === "light", { message: "saved" });
    await app.stop();
    app = await start(e);
    const l = await look(app);
    assert.deepEqual({ theme: l.theme, light: l.light, dark: l.dark, ...pick(l) }, { theme: "light", light: "marble", dark: "graphite", ...colours("marble") });
    await openAppearance(app);
    const [light, dark] = await choices(app);
    assert.deepEqual([light.options.map((o) => o[1]), dark.options.map((o) => o[1])], [[false, true], [false, true]]);
  } finally {
    await app.stop();
  }
});

test("System uses the chosen light theme when the system is light, and the chosen dark theme when it is dark", async () => {
  const e = env({ theme: "system", lightTheme: "marble", darkTheme: "graphite" });
  const out = {};
  for (const system of ["light", "dark"]) {
    const app = await start(e, system);
    try {
      const l = await look(app);
      await openAppearance(app);
      const picker = await app.exec(`return document.querySelector('[data-testid=settings] input[type=color]').value`);
      await closeSettings(app);
      out[system] = { theme: l.theme, systemDark: l.systemDark, picker, ...pick(l) };
      await openNote(app, "Garden.md");
      await shot(app, `system-${system}`);
    } finally {
      await app.stop();
    }
  }
  assert.deepEqual(out, {
    light: { theme: null, systemDark: false, picker: THEMES.marble.accent, ...colours("marble") },
    dark: { theme: null, systemDark: true, picker: THEMES.graphite.accent, ...colours("graphite") },
  });
  // Without a choice, System uses Limestone and Slate.
  const plain = env({ theme: "system" });
  for (const [system, id] of [["light", "limestone"], ["dark", "slate"]]) {
    const app = await start(plain, system);
    try {
      assert.deepEqual(pick(await look(app)), colours(id), `${system} system, no choice`);
    } finally {
      await app.stop();
    }
  }
});

test("a theme this version does not know shows the default and stays in the file", async () => {
  const e = env({ theme: "light", lightTheme: "sandstone", darkTheme: 7 });
  const app = await start(e);
  try {
    const l = await look(app);
    assert.deepEqual({ light: l.light, dark: l.dark, ...pick(l) }, { light: "limestone", dark: "slate", ...colours("limestone") });
    await openAppearance(app);
    const [light] = await choices(app);
    assert.deepEqual(light.options.map((o) => o[1]), [true, false]);
    await app.exec(`document.querySelector('[data-testid=theme-graphite]').click(); return 1`);
    await eventually(() => saved(e).darkTheme === "graphite", { message: "Graphite saved" });
    assert.deepEqual(savedThemes(e), { theme: "light", lightTheme: "sandstone", darkTheme: "graphite" });
    // Clicking the default shown in its place saves it.
    await app.exec(`document.querySelector('[data-testid=theme-limestone]').click(); return 1`);
    await eventually(() => saved(e).lightTheme === "limestone", { message: "Limestone saved" });
  } finally {
    await app.stop();
  }
});

// deriveAccent (accent.ts) for #f2c200: the accent and its tint in each theme.
const DERIVED = { limestone: ["#796100", "#f1eee3"], marble: ["#7e6500", "#ece8d9"], slate: ["#f2c200", "#2e2d1f"], graphite: ["#f2c200", "#2f2b1c"] };

test("a custom accent stays readable in every theme", async () => {
  const e = env({ theme: "light", accent: "#f2c200" });
  const app = await start(e);
  try {
    const bad = [];
    for (const id of Object.keys(THEMES)) {
      await openAppearance(app);
      await setMode(app, THEMES[id].scheme);
      await app.exec(`document.querySelector('[data-testid=theme-' + arguments[0] + ']').click(); return 1`, id);
      await eventually(async () => (await look(app))[THEMES[id].scheme] === id, { message: `${id} applied` });
      // The derived accent is set inline on <html> for the theme now in use (values from accent.ts).
      await eventually(
        async () => JSON.stringify(await app.exec(`const st = document.documentElement.style; return [st.getPropertyValue('--accent'), st.getPropertyValue('--accent-soft')]`)) === JSON.stringify(DERIVED[id]),
        { message: `${id}: accent derived for this theme` },
      );
      const r = await app.exec(
        `const cs = getComputedStyle(document.documentElement), v = (n) => cs.getPropertyValue(n).trim().replace(/^#(\\w)(\\w)(\\w)$/, '#$1$1$2$2$3$3');
         return { accent: v('--accent'), link: v('--link'), text: v('--accent-text'), soft: v('--accent-soft'), bg: v('--bg'), side: v('--bg-side'), hover: v('--bg-hover') };`,
      );
      for (const [fg, bg] of [["accent", "bg"], ["accent", "side"], ["accent", "hover"], ["accent", "soft"], ["link", "bg"], ["text", "accent"]]) {
        const ratio = contrast(r[fg], r[bg]);
        if (ratio < 4.5) bad.push(`${id}: --${fg} ${r[fg]} on ${r[bg]} = ${ratio.toFixed(2)}`);
      }
    }
    assert.deepEqual(bad, []);
  } finally {
    await app.stop();
  }
});

test("each theme, wide and narrow: screenshots, and the theme choices fit a phone-sized window", async () => {
  const e = env({ theme: "light" });
  const app = await start(e);
  try {
    await openNote(app, "Garden.md");
    const narrow = {};
    for (const id of ["limestone", "marble", "slate", "graphite"]) {
      const t = THEMES[id];
      await openAppearance(app);
      await setMode(app, t.scheme);
      await app.exec(`document.querySelector('[data-testid=theme-' + arguments[0] + ']').click(); return 1`, id);
      await eventually(async () => (await look(app)).bg === t.bg, { message: `${t.name} applied` });
      await closeSettings(app);
      await sleep(300);
      await shot(app, `${id}-wide`);
      await openAppearance(app);
      await shot(app, `${id}-wide-settings`);
      await closeSettings(app);
      // A 480 px window (the smallest the desktop app allows) zoomed to about 375 CSS px, a phone's width.
      await app.s.cmd("POST", "/window/rect", { width: 480, height: 820 });
      await app.exec(`window.__TAURI_INTERNALS__.invoke('plugin:webview|set_webview_zoom', { label: 'main', value: 1.28 }); return 1`);
      await eventually(() => app.exec(`return innerWidth < 400`), { message: "narrow window" });
      await sleep(300);
      await shot(app, `${id}-narrow`);
      await openAppearance(app);
      await sleep(200);
      const fit = () =>
        app.exec(
          `const sec = document.querySelector('[data-testid=settings] section'), w = sec.getBoundingClientRect().right;
           const out = [...document.querySelectorAll('[data-testid=settings] [role=radiogroup] label')].filter(l => { const b = l.getBoundingClientRect(); return b.left < 0 || b.right > w; }).map(l => l.textContent.trim());
           return { innerWidth, overflow: sec.scrollWidth > sec.clientWidth + 1, cardsOutside: out };`,
        );
      narrow[`${id} 375`] = await fit();
      await shot(app, `${id}-narrow-settings`);
      // 320 CSS px, the width WCAG reflow asks for: the cards stack.
      await app.exec(`window.__TAURI_INTERNALS__.invoke('plugin:webview|set_webview_zoom', { label: 'main', value: 1.5 }); return 1`);
      await eventually(() => app.exec(`return innerWidth <= 320`), { message: "320 px window" });
      await sleep(200);
      narrow[`${id} 320`] = await fit();
      if (id === "marble") await shot(app, `${id}-320-settings`);
      await closeSettings(app);
      await app.exec(`window.__TAURI_INTERNALS__.invoke('plugin:webview|set_webview_zoom', { label: 'main', value: 1 }); return 1`);
      await app.s.cmd("POST", "/window/maximize", {});
      await eventually(() => app.exec(`return innerWidth > 760`), { message: "wide window" });
    }
    for (const [id, n] of Object.entries(narrow)) assert.deepEqual({ overflow: n.overflow, cardsOutside: n.cardsOutside }, { overflow: false, cardsOutside: [] }, `${id} at ${n.innerWidth}px`);
  } finally {
    await app.stop();
  }
});

/** WCAG contrast of two #rrggbb colours. */
function contrast(a, b) {
  for (const c of [a, b]) if (!/^#[0-9a-f]{6}$/i.test(c)) throw new Error(`not a #rrggbb colour: ${c}`);
  const lum = (h) => {
    const [r, g, bl] = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
