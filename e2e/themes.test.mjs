// Themes in the real app: Settings > Appearance lists System and every theme
// by name in one Theme list, choosing a theme applies its colours and saves it
// in .cairn/settings.json, the choice comes back after a restart, "System" uses
// the light and dark theme picked under it, a theme this version does not know
// is kept in the file, and the editor's search highlights matches in the
// theme's colours.
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
import { launch, freshEnv, eventually, sleep, Key } from "./adv_editor_lib.mjs";

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

/** Pick an entry of a list in Settings > Appearance ("system" or a theme id), as the select does it. */
const pickIn = (app, testid, value) =>
  app.exec(
    `const s = document.querySelector('[data-testid=' + arguments[0] + ']'); s.value = arguments[1];
     if (s.value !== arguments[1]) throw new Error('no ' + arguments[1] + ' in ' + arguments[0]);
     s.dispatchEvent(new Event('change', { bubbles: true })); return 1`,
    testid,
    value,
  );
const pickTheme = (app, value) => pickIn(app, "theme-select", value);

/**
 * The Theme list as Settings shows it: its name, its entries by group, the one
 * shown, the swatch next to it, and the Light and Dark lists under System.
 */
const themeList = (app) =>
  app.exec(
    `const s = document.querySelector('[data-testid=theme-select]');
     const sw = document.querySelector('[data-testid=theme-swatch]'), v = (n) => sw?.style.getPropertyValue(n);
     const sub = (id) => { const x = document.querySelector('[data-testid=' + id + ']'); return x && { name: x.getAttribute('aria-label'), value: x.value, options: [...x.options].map(o => [o.value, o.textContent.trim()]) }; };
     return {
       label: s.getAttribute('aria-labelledby').split(' ').map(id => document.getElementById(id).textContent).join(' '),
       entries: [...s.children].map(c => c.tagName === 'OPTGROUP' ? [c.label, [...c.children].map(o => [o.value, o.textContent.trim()])] : [c.value, c.textContent.trim()]),
       value: s.value,
       swatch: sw && { hidden: sw.getAttribute('aria-hidden'), bg: v('--sw-bg'), side: v('--sw-side'), text: v('--sw-text'), accent: v('--sw-accent') },
       light: sub('theme-light-select'),
       dark: sub('theme-dark-select'),
     };`,
  );

/** The swatch of `id`, as themeList reads it. */
const swatchOf = (id) => ({ hidden: "true", bg: THEMES[id].bg, side: THEMES[id].side, text: THEMES[id].text, accent: THEMES[id].accent });

/** The Theme list's entries: System, then the light and the dark themes. */
const ENTRIES = [
  ["system", "System"],
  ["Light", Object.entries(THEMES).filter(([, t]) => t.scheme === "light").map(([id, t]) => [id, t.name])],
  ["Dark", Object.entries(THEMES).filter(([, t]) => t.scheme === "dark").map(([id, t]) => [id, t.name])],
];
/** The Light or Dark list under System. */
const subList = (scheme, value) => ({
  name: `${scheme === "light" ? "Light" : "Dark"} theme`,
  value,
  options: Object.entries(THEMES).filter(([, t]) => t.scheme === scheme).map(([id, t]) => [id, t.name]),
});

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

