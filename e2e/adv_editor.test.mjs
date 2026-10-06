// Adversarial tests for the editor (Live Preview, cursor, undo, IME,
// checkboxes, embeds) in the real desktop app.
//
// Run:  scripts/e2e-headless.sh e2e/adv_editor.test.mjs
//
// Every test starts its own app on its own temp vault (see adv_editor_lib.mjs).

import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import { withApp, eventually, sleep, Key } from "./adv_editor_lib.mjs";

const text = (app) => app.text();
const PNG = fs.readFileSync(new URL("../app/src-tauri/icons/32x32.png", import.meta.url));

// ---------------------------------------------------------------- findings

test(
  "FINDING-037: Ctrl+Z after an external change does not silently write the old text over the file",
  async () => {
    await withApp({ "Note.md": "line one\n" }, async (app, env) => {
      await app.open("Note.md");
      await app.setSel(8); // end of "line one"
      await app.keys(" mine");
      await eventually(() => env.vault.read("Note.md") === "line one mine\n", { message: "autosave of my edit" });
      // Another editor, git pull or a sync from another device changes the note.
      env.vault.write("Note.md", "line one mine\nEXTERNAL LINE FROM ANOTHER DEVICE\n");
      await eventually(async () => (await text(app)).includes("EXTERNAL LINE"), { message: "clean tab reloaded" });
      // The user presses Ctrl+Z once (to undo their own last edit).
      await app.chord(Key.ctrl, "z");
      await sleep(1500);
      const disk = env.vault.read("Note.md");
      const banner = await app.exec(`return document.querySelector('[data-testid=conflict-banner]')?.textContent ?? null`);
      assert.ok(
        disk.includes("EXTERNAL LINE") || banner,
        `the external edit was silently overwritten on disk.\n  editor now: ${JSON.stringify(await text(app))}\n  disk now:   ${JSON.stringify(disk)}\n  banner:     ${banner}`,
      );
    }, { shot: "ED-01-undo-external" });
  },
);

const TABLE_NOTE = (filler) =>
  "intro line\n" + Array.from({ length: filler }, (_, i) => `filler line ${i} with some words in it`).join("\n") + "\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nend\n";

test(
  "FINDING-091: clicking a rendered table after deleting lines above it puts the cursor on the table",
  async () => {
    const tail = Array.from({ length: 15 }, (_, i) => `tail line ${i} here`).join("\n") + "\n";
    await withApp({ "T.md": TABLE_NOTE(4) + "\n" + tail }, async (app) => {
      await app.open("T.md");
      await app.fakeFocus();
      await app.s.waitFor(`return !!document.querySelector('.cm-lp-table')`);
      // Select the filler lines (2..5) and delete them with Backspace.
      const r = await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; return [v.state.doc.line(2).from, v.state.doc.line(5).to + 1]`);
      await app.setSel(r[0], r[1]);
      await app.keys(Key.backspace);
      await sleep(200);
      const c = await app.center(".cm-lp-table");
      await app.clickAt(c.x, c.y);
      await sleep(300);
      const cur = await app.cursorLine();
      assert.equal(cur.text, "| A | B |", `clicking the table put the cursor on line ${cur.n}: ${JSON.stringify(cur.text)}`);
    }, { shot: "ED-02-table-click" });
  },
);

test(
  "FINDING-091: clicking a table after deleting many lines above it throws no RangeError and puts the cursor on the table",
  async () => {
    await withApp({ "T.md": TABLE_NOTE(40) }, async (app) => {
      await app.open("T.md");
      await app.fakeFocus();
      await app.s.waitFor(`return !!document.querySelector('.cm-lp-table')`);
      const r = await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; return [v.state.doc.line(2).from, v.state.doc.line(41).to + 1]`);
      await app.setSel(r[0], r[1]);
      await app.keys(Key.backspace);
      await sleep(200);
      const c = await app.center(".cm-lp-table");
      await app.clickAt(c.x, c.y);
      await sleep(300);
      const errs = await app.errors();
      const cur = await app.cursorLine();
      assert.deepEqual(errs, [], "page errors after clicking the table");
      assert.equal(cur.text, "| A | B |", `cursor after the click is on line ${cur.n}`);
    }, { shot: "ED-02-table-rangeerror" });
  },
);

test(
  "FINDING-092: Live Preview embeds update after the embedded note changes while the host note is being edited",
  async () => {
    await withApp({ "Host.md": "top line\n\n![[Inner]]\n\nbottom\n", "Inner.md": "OLD INNER TEXT\n" }, async (app, env) => {
      await app.open("Host.md");
      await app.fakeFocus();
      await app.setSel(8); // end of "top line"
      await app.s.waitFor(`return document.querySelector('.cm-lp-embed')?.textContent.includes('OLD INNER')`);
      env.vault.write("Inner.md", "NEW INNER TEXT\n");
      await sleep(1500);
      await app.keys(" edited");
      await sleep(1500);
      const shown = await app.exec(`return document.querySelector('.cm-lp-embed')?.textContent`);
      assert.ok(shown.includes("NEW INNER"), `embed still shows: ${JSON.stringify(shown)} (disk: ${JSON.stringify(env.vault.read("Inner.md"))})`);
    }, { shot: "ED-03-stale-embed" });
  },
);

