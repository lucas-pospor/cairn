// Cairn calls the folder of notes a notebook. This test fails when the old
// word "vault" comes back in text a user can read: markup, user-facing
// attributes and string literals in the Svelte and TypeScript code, and string
// literals in the Rust crates, the Tauri shell and the Android code (error
// messages reach the UI). The code keeps the old word in its names (Vault,
// vaultId, app.vault, vault://, the sync API), so only strings are checked,
// and ALLOWED lists the strings that are names, not text.

import { readdirSync, readFileSync } from "node:fs";
import { parse } from "svelte/compiler";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const ROOT = new URL("../../../", import.meta.url);
const WORD = /\bvaults?\b/i;

/** Strings with the old word that are names, not text, and why they keep it. */
const ALLOWED: { text: string | RegExp; why: string }[] = [
  { text: "vault", why: 'the Icon name (Icon name="vault") and the name of the vault:// scheme' },
  { text: "vaults", why: "the folder that holds the notebooks in app storage on Android; renaming it would lose them" },
  { text: "app:close-vault", why: "a command id; hotkeys in .cairn/settings.json are saved under it" },
  { text: "vault-changed", why: "the name of the Tauri event for changes on disk" },
  { text: /^(vault:\/\/|http:\/\/vault\.localhost\/)/, why: "the vault:// scheme that serves the notebook's files" },
  { text: "cairn-vault-key-v1", why: "associated data of the key envelope; another value cannot open existing keys" },
  { text: /^\/vaults\//, why: "paths of the sync server's API" },
  { text: /^\s*(PRAGMA|SELECT|INSERT|CREATE)\b/, why: "SQL for the sync server's tables" },
  { text: "no such vault", why: "the sync server's error body, which older apps match; the app shows it as no such notebook" },
  { text: "vault exists", why: "the sync server's error body; the app shows it in its own words" },
];

interface Hit {
  file: string;
  line: number;
  text: string;
}

function files(dir: string, ext: RegExp): string[] {
  return readdirSync(new URL(dir, ROOT), { recursive: true })
    .filter((p) => ext.test(p))
    .map((p) => dir + p);
}

const read = (file: string) => readFileSync(new URL(file, ROOT), "utf8");

/** The line number of a position in `src`. */
function lineFinder(src: string): (pos: number) => number {
  const starts = [0];
  for (let i = src.indexOf("\n"); i >= 0; i = src.indexOf("\n", i + 1)) starts.push(i + 1);
  return (pos) => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= pos) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  };
}

/** String literals and the text parts of template literals. */
function typescriptStrings(file: string, src: string): Hit[] {
  const out: Hit[] = [];
  const lineAt = lineFinder(src);
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const visit = (n: ts.Node) => {
    if (ts.isImportDeclaration(n) || ts.isExportDeclaration(n) || ts.isLiteralTypeNode(n)) return;
    if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n) || ts.isTemplateLiteralToken(n)) {
      out.push({ file, line: lineAt(n.getStart(sf)), text: n.text });
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return out;
}

/** Attributes whose values are names for code, never shown. */
const CODE_ATTRS = new Set(["class", "data-testid", "id", "for", "href", "src", "style"]);

/** Markup text, attribute values, and string literals in scripts and in {expressions}. */
function svelteStrings(file: string, src: string): Hit[] {
  const out: Hit[] = [];
  const lineAt = lineFinder(src);
  const ast = parse(src, { filename: file, modern: true });
  const visit = (node: unknown, inCodeAttr: boolean): void => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) return node.forEach((c) => visit(c, inCodeAttr));
    const n = node as Record<string, unknown>;
    if (n.type === "ImportDeclaration" || n.type === "TSLiteralType") return;
    if (n.type === "Attribute" && CODE_ATTRS.has(n.name as string)) inCodeAttr = true;
    let text: string | null = null;
    if (n.type === "Text") text = n.data as string;
    else if (n.type === "Literal" && typeof n.value === "string") text = n.value;
    else if (n.type === "TemplateElement") text = (n.value as { cooked?: string; raw: string }).cooked ?? (n.value as { raw: string }).raw;
    if (text !== null && !inCodeAttr) out.push({ file, line: lineAt(n.start as number), text: text.trim().replace(/\s+/g, " ") });
    for (const [k, v] of Object.entries(n)) if (k !== "parent" && k !== "metadata") visit(v, inCodeAttr);
  };
  visit(ast.fragment, false);
  visit(ast.instance, false);
  visit(ast.module, false);
  return out;
}

/**
 * String literals of Rust or Kotlin source, without comments, char literals
 * and, in Rust, items under #[cfg(test)]. Format placeholders such as {vault}
 * (Rust) and $name or ${...} (Kotlin) become a space.
 */