test("Settings > Appearance lists System and every theme by name in one Theme list, with a swatch of the theme in use", async () => {
  const e = env({ theme: "light" });
  const app = await start(e);
  try {
    await openAppearance(app);
    assert.deepEqual(await themeList(app), { label: "Theme", entries: ENTRIES, value: "limestone", swatch: swatchOf("limestone"), light: null, dark: null });
    // No radio cards and no Light theme or Dark theme rows any more.
    assert.equal(await app.exec(`return document.querySelectorAll('[data-testid=settings] [role=radiogroup], [data-testid=settings] input[type=radio]').length`), 0);
    // Nothing is written by opening Settings (a save would come 300 ms after a change).
    await sleep(600);
    assert.deepEqual(await app.invokes("write_config"), []);
    assert.deepEqual(saved(e), { theme: "light" });
    // System shows the Light and Dark lists under it, with the themes that System uses.
    await pickTheme(app, "system");
    await eventually(() => saved(e).theme === "system", { message: "System saved" });
    assert.deepEqual(savedThemes(e), { theme: "system", lightTheme: undefined, darkTheme: undefined }, "System alone writes no theme ids");
    const l = await themeList(app);
    assert.deepEqual({ value: l.value, light: l.light, dark: l.dark }, { value: "system", light: subList("light", "limestone"), dark: subList("dark", "slate") });
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
    // Each pick a change: Limestone is shown at the start.
    for (const id of ["marble", "graphite", "slate", "limestone"]) {
      const t = THEMES[id];
      await openAppearance(app);
      await pickTheme(app, id);
      await eventually(async () => (await look(app)).bg === t.bg, { message: `${t.name} applied` });
      const l = await look(app);
      assert.deepEqual({ theme: l.theme, [t.scheme]: l[t.scheme], ...pick(l) }, { theme: t.scheme, [t.scheme]: id, ...colours(id) }, t.name);
      await eventually(() => saved(e)[`${t.scheme}Theme`] === id && saved(e).theme === t.scheme, { message: `${t.name} saved` });
      const list = await themeList(app);
      assert.deepEqual({ value: list.value, swatch: list.swatch, light: list.light }, { value: id, swatch: swatchOf(id), light: null }, `${t.name} shown`);
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
    assert.equal((await themeList(app)).value, "graphite");
    await pickTheme(app, "marble");
    await eventually(async () => (await look(app)).bg === THEMES.marble.bg, { message: "Marble applied" });
    await eventually(() => saved(e).theme === "light", { message: "saved" });
    await app.stop();
    app = await start(e);
    const l = await look(app);
    assert.deepEqual({ theme: l.theme, light: l.light, dark: l.dark, ...pick(l) }, { theme: "light", light: "marble", dark: "graphite", ...colours("marble") });
    await openAppearance(app);
    assert.equal((await themeList(app)).value, "marble");
    // System keeps both: Marble and Graphite are listed under it.
    await pickTheme(app, "system");
    const list = await themeList(app);
    assert.deepEqual([list.light.value, list.dark.value], ["marble", "graphite"]);
    await eventually(() => saved(e).theme === "system", { message: "System saved" });
    assert.deepEqual(savedThemes(e), { theme: "system", lightTheme: "marble", darkTheme: "graphite" });
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
      const list = await themeList(app);
      await closeSettings(app);
      out[system] = { theme: l.theme, systemDark: l.systemDark, picker, list: [list.value, list.light.value, list.dark.value], swatch: list.swatch, ...pick(l) };
      await openNote(app, "Garden.md");
      await shot(app, `system-${system}`);
    } finally {
      await app.stop();
    }
  }
  const lists = ["system", "marble", "graphite"];
  assert.deepEqual(out, {
    light: { theme: null, systemDark: false, picker: THEMES.marble.accent, list: lists, swatch: swatchOf("marble"), ...colours("marble") },
    dark: { theme: null, systemDark: true, picker: THEMES.graphite.accent, list: lists, swatch: swatchOf("graphite"), ...colours("graphite") },
  });
  // The Light and Dark lists under System change the pair and keep System.
  const pair = env({ theme: "system" });
  const app = await start(pair, "dark");
  try {
    await openAppearance(app);
    await pickIn(app, "theme-light-select", "marble");
    await eventually(() => saved(pair).lightTheme === "marble", { message: "Marble saved under System" });
    assert.deepEqual(pick(await look(app)), colours("slate"), "a dark system still shows the dark theme");
    await pickIn(app, "theme-dark-select", "graphite");
    await eventually(async () => (await look(app)).bg === THEMES.graphite.bg, { message: "Graphite applied" });
    await eventually(() => saved(pair).darkTheme === "graphite", { message: "Graphite saved under System" });
    assert.deepEqual(savedThemes(pair), { theme: "system", lightTheme: "marble", darkTheme: "graphite" });
    const list = await themeList(app);
    assert.deepEqual({ value: list.value, swatch: list.swatch, light: list.light, dark: list.dark }, { value: "system", swatch: swatchOf("graphite"), light: subList("light", "marble"), dark: subList("dark", "graphite") });
  } finally {
    await app.stop();
  }
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
    assert.equal((await themeList(app)).value, "limestone", "the default is shown in the list");
    await pickTheme(app, "graphite");
    await eventually(() => saved(e).darkTheme === "graphite", { message: "Graphite saved" });
    assert.deepEqual(savedThemes(e), { theme: "dark", lightTheme: "sandstone", darkTheme: "graphite" });
    // Picking the default that stood in for the unknown theme saves it.
    await pickTheme(app, "limestone");
    await eventually(() => saved(e).lightTheme === "limestone", { message: "Limestone saved" });
    assert.deepEqual(savedThemes(e), { theme: "light", lightTheme: "limestone", darkTheme: "graphite" });
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
      await pickTheme(app, id);
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

/** Settings > Appearance: show theme `id` (forcing its light or dark mode), then close Settings. */
async function useTheme(app, id) {
  const t = THEMES[id];
  await openAppearance(app);
  await pickTheme(app, id);
  await eventually(async () => (await look(app)).bg === t.bg, { message: `${t.name} applied` });
  await closeSettings(app);
}

test("the editor's search (Ctrl+F) highlights matches in each theme's own colours", async () => {
  const e = env({ theme: "light" });
  const app = await start(e);
  try {
    await openNote(app, "Garden.md");
    const out = {};
    for (const id of ["limestone", "marble", "slate", "graphite"]) {
      await useTheme(app, id);
      await app.exec(`document.querySelector('.cm-content').focus(); return 1`);
      await app.s.keys({ chord: [Key.ctrl, "f"] });
      await app.s.waitFor(`return document.activeElement?.name === 'search'`, { message: "search field focused" });
      await app.exec(`const i = document.activeElement; i.value = 'e'; i.dispatchEvent(new Event('change')); return 1`);
      await app.s.keys(Key.enter);
      await app.s.waitFor(`return !!document.querySelector('.cm-searchMatch-selected') && !!document.querySelector('.cm-searchMatch:not(.cm-searchMatch-selected)')`, { message: "search matches shown" });
      await shot(app, `${id}-editor-search`);
      out[id] = await app.exec(
        `const hit = getComputedStyle(document.documentElement).getPropertyValue('--hit').trim();
         const probe = document.createElement('span'); document.body.append(probe);
         const paint = (c) => { probe.style.backgroundColor = c; return getComputedStyle(probe).backgroundColor; };
         const want = { match: paint('color-mix(in srgb, ' + hit + ' 70%, transparent)'), current: paint(hit) };
         probe.remove();
         const bg = (css) => getComputedStyle(document.querySelector(css)).backgroundColor;
         return { match: bg('.cm-searchMatch:not(.cm-searchMatch-selected)'), current: bg('.cm-searchMatch-selected'), want };`,
      );
      await app.s.keys(Key.escape);
      await app.s.waitFor(`return !document.querySelector('.cm-search')`, { message: "search closed" });
    }
    // Not CodeMirror's own yellow (#ffff0054) and orange (#ff6a0054).
    for (const [id, o] of Object.entries(out)) assert.deepEqual({ match: o.match, current: o.current }, o.want, id);
  } finally {
    await app.stop();
  }
});

test("each theme, wide and narrow: screenshots, and the Theme list fits a phone-sized window", async () => {
  const e = env({ theme: "light" });
  const app = await start(e);
  try {
    await openNote(app, "Garden.md");
    const narrow = {};
    for (const id of ["limestone", "marble", "slate", "graphite"]) {
      const t = THEMES[id];
      await openAppearance(app);
      await pickTheme(app, id);
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
      // The Theme row's lists and swatch, and the Light and Dark lists under System, inside the section.
      const fit = () =>
        app.exec(
          `const sec = document.querySelector('[data-testid=settings] section'), w = sec.getBoundingClientRect().right;
           const out = [...document.querySelectorAll('[data-testid=settings] .theme-pick select, [data-testid=settings] .theme-pick .swatch, [data-testid=settings] .theme-pick label')]
             .filter(el => { const b = el.getBoundingClientRect(); return b.left < 0 || b.right > w; }).map(el => el.dataset.testid ?? el.textContent.trim());
           return { innerWidth, overflow: sec.scrollWidth > sec.clientWidth + 1, outside: out, lists: document.querySelectorAll('[data-testid=settings] .theme-pick select').length };`,
        );
      const fitBoth = async () => {
        const one = await fit();
        await pickTheme(app, "system");
        await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-dark-select]')`, { message: "Light and Dark lists" });
        const sys = await fit();
        await pickTheme(app, id);
        await app.s.waitFor(`return !document.querySelector('[data-testid=theme-dark-select]')`, { message: `${t.name} again` });
        return { innerWidth: one.innerWidth, overflow: one.overflow || sys.overflow, outside: [...one.outside, ...sys.outside], lists: [one.lists, sys.lists] };
      };
      narrow[`${id} 375`] = await fitBoth();
      await shot(app, `${id}-narrow-settings`);
      // 320 CSS px, the width WCAG reflow asks for.
      await app.exec(`window.__TAURI_INTERNALS__.invoke('plugin:webview|set_webview_zoom', { label: 'main', value: 1.5 }); return 1`);
      await eventually(() => app.exec(`return innerWidth <= 320`), { message: "320 px window" });
      await sleep(200);
      narrow[`${id} 320`] = await fitBoth();
      if (id === "marble") {
        await pickTheme(app, "system");
        await sleep(200);
        await shot(app, `${id}-320-settings-system`);
        await pickTheme(app, id);
        await eventually(async () => (await look(app)).bg === t.bg, { message: `${t.name} again` });
      }
      await closeSettings(app);
      await app.exec(`window.__TAURI_INTERNALS__.invoke('plugin:webview|set_webview_zoom', { label: 'main', value: 1 }); return 1`);
      await app.s.cmd("POST", "/window/maximize", {});
      await eventually(() => app.exec(`return innerWidth > 760`), { message: "wide window" });
    }
    for (const [id, n] of Object.entries(narrow)) assert.deepEqual({ overflow: n.overflow, outside: n.outside, lists: n.lists }, { overflow: false, outside: [], lists: [1, 3] }, `${id} at ${n.innerWidth}px`);
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