test(
  "FINDING-093: Live Preview shows frontmatter lines the properties box cannot parse, and body text after an opening --- rule",
  async () => {
    const yaml =
      "---\ntitle: Trip\ndescription: |\n  SECRET PLAN line one\n  line two\nlocation:\n  city: PARIS\n# a comment\n---\n# Heading\n\nlast line\n";
    const rule = "---\nIntro paragraph BODY TEXT that is not YAML.\n\n---\n\nAfter the rule.\n";
    await withApp({ "Y.md": yaml, "R.md": rule }, async (app) => {
      const problems = [];
      for (const [file, needles] of [
        ["Y.md", ["SECRET PLAN", "PARIS", "a comment"]],
        ["R.md", ["BODY TEXT"]],
      ]) {
        await app.open(file);
        await app.fakeFocus();
        // cursor on the last line, away from the frontmatter
        await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } }); return 1`);
        await sleep(200);
        const visible = await app.exec(`return document.querySelector('.cm-content').innerText`);
        for (const n of needles) if (!visible.includes(n)) problems.push(`${file}: "${n}" is in the file but not visible anywhere: ${JSON.stringify(visible)}`);
      }
      assert.deepEqual(problems, []);
    }, { shot: "ED-04-props" });
  },
);

test(
  "FINDING-038: links inside a rendered Live Preview table open when clicked",
  async () => {
    const src = "top\n\n| Col | Link |\n|---|---|\n| a | [[Target]] |\n| b | [ext](https://example.com/x) |\n\nend\n";
    await withApp({ "T.md": src, "Target.md": "# Target\n" }, async (app) => {
      await app.open("T.md");
      await app.fakeFocus();
      await app.focusEnd(); // cursor on the last line: the table renders
      await app.s.waitFor(`return !!document.querySelector('.cm-lp-table a.internal-link')`);
      const wiki = await app.center(".cm-lp-table a.internal-link");
      await app.clickAt(wiki.x, wiki.y);
      await sleep(800);
      const afterWiki = { tab: await app.activeTab(), hash: await app.exec(`return location.hash`) };
      if (afterWiki.tab !== "T.md") await app.open("T.md");
      await app.fakeFocus();
      await app.focusEnd();
      await app.s.waitFor(`return !!document.querySelector('.cm-lp-table a[href^="https://"]')`);
      const ext = await app.center('.cm-lp-table a[href^="https://"]');
      const appUrl = await app.exec(`return location.href`);
      await app.clickAt(ext.x, ext.y);
      await sleep(1500);
      // If the click navigated the app's own webview, the page (and our
      // instrumentation) is gone: read where the window is now.
      const after = await app.exec(`return { href: location.href, workspace: !!document.querySelector('.workspace'), title: document.title }`);
      const opened = after.workspace ? (await app.invokes("plugin:opener|open_url")).length : 0;
      console.log(`after clicking the https link in the table: ${JSON.stringify(after)} (app was at ${appUrl}); open_url calls: ${opened}`);
      const problems = [];
      if (afterWiki.tab !== "Target.md") problems.push(`clicking [[Target]] in the table left the active tab at ${afterWiki.tab} (location.hash became ${JSON.stringify(afterWiki.hash)})`);
      if (!after.workspace) problems.push(`clicking the https link in the table navigated the app window itself to ${after.href} (title ${JSON.stringify(after.title)}): the Cairn UI is gone`);
      else if (opened === 0) problems.push("clicking the https link in the table did not ask the system to open it (no open_url call)");
      assert.deepEqual(problems, []);
    }, { shot: "ED-05-table-links" });
  },
);

test(
  "FINDING-094: clicking into a bare URL on the line being edited moves the cursor and opens no browser",
  async () => {
    const src = "first line\nsee https://example.com/some/long/page here\nlast line\n";
    await withApp({ "U.md": src }, async (app) => {
      await app.open("U.md");
      await app.fakeFocus();
      // Put the cursor on line 2: the line is now in "source" (editing) state.
      await app.setSel(src.indexOf("see"));
      await sleep(200);
      const shown = await app.exec(`return document.querySelector('.cm-line:nth-child(2)').textContent`);
      assert.equal(shown, "see https://example.com/some/long/page here", "line 2 shows its source");
      // Markdown links and wikilinks on the cursor line are plain text (Ctrl+click opens them);
      // click in the middle of the URL to put the cursor there.
      const c = await app.exec(`
        const el = [...document.querySelectorAll('.cm-line')][1];
        const t = el.firstChild && el.querySelector('[data-url]');
        const r = (t || el).getBoundingClientRect();
        return { x: r.left + r.width * 0.6, y: r.top + r.height / 2, decorated: !!t };`);
      await app.clickAt(c.x, c.y);
      await sleep(500);
      const opened = await app.invokes("plugin:opener|open_url");
      const cur = await app.sel();
      assert.deepEqual(
        { opened: opened.length, cursorMoved: cur.head !== src.indexOf("see") },
        { opened: 0, cursorMoved: true },
        `URL span decorated as a link on the cursor line: ${c.decorated}; open_url calls: ${JSON.stringify(opened)}; cursor head ${cur.head}`,
      );
    }, { shot: "ED-06-bare-url" });
  },
);

test(
  "FINDING-095: Ctrl+I on a selected bold word adds italic and keeps the bold",
  async () => {
    await withApp({ "B.md": "a **bold** b\n" }, async (app) => {
      await app.open("B.md");
      await app.fakeFocus();
      await app.setSel(4, 8); // "bold"
      await app.chord(Key.ctrl, "i");
      await sleep(150);
      const t = await text(app);
      assert.ok(/\*\*\*bold\*\*\*|\*\*_bold_\*\*|_\*\*bold\*\*_/.test(t), `after Ctrl+I the note is ${JSON.stringify(t)} (bold is gone, it is now italic)`);
    }, { shot: "ED-07-italic-on-bold" });
  },
);

test(
  "FINDING-039: embedding a non-image attachment (PDF, archive) does not dump its bytes into the note or block the UI",
  async () => {
    const files = {
      "Pdf.md": "top\n\n![[doc.pdf]]\n\nend\n",
      "Zip.md": "![[backup.zip]]\n\nend\n",
      "doc.pdf": Buffer.concat([Buffer.from("%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\nstream\n"), crypto.randomBytes(30_000)]),
      "backup.zip": crypto.randomBytes(6 * 1024 * 1024),
    };
    await withApp(files, async (app) => {
      const problems = [];
      for (const note of ["Pdf.md", "Zip.md"]) {
        await app.open(note);
        // Measure how long the page's main thread is blocked while the embed fills.
        await app.exec(`window.__gap = 0; let last = performance.now(); clearInterval(window.__tick); window.__tick = setInterval(() => { const n = performance.now(); window.__gap = Math.max(window.__gap, n - last); last = n; }, 20); return 1`);
        await app.fakeFocus();
        await app.focusEnd();
        const t0 = Date.now();
        await app.s.waitFor(`const e = document.querySelector('.cm-lp-embed'); return !!e && e.textContent.trim() !== '…'`, { timeout: 90000, message: `${note}: embed filled` });
        const fillMs = Date.now() - t0;
        await sleep(300);
        const e = await app.exec(`clearInterval(window.__tick); const e = document.querySelector('.cm-lp-embed'); return { gap: Math.round(window.__gap), len: e.textContent.length, head: e.textContent.slice(0, 50), fffd: (e.textContent.match(/\\uFFFD/g) || []).length, height: Math.round(e.getBoundingClientRect().height) }`);
        console.log(`${note}: embed filled after ${fillMs} ms; longest main-thread stall ${e.gap} ms; ${e.len} chars (${e.fffd} U+FFFD), ${e.height}px tall, starts ${JSON.stringify(e.head)}`);
        if (e.len > 2000 || e.fffd > 0) problems.push(`${note}: the embed shows ${e.len} characters of raw file bytes (${e.fffd} U+FFFD, ${e.height}px tall), starting ${JSON.stringify(e.head)}`);
        if (e.gap > 1000) problems.push(`${note}: the UI thread was blocked for ${e.gap} ms while the embed filled`);
      }
      assert.deepEqual(problems, []);
    }, { shot: "ED-08-binary-embed" });
  },
);

test("FINDING-039: a small text file still embeds as text; a PDF or a big file gets a card that links to it", async () => {
  const files = {
    "T.md": "![[notes.txt]]\n\n![[doc.pdf]]\n\n![[big.log]]\n\nend\n",
    "notes.txt": "plain text line\nsecond <b>line</b>\n",
    "doc.pdf": Buffer.from("%PDF-1.4\n%\xe2\xe3\xcf\xd3\n1 0 obj\n<< /Type /Catalog >>\nendobj\n", "latin1"),
    "big.log": "a log line\n".repeat(100_000),
  };
  await withApp(files, async (app) => {
    await app.open("T.md");
    await app.fakeFocus();
    await app.focusEnd();
    const read = (root) =>
      app.exec(
        `return [...document.querySelectorAll(arguments[0] + ' span.embed')].map(e => ({ link: e.querySelector('.embed-title')?.dataset.href ?? null, pre: e.querySelector('pre')?.textContent ?? null, card: e.querySelector('.embed-file')?.textContent ?? null }))`,
        root,
      );
    const want = [
      { link: "notes.txt", pre: "plain text line\nsecond <b>line</b>\n", card: null },
      { link: "doc.pdf", pre: null, card: "PDF file, opens in another app." },
      { link: "big.log", pre: null, card: "LOG file, opens in another app." },
    ];
    const filled = (root) => `const e = [...document.querySelectorAll('${root} span.embed')]; return e.length === 3 && e.every(x => x.dataset.filled && x.textContent.trim() !== '…' && x.querySelector('.embed-title'))`;
    await app.s.waitFor(filled(".cm-lp-embed"), { message: "Live Preview embeds filled" });
    assert.deepEqual(await read(".cm-lp-embed"), want, "Live Preview");
    await app.setMode("preview");
    await app.s.waitFor(filled("article.md-render"), { message: "Reading view embeds filled" });
    assert.deepEqual(await read("article.md-render"), want, "Reading view");
  }, { shot: "ED-08-embed-cards" });
});

test(
  "FINDING-099: clicking a rendered image, note embed or horizontal rule moves the cursor to its source line",
  async () => {
    const src = "intro\n\n---\n\n![[pic.png]]\n\ninline ![[pic.png]] image\n\n![[Inner]]\n\nend\n";
    await withApp({ "W.md": src, "Inner.md": "inner text\n", "pic.png": PNG }, async (app) => {
      await app.open("W.md");
      await app.fakeFocus();
      const results = {};
      for (const [what, css, line] of [
        ["horizontal rule", ".cm-lp-hr", "---"],
        ["block image", ".cm-lp-image-block img", "![[pic.png]]"],
        ["inline image", ".cm-lp-image img", "inline ![[pic.png]] image"],
        ["note embed (body, not its title link)", ".cm-lp-embed .embed-body", "![[Inner]]"],
      ]) {
        await app.focusEnd();
        await sleep(300);
        await app.s.waitFor(`return !!document.querySelector(${JSON.stringify(css)})`, { message: css });
        const c = await app.center(css);
        await app.clickAt(c.x, c.y);
        await sleep(300);
        const cur = await app.cursorLine();
        results[what] = cur.text === line ? "cursor moved to its line" : `cursor stayed on line ${cur.n} ${JSON.stringify(cur.text)}`;
      }
      const stuck = Object.entries(results).filter(([, r]) => r !== "cursor moved to its line");
      assert.deepEqual(stuck, [], JSON.stringify(results));
    }, { shot: "ED-15-widget-clicks" });
  },
);

test(
  "FINDING-195: typing stays fast in notes with many tables (median under 50 ms per keystroke with 300 tables)",
  async () => {
    const table = (i) => `| Name ${i} | Value | Link |\n|---|---|---|\n` + Array.from({ length: 8 }, (_, r) => `| row ${r} | **${r * i}** | [[Target]] |`).join("\n");
    const note = (n) => "# Tables\n\n" + Array.from({ length: n }, (_, i) => `Paragraph ${i} with some text.\n\n${table(i)}\n`).join("\n") + "\nend\n";
    await withApp({ "T60.md": note(60), "T300.md": note(300), "Target.md": "x\n" }, async (app) => {
      const out = {};
      for (const f of ["T60.md", "T300.md"]) {
        await app.open(f);
        await app.fakeFocus();
        await sleep(1500);
        out[f] = await app.exec(`
          const v = document.querySelector('.cm-editor').__cairnView; v.focus();
          v.dispatch({ selection: { anchor: 5 } });
          const t = [];
          for (let k = 0; k < 8; k++) {
            const a = performance.now();
            v.dispatch({ changes: { from: 5 + k, insert: 'x' }, selection: { anchor: 6 + k }, userEvent: 'input.type' });
            t.push(Math.round(performance.now() - a));
          }
          t.sort((a, b) => a - b);
          return { median: t[4], max: t[7] };`);
      }
      console.log(`ms per typed character at the top of the note (only the heading is on screen): ${JSON.stringify(out)}`);
      assert.ok(out["T300.md"].median < 50, `median ${out["T300.md"].median} ms per keystroke with 300 tables (60 tables: ${out["T60.md"].median} ms)`);
    }, { shot: "ED-16-tables-perf" });
  },
);

test(
  "FINDING-043/195: in a 1 MB note with 3,000 tables, tables still render after typing while the parse is behind (in the middle at once, then at the end)",
  async () => {
    const table = (i) => `| Name ${i} | Value | Link |\n|---|---|---|\n` + Array.from({ length: 8 }, (_, r) => `| row ${r} | **${r * i}** | [[Target]] |`).join("\n");
    const note =
      "---\ntitle: big\n---\n# Tables\n\n" +
      Array.from({ length: 3000 }, (_, i) => `Paragraph ${i} with some text to type in.\n\n${table(i)}\n`).join("\n") +
      "\nend\n";
    // Raw table rows on screen (a rendered table is one widget, not lines).
    const rawRows = `
      const v = document.querySelector('.cm-editor').__cairnView;
      const box = v.scrollDOM.getBoundingClientRect();
      return [...v.contentDOM.querySelectorAll('.cm-line')].filter((l) => {
        const r = l.getBoundingClientRect();
        return r.bottom > box.top && r.top < box.bottom && l.textContent.startsWith('|');
      }).length;`;
    // Type at the end of the line that holds pos (a script expression).
    const typeAt = (pos) => `
      const v = document.querySelector('.cm-editor').__cairnView; v.focus();
      const l = v.state.doc.lineAt(${pos});
      v.dispatch({ selection: { anchor: l.to }, scrollIntoView: true });
      for (const ch of 'abcdefghijkl') {
        const at = v.state.selection.main.head;
        v.dispatch({ changes: { from: at, insert: ch }, selection: { anchor: at + 1 }, userEvent: 'input.type' });
      }
      return v.state.doc.lineAt(v.state.selection.main.head).text;`;
    await withApp({ "Big.md": note, "Target.md": "x\n" }, async (app) => {
      await app.open("Big.md");
      await app.fakeFocus();
      // Right after opening, in the middle (the background parse is not
      // there yet), on a paragraph line.
      const typed = await app.exec(typeAt(note.indexOf("Paragraph 1500 ")));
      assert.match(typed, /^Paragraph 1500 .*abcdefghijkl$/);
      await eventually(async () => (await app.exec(rawRows)) === 0, { timeout: 30000, message: "tables around the middle rendered" });
      // At the end of the note.
      assert.equal(await app.exec(typeAt("v.state.doc.length - 1")), "endabcdefghijkl");
      await eventually(async () => (await app.exec(rawRows)) === 0, { timeout: 30000, message: "tables at the end rendered" });
      // And the table under the cursor shows as text while the cursor is in it.
      await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ selection: { anchor: v.state.doc.length - 30 } }); return true`);
      await eventually(async () => (await app.exec(rawRows)) > 0, { message: "the cursor's table shows as text" });
    }, { shot: "ED-16b-tables-after-parse-cut" });
  },
);