function codeStrings(file: string, src: string, lang: "rs" | "kt"): Hit[] {
  const out: Hit[] = [];
  const lineAt = lineFinder(src);
  const push = (pos: number, raw: string) => {
    const text = lang === "rs" ? raw.replace(/\{\{|\}\}/g, "").replace(/\{[^{}]*\}/g, " ") : raw.replace(/\$[A-Za-z_]\w*/g, " ");
    out.push({ file, line: lineAt(pos), text: WORD.test(text) ? raw : text });
  };
  let depth = 0;
  let testItem = false; // after #[cfg(test)], until its item ends
  let testDepth = -1; // brace depth at which the test item's body started
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (src.startsWith("//", i)) {
      const end = src.indexOf("\n", i);
      i = end < 0 ? src.length : end;
    } else if (src.startsWith("/*", i)) {
      let d = 1;
      i += 2;
      while (i < src.length && d) {
        if (src.startsWith("/*", i)) d++;
        else if (src.startsWith("*/", i)) d--;
        else {
          i++;
          continue;
        }
        i += 2;
      }
    } else if (lang === "rs" && src.startsWith("#[cfg(test)]", i)) {
      testItem = true;
      i += "#[cfg(test)]".length;
    } else if (c === "{") {
      if (testItem && testDepth < 0) testDepth = depth;
      depth++;
      i++;
    } else if (c === "}") {
      depth--;
      if (depth === testDepth) {
        testItem = false;
        testDepth = -1;
      }
      i++;
    } else if (c === ";" && testItem && testDepth < 0) {
      testItem = false;
      i++;
    } else if (lang === "rs" && /[rb]/.test(c) && !/\w/.test(src[i - 1] ?? " ") && /^(br|rb|r)(#*)"/.test(src.slice(i, i + 8))) {
      const [open, , hashes] = /^(br|rb|r)(#*)"/.exec(src.slice(i, i + 8))!;
      const end = src.indexOf('"' + hashes, i + open.length);
      if (!testItem) push(i, src.slice(i + open.length, end));
      i = end + 1 + hashes.length;
    } else if (lang === "kt" && src.startsWith('"""', i)) {
      const end = src.indexOf('"""', i + 3);
      push(i, src.slice(i + 3, end));
      i = end + 3;
    } else if (c === '"') {
      let j = i + 1;
      let text = "";
      while (j < src.length && src[j] !== '"') {
        if (src[j] === "\\") {
          text += src[j + 1];
          j += 2;
        } else if (lang === "kt" && src.startsWith("${", j)) {
          let d = 1;
          for (j += 2; j < src.length && d; j++) d += src[j] === "{" ? 1 : src[j] === "}" ? -1 : 0;
          text += " ";
        } else text += src[j++];
      }
      if (!testItem) push(i, text);
      i = j + 1;
    } else if (c === "'") {
      // a char literal ('a', '\n', '\'') or a Rust lifetime ('a)
      if (src[i + 1] === "\\") i = src.indexOf("'", i + 3) + 1;
      else {
        const width = (src.codePointAt(i + 1) ?? 0) > 0xffff ? 2 : 1;
        i += src[i + 1 + width] === "'" ? 2 + width : 1;
      }
    } else i++;
  }
  return out;
}

/** The text of each <string> in Android's res/values files. */
function androidResourceStrings(file: string, src: string): Hit[] {
  const lineAt = lineFinder(src);
  return [...src.matchAll(/<string\b[^>]*>([^<]*)<\/string>/g)].map((m) => ({ file, line: lineAt(m.index), text: m[1] }));
}

function allStrings(): Hit[] {
  const hits: Hit[] = [];
  const isTestFile = /\.test\.ts$|\.testutil\.ts$|\.d\.ts$/;
  for (const f of files("app/src/", /\.svelte$/)) hits.push(...svelteStrings(f, read(f)));
  for (const f of files("app/src/", /\.(ts|js)$/)) if (!isTestFile.test(f)) hits.push(...typescriptStrings(f, read(f)));
  for (const dir of ["crates/cairn-core/src/", "crates/cairn-sync/src/", "crates/cairn-server/src/", "app/src-tauri/src/"])
    for (const f of files(dir, /\.rs$/)) hits.push(...codeStrings(f, read(f), "rs"));
  const android = "app/src-tauri/gen/android/app/src/main/";
  for (const f of files(android, /\.(kt|java)$/)) hits.push(...codeStrings(f, read(f), "kt"));
  for (const f of files(android, /^res\/values[^/]*\/[^/]+\.xml$/)) hits.push(...androidResourceStrings(f, read(f)));
  return hits;
}

const allowed = (h: Hit) => ALLOWED.find((a) => (typeof a.text === "string" ? h.text === a.text : a.text.test(h.text)));

describe("the word for the folder of notes", () => {
  const hits = allStrings();
  const old = hits.filter((h) => WORD.test(h.text));

  it("finds the strings it checks", () => {
    // A scanner that broke would find nothing and pass.
    const texts = hits.map((h) => h.text);
    expect(texts).toContain("Open folder as notebook");
    expect(texts).toContain("the notebook folder looks empty or missing, so nothing was synced. If you deleted every note on purpose, add a note and sync again");
    expect(texts).toContain("Cairn");
    expect(old.length).toBeGreaterThan(0);
  });

  it("is notebook, not vault, in every string a user can read", () => {
    const shown = old.filter((h) => !allowed(h)).map((h) => `${h.file}:${h.line}: ${JSON.stringify(h.text)}`);
    // Say notebook in text. A string that is a name for code goes in ALLOWED, with the reason.
    expect(shown).toEqual([]);
  });

  it("allows only names that are still in the code", () => {
    const unused = ALLOWED.filter((a) => !old.some((h) => allowed(h) === a)).map((a) => String(a.text));
    expect(unused).toEqual([]);
  });
});