test(
  "FINDING-100: after an external change reloads the open note, the cursor stays with its text: the next typed text lands in the right place",
  async () => {
    await withApp({ "S.md": "first line\nsecond line\nthird line\n" }, async (app, env) => {
      await app.open("S.md");
      await app.fakeFocus();
      await app.setSel("first line\nsecond line\nthird line".length); // end of "third line"
      // Another device / git pull adds a line at the top while the note is open and clean.
      env.vault.write("S.md", "NEW TOP LINE\nfirst line\nsecond line\nthird line\n");
      await eventually(async () => (await app.text()).startsWith("NEW TOP"), { message: "clean tab reloaded" });
      await app.keys("!");
      await sleep(200);
      const t = await app.text();
      assert.equal(t, "NEW TOP LINE\nfirst line\nsecond line\nthird line!\n", `typed "!" landed here: ${JSON.stringify(t)}`);
    }, { shot: "ED-17-cursor-after-reload" });
  },
);

test(
  "FINDING-196: Ctrl+L (toggle checkbox) on a quote or heading line does not put '- [ ] ' in front of the '>' / '#'",
  async () => {
    const src = "> quoted line\n# Heading\n";
    await withApp({ "F.md": src }, async (app) => {
      await app.open("F.md");
      await app.fakeFocus();
      await app.setSel(0, src.length - 1); // both lines
      await app.chord(Key.ctrl, "l");
      await sleep(150);
      const t = await text(app);
      assert.ok(!/^- \[ \] [>#]/m.test(t), `after Ctrl+L: ${JSON.stringify(t)} (expected e.g. "> - [ ] quoted line")`);
    }, { shot: "ED-18-task-on-quote" });
  },
);

test(
  "FINDING-198: Ctrl+click on a link opens the same tabs whether or not the cursor is on the link's line",
  async () => {
    const src = "first line\nsee [[Target]] here\nlast line\n";
    await withApp({ "L.md": src, "Target.md": "# Target\n" }, async (app) => {
      const ctrlClickLink = async () => {
        const c = await app.exec(`
          const v = document.querySelector('.cm-editor').__cairnView;
          const pos = v.state.doc.toString().indexOf('Target') + 2;
          const r = v.coordsAtPos(pos);
          return { x: r.left + 1, y: (r.top + r.bottom) / 2 };`);
        await app.s.cmd("POST", "/actions", {
          actions: [
            { type: "key", id: "kb", actions: [{ type: "keyDown", value: Key.ctrl }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "keyUp", value: Key.ctrl }] },
            { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", x: Math.round(c.x), y: Math.round(c.y) }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }, { type: "pause", duration: 0 }] },
          ],
        });
        await app.s.cmd("DELETE", "/actions");
        await sleep(600);
      };
      await app.open("L.md");
      await app.fakeFocus();
      // Cursor elsewhere: the link is rendered.
      await app.setSel(0);
      await sleep(150);
      await ctrlClickLink();
      const rendered = await app.tabs();
      // Back to one tab, cursor on the link's line: the link shows as source.
      while ((await app.tabs()).length > 1) await app.exec(`document.querySelector('[data-testid=tab][aria-selected=true] .close, [data-testid=tab][aria-selected=true] button')?.click(); return 1`), await sleep(200);
      if ((await app.activeTab()) !== "L.md") await app.open("L.md");
      await app.fakeFocus();
      await app.setSel(src.indexOf("see"));
      await sleep(150);
      await ctrlClickLink();
      const source = await app.tabs();
      assert.deepEqual(source, rendered, `tabs after Ctrl+click: rendered link -> ${JSON.stringify(rendered)}, same link on the cursor line -> ${JSON.stringify(source)}`);
    }, { shot: "ED-20-ctrl-click" });
  },
);

// ---------------------------------------------------------------- held up

const WEIRD = [
  "[[Target]](http://x.com)",
  "[![[Target]]](http://x.com)",
  "**[[Target|**bold alias**]]**",
  "*unclosed emphasis [[Target]]",
  "Some *emphasis with [a link](https://example.com) inside* and **bold [[Target]] wiki**.",
  "A [md link with [[Target]] inside](Target.md) here.",
  "[[Target#Sec|alias with [brackets]]]",
  "![[Target|300]] inline embed in text",
  "text ![img](missing.png) and ![[nope.png]] inline",
  "`code [[Target]]` and [[Target]] after",
  "<span>[[Target]]</span>",
  "[link](<http://x.com/a b>)",
  "#tag-at-start and middle #tag/nested and not#tag and #123",
  "https://example.com/path?q=1#frag bare url and <https://example.com> autolink",
  "~~strike [[Target]] strike~~ ==highlight==",
  "- [ ] [[Target]] task with link",
  "\t- [x] tab-indented done",
  "> quote [[Target]]",
  "> > nested quote",
  "***",
  "Setext heading",
  "==============",
  "Escaped \\*not emphasis\\* and \\[[notwiki]]",
  "| C1 | C2 |",
  "|---|---|",
  "| [[Target\\|alias]] | x |",
  "| [[Target|bad]] | y |",
  "",
  "```",
  "code [[Target]] and ![[Target]]",
  "```",
  "עברית [[Target]] עם קישור **מודגש** ו-English",
  "中文 [[Target]] 日本語 **太字** 한국어",
  "emoji 👨‍👩‍👧‍👦 [[Target|🎉 party]] é combining ä́",
  "```",
  "unclosed fence [[Target]] and ![[Target]]",
  "",
  "last",
].join("\n");

test("Live Preview: nested/malformed Markdown, RTL, CJK, emoji: no errors, and the cursor line always shows the exact source (held up)", async () => {
  await withApp({ "W.md": WEIRD, "Target.md": "# Target\n\n## Sec\n" }, async (app) => {
    await app.open("W.md");
    await app.fakeFocus();
    const out = await app.exec(`
      const v = document.querySelector('.cm-editor').__cairnView; v.focus();
      const bad = [];
      for (let i = 1; i <= v.state.doc.lines; i++) {
        const l = v.state.doc.line(i);
        for (const p of [l.from, Math.floor((l.from + l.to) / 2), l.to]) v.dispatch({ selection: { anchor: p } });
        const dom = document.querySelectorAll('.cm-line');
        // find the DOM line for this doc line through posAtDOM
        const el = [...dom].find((d) => { try { return v.state.doc.lineAt(v.posAtDOM(d)).number === i; } catch { return false; } });
        const shown = el ? el.textContent : null;
        if (shown !== l.text) bad.push([i, l.text, shown]);
      }
      return { bad, docLen: v.state.doc.length };
    `);
    assert.deepEqual(out.bad, [], "lines whose source is not shown verbatim while the cursor is on them");
    assert.equal(await text(app), WEIRD, "rendering changed the document");
    assert.deepEqual(await app.errors(), [], "page errors while rendering");
    // The escaped wikilink in a table cell renders as a link that opens Target.
    await app.setSel(0);
    await app.s.waitFor(`return [...document.querySelectorAll('.cm-lp-table a.internal-link')].some(a => a.textContent === 'alias')`);
    // Code blocks (closed and unclosed) and inline code get no link / embed rendering.
    await app.blur();
    await sleep(200);
    const inCode = await app.exec(`
      const lines = [...document.querySelectorAll('.cm-line')];
      return lines.filter(l => /code \\[\\[Target\\]\\] and !\\[\\[Target\\]\\]|unclosed fence/.test(l.textContent))
        .map(l => ({ text: l.textContent, links: l.querySelectorAll('.cm-lp-link, .cm-lp-embed, .cm-lp-image').length }))
        .concat([...document.querySelectorAll('.cm-lp-embed, .cm-lp-image-block')].map(e => ({ text: 'block widget: ' + e.textContent.slice(0, 40), links: 1 })));`);
    assert.deepEqual(inCode.filter((l) => l.links), [], JSON.stringify(inCode));
    assert.equal(inCode.length, 2, JSON.stringify(inCode));
  }, { shot: "ED-weird" });
});

test("Live Preview checkboxes: nested, ordered, quoted, [X], * and + items toggle exactly their own line; code blocks have none; undo restores (held up)", async () => {
  const src = "- [ ] one\n  - [ ] nested two\n1. [ ] ordered\n> - [ ] quoted\n- [X] upper\n* [ ] star\n+ [ ] plus\n\n```\n- [ ] in code\n```\n";
  await withApp({ "Tasks.md": src }, async (app, env) => {
    await app.open("Tasks.md");
    await app.blur();
    await sleep(200);
    const n = await app.exec(`return document.querySelectorAll('.cm-lp-task').length`);
    assert.equal(n, 7, "one checkbox per task item, none in the code block");
    const lines = src.split("\n");
    const expected = [...lines];
    for (let i = 0; i < 7; i++) {
      const c = await app.center(".cm-lp-task", i);
      await app.clickAt(c.x, c.y);
      await sleep(150);
      expected[i] = /\[ \]/.test(expected[i]) ? expected[i].replace("[ ]", "[x]") : expected[i].replace(/\[[xX]\]/, "[ ]");
      assert.equal(await text(app), expected.join("\n"), `after clicking checkbox ${i}`);
    }
    await eventually(() => env.vault.read("Tasks.md") === expected.join("\n"), { message: "toggles saved" });
    // Undo the last toggle (Ctrl+Z goes to the editor even after widget clicks).
    await app.exec(`document.querySelector('.cm-editor').__cairnView.focus(); return 1`);
    await app.chord(Key.ctrl, "z");
    expected[6] = lines[6];
    assert.equal(await text(app), expected.join("\n"), "undo reverted exactly the last toggle");
  }, { shot: "ED-tasks" });
});

test("formatting commands (Ctrl+B, Ctrl+I, Ctrl+L, Ctrl+K, Ctrl+Shift+C) each undo back to the exact text (held up)", async () => {
  const src = "alpha beta gamma\nsecond line\n";
  await withApp({ "F.md": src }, async (app) => {
    await app.open("F.md");
    await app.fakeFocus();
    for (const [keys, expectAfter] of [
      [[Key.ctrl, "b"], "alpha **beta** gamma\nsecond line\n"],
      [[Key.ctrl, "i"], "alpha *beta* gamma\nsecond line\n"],
      [[Key.ctrl, "l"], "- [ ] alpha beta gamma\nsecond line\n"],
      [[Key.ctrl, "k"], "alpha [[beta]] gamma\nsecond line\n"],
      [[Key.ctrl, Key.shift, "c"], "alpha `beta` gamma\nsecond line\n"],
    ]) {
      await app.setSel(6, 10); // "beta"
      await app.chord(...keys);
      await sleep(100);
      assert.equal(await text(app), expectAfter, `after ${JSON.stringify(keys)}`);
      await app.chord(Key.ctrl, "z");
      await sleep(100);
      assert.equal(await text(app), src, `undo after ${JSON.stringify(keys)}`);
    }
  }, { shot: "ED-format" });
});

// Synthetic IME composition: the events and DOM mutations WebKit produces
// for an input method (WebDriver cannot drive a real IME).
const IME = `
  const done = arguments[arguments.length - 1];
  const [at, steps] = arguments;
  (async () => {
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const v = document.querySelector('.cm-editor').__cairnView;
    v.focus();
    v.dispatch({ selection: { anchor: at } });
    await sleep(60);
    const cd = v.contentDOM;
    cd.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true, data: '' }));
    let { node, offset } = v.domAtPos(at);
    if (node.nodeType !== 3) {
      const t = node.childNodes[offset];
      if (t && t.nodeType === 3) { node = t; offset = 0; }
      else { const tn = document.createTextNode(''); node.insertBefore(tn, t || null); node = tn; offset = 0; }
    }
    const start = offset;
    let prev = '';
    for (const s of steps) {
      cd.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, inputType: 'insertCompositionText', data: s, isComposing: true }));
      node.data = node.data.slice(0, start) + s + node.data.slice(start + prev.length);
      prev = s;
      document.getSelection().collapse(node, start + s.length);
      cd.dispatchEvent(new CompositionEvent('compositionupdate', { bubbles: true, data: s }));
      cd.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertCompositionText', data: s, isComposing: true }));
      await sleep(60);
    }
    cd.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: prev }));
    cd.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromComposition', data: prev, isComposing: false }));
    await sleep(250);
    done({ doc: v.state.doc.toString() });
  })().catch((e) => done({ err: String(e && e.stack || e) }));
`;

test("IME composition (synthetic) in Live Preview: text lands once, at the cursor, next to hidden syntax and inside marks (held up)", async () => {
  const src = "Hello [[Target]] #tag world\n**bold text** end\n";
  await withApp({ "I.md": src, "Target.md": "x\n" }, async (app, env) => {
    await app.open("I.md");
    await app.fakeFocus();
    let r = await app.s.execAsync(IME, 27, ["に", "日", "日本"]);
    assert.equal(r.doc, "Hello [[Target]] #tag world日本\n**bold text** end\n", JSON.stringify(r));
    // composition starting on a line whose syntax was hidden until now
    r = await app.s.execAsync(IME, 34, ["k", "か", "漢字"]);
    assert.equal(r.doc, "Hello [[Target]] #tag world日本\n**bo漢字ld text** end\n", JSON.stringify(r));
    // inside the wikilink mark and right before a tag
    r = await app.s.execAsync(IME, 10, ["ä", "äö"]);
    r = await app.s.execAsync(IME, 19, ["ü", "üß"]);
    assert.equal(r.doc, "Hello [[Taäörget]] üß#tag world日本\n**bo漢字ld text** end\n", JSON.stringify(r));
    await eventually(() => env.vault.read("I.md") === r.doc, { message: "composed text saved" });
    // Right after "#" (a tag mark grows around the composition) and at the end
    // of a bare URL (link mark on the cursor line), then inside a wikilink in source mode.
    await app.setSel(r.doc.length);
    await app.keys("tag #");
    let at = (await text(app)).length;
    r = await app.s.execAsync(IME, at, ["に", "にほ", "日本"]);
    assert.ok(r.doc.endsWith("tag #日本"), JSON.stringify(r));
    await app.keys(" https://x.com/");
    at = (await text(app)).length;
    r = await app.s.execAsync(IME, at, ["か", "漢字"]);
    assert.ok(r.doc.endsWith("tag #日本 https://x.com/漢字"), JSON.stringify(r));
    await app.setMode("source");
    r = await app.s.execAsync(IME, r.doc.indexOf("get]]") + 2, ["é", "éè"]);
    assert.ok(r.doc.startsWith("Hello [[Taäörgeéèt]]"), JSON.stringify(r));
    assert.deepEqual(await app.errors(), []);
  }, { shot: "ED-ime" });
});

test("a 100,000-character line with 6,250 wikilinks: opens, stays editable, no errors (held up)", async () => {
  const long = "start **bold** " + "word [[Target]] ".repeat(6250) + " end\n\nsecond line\n";
  await withApp({ "Long.md": long, "Target.md": "x\n" }, async (app, env) => {
    const t0 = Date.now();
    await app.open("Long.md");
    const openMs = Date.now() - t0;
    await app.fakeFocus();
    const timing = await app.exec(`
      const v = document.querySelector('.cm-editor').__cairnView; v.focus();
      const t = [];
      for (const p of [5, 50000, v.state.doc.line(1).to]) {
        const a = performance.now();
        v.dispatch({ selection: { anchor: p } });
        v.dispatch({ changes: { from: p, insert: 'x' }, selection: { anchor: p + 1 }, userEvent: 'input.type' });
        t.push(Math.round(performance.now() - a));
      }
      const a = performance.now();
      v.dispatch({ selection: { anchor: v.state.doc.length } });
      t.push(Math.round(performance.now() - a));
      return t;
    `);
    console.log(`open ${openMs} ms; edit in long line / leave it: ${JSON.stringify(timing)} ms`);
    assert.ok(openMs < 8000, `open took ${openMs} ms`);
    assert.ok(Math.max(...timing) < 400, `an edit took ${Math.max(...timing)} ms`);
    await eventually(() => env.vault.read("Long.md").length === long.length + 3, { timeout: 8000, message: "saved" });
    assert.deepEqual(await app.errors(), []);
  }, { shot: "ED-long" });
});

test("embeds: self-embed, mutual embeds, missing note/heading/block render a message and nothing hangs (held up)", async () => {
  await withApp(
    {
      "Self.md": "Self text\n\n![[Self]]\n",
      "MutA.md": "A text\n\n![[MutB]]\n",
      "MutB.md": "B text\n\n![[MutA]]\n",
      "Embeds.md": "top\n\n![[Missing]]\n\n![[Target#Nope]]\n\n![[Target#^nope]]\n\n![[Target#Sec]]\n\n![[Target#^blk1]]\n",
      "Target.md": "# Target\n\n## Sec\n\nsection text ^blk1\n",
      "Chain0.md": "zero\n\n![[Chain1]]\n",
      "Chain1.md": "one\n\n![[Chain2]]\n",
      "Chain2.md": "two\n\n![[Chain3]]\n",
      "Chain3.md": "three\n\n![[Chain4]]\n",
      "Chain4.md": "four\n",
    },
    async (app) => {
      // A chain of embeds stops after 3 levels with a message.
      await app.open("Chain0.md");
      await app.s.waitFor(`return /skipped/.test(document.querySelector('.cm-lp-embed')?.textContent || '')`);
      const chain = await app.exec(`return document.querySelector('.cm-lp-embed').textContent`);
      assert.match(chain, /one[\s\S]*two[\s\S]*three[\s\S]*Chain4 skipped/);
      assert.doesNotMatch(chain, /four/);
      await app.open("Self.md");
      await app.s.waitFor(`return /skipped/.test(document.querySelector('.cm-lp-embed')?.textContent || '')`);
      await app.open("MutA.md");
      await app.s.waitFor(`return /skipped/.test(document.querySelector('.cm-lp-embed')?.textContent || '')`);
      await app.open("Embeds.md");
      await app.s.waitFor(`return document.querySelectorAll('.cm-lp-embed').length === 5 && ![...document.querySelectorAll('.cm-lp-embed')].some(e => e.textContent.trim() === '…')`, { timeout: 8000 });
      const shown = await app.exec(`return [...document.querySelectorAll('.cm-lp-embed')].map(e => e.textContent.trim())`);
      assert.match(shown[0], /does not exist/);
      assert.match(shown[1], /not found/);
      assert.match(shown[2], /not found/);
      assert.match(shown[3], /section text/);
      assert.match(shown[4], /section text/);
      assert.deepEqual(await app.errors(), []);
    },
    { shot: "ED-embeds" },
  );
});

test("frontmatter edge cases (BOM, empty, frontmatter only, unclosed, invalid YAML, --- later in the body) render without errors and never change the file (held up)", async () => {
  const notes = {
    "Bom.md": "\uFEFF---\ntitle: bom\n---\nbody after bom\n",
    "Empty.md": "---\n---\nbody after empty frontmatter\n",
    "Only.md": "---\ntitle: only\n---",
    "Unclosed.md": "---\ntitle: unclosed\nmore: x\n\nbody unclosed\n",
    "Invalid.md": "---\nkey: [unclosed\nother: value\n---\nbody after invalid yaml\n",
    "Later.md": "intro paragraph\n\n---\ntitle: not frontmatter\n---\n\nend\n",
  };
  await withApp(notes, async (app, env) => {
    for (const [name, src] of Object.entries(notes)) {
      await app.open(name);
      await app.fakeFocus();
      await app.focusEnd();
      await sleep(150);
      await app.blur();
      await sleep(150);
      assert.equal(await text(app), src, `${name}: editor text equals the file`);
      // Walk the cursor over every line: no exceptions.
      await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); for (let i = 1; i <= v.state.doc.lines; i++) v.dispatch({ selection: { anchor: v.state.doc.line(i).from } }); return 1`);
      const visible = await app.exec(`return document.querySelector('.cm-content').innerText`);
      for (const word of ["body after", "body unclosed", "intro paragraph", "end"]) {
        if (src.includes(word)) assert.ok(visible.includes(word), `${name}: "${word}" visible`);
      }
    }
    await sleep(800);
    for (const [name, src] of Object.entries(notes)) assert.equal(env.vault.read(name), src, `${name} untouched on disk`);
    assert.deepEqual(await app.errors(), []);
  }, { shot: "ED-frontmatter" });
});

test("cursor keys over hidden syntax: End goes to the real end of a line with a hidden link and bold, typing lands there (held up)", async () => {
  const src = "first line\nSome **bold** and [[Target|alias]] and [md](http://x.com) end\nthird\n";
  await withApp({ "C.md": src, "Target.md": "x\n" }, async (app) => {
    await app.open("C.md");
    await app.fakeFocus();
    await app.setSel(3);
    await app.keys(Key.down);
    let cur = await app.cursorLine();
    assert.equal(cur.n, 2);
    await app.keys(""); // End
    cur = await app.cursorLine();
    assert.equal(cur.head, src.indexOf(" end\n") + 4, "End goes to the end of the source line");
    await app.keys("!");
    assert.equal(await text(app), src.replace(" end\n", " end!\n"));
    // Shift+Home selects the whole source line; typing replaces exactly that line.
    await app.s.keys({ chord: [Key.shift, ""] });
    await app.keys("X");
    assert.equal(await text(app), "first line\nX\nthird\n");
  }, { shot: "ED-cursor" });
});
