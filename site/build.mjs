#!/usr/bin/env node
// Builds the Cairn Notes website, a folder of static pages with no scripts:
//
//   node site/build.mjs [--out target/site] [--release v1.3.1 | --release latest]
//                       [--release-json release.json] [--sums SHA256SUMS]
//
// The home and download pages come from site/home.html and site/download.html.
// The manual comes from the Markdown in docs/manual, in the order of the list
// in docs/manual/README.md, and the release notes from docs/RELEASE_NOTES.md.
// Every page shares site/template.html.
//
// The version, the release date and every file of the release, with its size
// and SHA-256, come from the GitHub release when the site is built: by default
// the release of the version in app/src-tauri/tauri.conf.json, or with
// --release latest the latest release (CI uses that, since main can be ahead of
// it). --release-json and --sums read the GitHub API's answer and SHA256SUMS
// from files instead, for a build without network.
//
// The build fails if the release lacks a file or has one the download page
// does not list, if a SHA-256 differs from SHA256SUMS, if the copies of the
// APK's signing fingerprint differ, or if a link or image between the pages
// leads nowhere.
//
// It uses markdown-it, which the app already depends on, so run `npm ci` in
// app/ first. GITHUB_TOKEN, if set, raises the GitHub API's rate limit.

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";

const ROOT = path.resolve(import.meta.dirname, "..");
const REPO = "lucas-pospor/cairn";
const GITHUB = `https://github.com/${REPO}`;
const MARKER = ".cairn-site";
const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

class BuildError extends Error {}
const fail = (message) => {
  throw new BuildError(message);
};

const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// --- Options ---

const argv = process.argv.slice(2);
const options = {};
for (let i = 0; i < argv.length; i += 2) {
  const name = argv[i];
  if (!["--out", "--release", "--release-json", "--sums"].includes(name)) fail(`unknown option ${name}`);
  if (argv[i + 1] === undefined || argv[i + 1].startsWith("--")) fail(`${name} needs a value`);
  options[name.slice(2)] = argv[i + 1];
}

// --- The release ---

const appVersion = JSON.parse(read("app/src-tauri/tauri.conf.json")).version;
const wanted = options.release ?? `v${appVersion}`;

async function getJson(url) {
  const headers = { Accept: "application/vnd.github+json", "User-Agent": "cairn-site-build" };
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (token) headers.Authorization = `Bearer ${token}`;
  const res = await fetch(url, { headers });
  if (!res.ok) fail(`${url} answered ${res.status}: ${(await res.text()).slice(0, 300)}`);
  return res.json();
}

async function getText(url) {
  const res = await fetch(url, { headers: { "User-Agent": "cairn-site-build" } });
  if (!res.ok) fail(`${url} answered ${res.status}`);
  return res.text();
}

// Each file of a release, and how the download page lists it.
const KINDS = [
  { key: "deb", match: /\.deb$/, group: "linux", label: "For Debian and Ubuntu" },
  { key: "rpm", match: /\.rpm$/, group: "linux", label: "For Fedora" },
  { key: "appimage", match: /\.AppImage$/, group: "linux", label: "For other distributions" },
  { key: "exe", match: /-setup\.exe$/, group: "windows", label: "Installer" },
  { key: "apk", match: /\.apk$/, group: "android", label: "For phones, tablets and emulators" },
  { key: "server", match: /-docker-image\.tar\.gz$/, group: "server", label: "Docker image, for x86_64" },
  { key: "sums", match: /^SHA256SUMS$/, group: "sums", label: "SHA-256 of the files above" },
];

async function loadRelease() {
  const release = options["release-json"]
    ? JSON.parse(fs.readFileSync(options["release-json"], "utf8"))
    : await getJson(
        wanted === "latest"
          ? `https://api.github.com/repos/${REPO}/releases/latest`
          : `https://api.github.com/repos/${REPO}/releases/tags/${encodeURIComponent(wanted)}`,
      );
  if (release.draft || release.prerelease) fail(`${release.tag_name} is a draft or a prerelease`);
  const version = release.tag_name.replace(/^v/, "");
  if (wanted !== "latest" && version !== appVersion) {
    fail(`release ${release.tag_name} is not version ${appVersion} of app/src-tauri/tauri.conf.json`);
  }

  const assets = {};
  for (const a of release.assets) {
    const kind = KINDS.find((k) => k.match.test(a.name));
    if (!kind) fail(`release ${release.tag_name} has a file the download page does not list: ${a.name}`);
    if (assets[kind.key]) fail(`release ${release.tag_name} has two ${kind.key} files: ${assets[kind.key].name} and ${a.name}`);
    const sha = /^sha256:([0-9a-f]{64})$/.exec(a.digest ?? "")?.[1];
    if (!sha) fail(`the GitHub API gives no SHA-256 for ${a.name}`);
    assets[kind.key] = { kind, name: a.name, size: a.size, sha, url: a.browser_download_url };
  }
  for (const k of KINDS) if (!assets[k.key]) fail(`release ${release.tag_name} has no ${k.key} file`);

  // SHA256SUMS must list every other file with the digest GitHub reports.
  const sumsText = options.sums ? fs.readFileSync(options.sums, "utf8") : await getText(assets.sums.url);
  const sums = new Map();
  for (const line of sumsText.split("\n")) {
    if (!line.trim()) continue;
    const m = /^([0-9a-f]{64}) [ *](.+)$/.exec(line);
    if (!m) fail(`SHA256SUMS has a line that is not a checksum: ${line}`);
    sums.set(m[2], m[1]);
  }
  for (const a of Object.values(assets)) {
    if (a.kind.key === "sums") continue;
    if (sums.get(a.name) !== a.sha) fail(`SHA256SUMS does not give ${a.name} the SHA-256 that GitHub reports`);
    sums.delete(a.name);
  }
  if (sums.size) fail(`SHA256SUMS lists files that are not in the release: ${[...sums.keys()].join(", ")}`);

  const d = new Date(release.published_at);
  const released = `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
  return { tag: release.tag_name, version, released, url: release.html_url, body: release.body ?? "", assets };
}

// The APK's signing fingerprint is written in the Android page of the manual.
// The release notes and the release's own text must give the same one.
function signingFingerprint(release) {
  const re = /\b[0-9A-F]{2}(?::[0-9A-F]{2}){31}\b/g;
  const inManual = read("docs/manual/android.md").match(re) ?? [];
  if (!inManual.length) fail("docs/manual/android.md gives no signing fingerprint");
  const all = new Set([...inManual, ...(read("docs/RELEASE_NOTES.md").match(re) ?? []), ...(release.body.match(re) ?? [])]);
  if (all.size !== 1) fail(`the APK's signing fingerprint differs between docs/manual/android.md, docs/RELEASE_NOTES.md and the release: ${[...all].join(" / ")}`);
  return inManual[0];
}

// --- Pages ---

function gitRef() {
  const git = (...a) => execFileSync("git", a, { cwd: ROOT, stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
  try {
    return git("describe", "--tags", "--exact-match", "HEAD");
  } catch {
    try {
      return git("rev-parse", "HEAD");
    } catch {
      fail("this is not a git checkout with a commit; the pages link their source on GitHub at the commit they are built from");
    }
  }
}

/** Replaces each {{name}} in `text`, in one pass, so inserted values are not read again. */
function fill(text, values, where) {
  return text.replace(/\{\{([a-z_]+(?::[a-z0-9_.-]+)?)\}\}/gi, (_, key) => {
    if (!(key in values)) fail(`${where}: no value for {{${key}}}`);
    return values[key];
  });
}

/** Width and height of a PNG, for the img tag, so the page does not jump while it loads. */
function pngSize(file) {
  const b = fs.readFileSync(file);
  if (b.toString("latin1", 1, 4) !== "PNG") return "";
  return `width="${b.readUInt32BE(16)}" height="${b.readUInt32BE(20)}"`;
}

/** The anchor GitHub gives a heading, so links to #anchors work there and here. */
function slug(text) {
  return text.toLowerCase().trim().replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, "").replace(/ /g, "-");
}

/** The manual's pages in reading order, from the list in docs/manual/README.md. */
function manualPages() {
  const index = { title: "Manual", description: "", src: "docs/manual/README.md", out: "manual/index.html" };
  const pages = [index];
  for (const m of read(index.src).matchAll(/^- \[([^\]]+)\]\(([^)#]+)\)(?::\s*(.*))?$/gm)) {
    const src = path.posix.normalize(path.posix.join("docs/manual", m[2]));
    const base = src === "docs/RELEASE_NOTES.md" ? "release-notes" : path.posix.basename(src, ".md");
    if (!src.endsWith(".md") || !fs.existsSync(path.join(ROOT, src))) fail(`docs/manual/README.md lists ${m[2]}, which is not a Markdown file`);
    pages.push({ title: m[1], description: m[3] ?? "", src, out: `manual/${base}.html` });
  }
  return pages;
}

function main(release, ref, out) {
  const version = release.version;
  const template = read("site/template.html");
  const pages = manualPages();
  const pageBySrc = new Map(pages.map((p) => [p.src, p]));
  const copied = new Map();

  const relative = (fromOut, toOut) => path.posix.relative(path.posix.dirname(fromOut), toOut) || path.posix.basename(toOut);

  function copyImage(src, fromOut) {
    const name = path.posix.basename(src);
    if (copied.has(name) && copied.get(name) !== src) fail(`two images are called ${name}: ${copied.get(name)} and ${src}`);
    copied.set(name, src);
    fs.mkdirSync(path.join(out, "images"), { recursive: true });
    fs.copyFileSync(path.join(ROOT, src), path.join(out, "images", name));
    return relative(fromOut, `images/${name}`);
  }

  // Links in the Markdown point at files in the repository, so they work on
  // GitHub. Here a link to a manual page leads to its HTML page, and a link to
  // any other file to that file on GitHub, at the commit the site is built from.
  function resolveLink(href, env) {
    if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("#")) return href;
    const [file, hash] = href.split(/(?=#)/);
    const target = path.posix.normalize(path.posix.join(path.posix.dirname(env.page.src), decodeURI(file)));
    if (target.startsWith("../")) fail(`${env.page.src}: ${href} leads out of the repository`);
    const page = pageBySrc.get(target);
    if (page) return relative(env.page.out, page.out) + (hash ?? "");
    if (!fs.existsSync(path.join(ROOT, target))) fail(`${env.page.src}: ${href} leads nowhere`);
    const kind = fs.statSync(path.join(ROOT, target)).isDirectory() ? "tree" : "blob";
    return `${GITHUB}/${kind}/${ref}/${target}${hash ?? ""}`;
  }

  const md = createRequire(path.join(ROOT, "app/package.json"))("markdown-it")({ html: false, linkify: false, typographer: false });
  md.core.ruler.push("heading_ids", (state) => {
    const seen = new Map();
    state.tokens.forEach((t, i) => {
      if (t.type !== "heading_open") return;
      const text = state.tokens[i + 1].children.map((c) => (c.type === "text" || c.type === "code_inline" ? c.content : "")).join("");
      const base = slug(text);
      const n = seen.get(base) ?? 0;
      seen.set(base, n + 1);
      const id = n ? `${base}-${n}` : base;
      t.attrSet("id", id);
      state.env.headings.push({ level: Number(t.tag.slice(1)), text, id });
    });
  });
  const renderToken = (tokens, idx, opts, env, self) => self.renderToken(tokens, idx, opts);
  const linkOpen = md.renderer.rules.link_open ?? renderToken;
  md.renderer.rules.link_open = (tokens, idx, opts, env, self) => {
    tokens[idx].attrSet("href", resolveLink(tokens[idx].attrGet("href"), env));
    return linkOpen(tokens, idx, opts, env, self);
  };
  const image = md.renderer.rules.image;
  md.renderer.rules.image = (tokens, idx, opts, env, self) => {
    const t = tokens[idx];
    const src = path.posix.normalize(path.posix.join(path.posix.dirname(env.page.src), decodeURI(t.attrGet("src"))));
    if (!fs.existsSync(path.join(ROOT, src))) fail(`${env.page.src}: the image ${t.attrGet("src")} is missing`);
    t.attrSet("src", copyImage(src, env.page.out));
    const dims = Object.fromEntries([...pngSize(path.join(ROOT, src)).matchAll(/(\w+)="(\d+)"/g)].map((m) => [m[1], m[2]]));
    for (const [k, v] of Object.entries(dims)) t.attrSet(k, v);
    // A phone screenshot would fill the column at a great height: the style sheet narrows it.
    if (Number(dims.height) > Number(dims.width)) t.attrSet("class", "portrait");
    t.attrSet("loading", "lazy");
    return image(tokens, idx, opts, env, self);
  };

  const write = (file, html) => {
    fs.mkdirSync(path.dirname(path.join(out, file)), { recursive: true });
    fs.writeFileSync(path.join(out, file), html);
  };
  const layout = (file, { title, description, current, body }) =>
    write(
      file,
      fill(
        template,
        {
          title: esc(title),
          description: esc(description),
          root: file.includes("/") ? "../" : "",
          version: esc(version),
          current_download: current === "download" ? ' aria-current="page"' : "",
          current_manual: current === "manual" ? ' aria-current="page"' : "",
          body,
        },
        "site/template.html",
      ),
    );

  // Home
  const images = Object.fromEntries(
    fs.readdirSync(path.join(ROOT, "docs/images")).map((name) => [`size:${name}`, pngSize(path.join(ROOT, "docs/images", name))]),
  );
  for (const name of fs.readdirSync(path.join(ROOT, "docs/images"))) copyImage(`docs/images/${name}`, "index.html");
  layout("index.html", {
    title: "Cairn Notes",
    description: "Cairn is a free, open-source Markdown notes app for Linux, Windows and Android. A notebook is an ordinary folder of Markdown files, and sync between devices goes through your own server, end-to-end encrypted.",
    body: fill(read("site/home.html"), { version: esc(version), released: esc(release.released), ...images }, "site/home.html"),
  });

  // Download
  const a = release.assets;
  const files = (group) =>
    `<ul class="files">${KINDS.map((k) => a[k.key])
      .filter((x) => x.kind.group === group)
      .map(
        (x) =>
          `<li><a class="file-name" href="${esc(x.url)}">${esc(x.name)}</a>` +
          `<span class="file-meta">${size(x.size)} · ${esc(x.kind.label)}</span>` +
          `<span class="file-sum">SHA-256 <code>${x.sha}</code></span></li>`,
      )
      .join("")}</ul>`;
  const values = {
    version: esc(version),
    released: esc(release.released),
    release_url: esc(release.url),
    fingerprint: esc(signingFingerprint(release)),
    server_image: esc(`cairn-server:${version}`),
  };
  for (const k of KINDS) values[`asset:${k.key}`] = esc(a[k.key].name);
  for (const g of new Set(KINDS.map((k) => k.group))) values[`files:${g}`] = files(g);
  layout("download.html", {
    title: `Download Cairn ${version} · Cairn Notes`,
    description: `Cairn ${version} for Linux, Windows and Android, and the sync server's Docker image, with the size and SHA-256 of every file.`,
    current: "download",
    body: fill(read("site/download.html"), values, "site/download.html"),
  });

  // Manual
  const frame = read("site/manual.html");
  pages.forEach((page, i) => {
    const env = { page, headings: [] };
    let article = md.render(read(page.src), env);
    article = article
      .replace(/<pre><code/g, '<pre tabindex="0"><code')
      .replace(/\((FINDING-\d+(?:, FINDING-\d+)*)\)/g, '<span class="finding">($1)</span>');
    const h1 = env.headings.find((h) => h.level === 1);
    if (!h1 || env.headings[0] !== h1) fail(`${page.src} must start with a level 1 heading`);
    env.headings.reduce((prev, h) => {
      if (h.level > prev + 1) fail(`${page.src}: the heading "${h.text}" skips a level`);
      return h.level;
    }, 0);

    const toc =
      `<ul class="toc">` +
      pages
        .slice(1)
        .map((p) => {
          const here = p === page;
          const sub = here
            ? `<ul>${env.headings
                .filter((h) => h.level === 2)
                .map((h) => `<li><a href="#${esc(h.id)}">${esc(h.text)}</a></li>`)
                .join("")}</ul>`
            : "";
          return `<li><a href="${relative(page.out, p.out)}"${here ? ' aria-current="page"' : ""}>${esc(p.title)}</a>${here && sub !== "<ul></ul>" ? sub : ""}</li>`;
        })
        .join("") +
      `</ul>`;
    const prev = pages[i - 1];
    const next = pages[i + 1];
    const pager =
      (prev ? `<a class="prev" href="${relative(page.out, prev.out)}"><span>Previous</span>${esc(prev.title)}</a>` : "") +
      (next ? `<a class="next" href="${relative(page.out, next.out)}"><span>Next</span>${esc(next.title)}</a>` : "");
    const description = page.description ? page.description[0].toUpperCase() + page.description.slice(1) : "The manual of Cairn, a local-first Markdown notes app.";
    layout(page.out, {
      title: `${h1.text} · Cairn Notes`,
      description,
      current: "manual",
      body: fill(
        frame,
        { version: esc(version), current_index: i === 0 ? ' aria-current="page"' : "", toc, article, pager, source_url: esc(`${GITHUB}/blob/${ref}/${page.src}`), source_path: esc(page.src) },
        "site/manual.html",
      ),
    });
  });

  // Static files
  fs.copyFileSync(path.join(ROOT, "site/style.css"), path.join(out, "style.css"));
  fs.copyFileSync(path.join(ROOT, "app/src-tauri/icons/icon.svg"), path.join(out, "icon.svg"));
  fs.copyFileSync(path.join(ROOT, "app/src-tauri/icons/32x32.png"), path.join(out, "icon-32.png"));
  fs.copyFileSync(path.join(ROOT, "app/src-tauri/icons/128x128@2x.png"), path.join(out, "icon-256.png"));
  return pages.length + 2;
}

function size(bytes) {
  return bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : bytes >= 1e3 ? `${(bytes / 1e3).toFixed(1)} kB` : `${bytes} bytes`;
}

// --- Checks on the result ---

/** Every link and image between the pages must lead to a file, and every #anchor to an id there. */
function checkLinks(out) {
  const problems = [];
  const htmlFiles = fs.readdirSync(out, { recursive: true }).filter((f) => f.endsWith(".html")).map((f) => path.join(out, f));
  const ids = new Map(htmlFiles.map((f) => [f, new Set([...fs.readFileSync(f, "utf8").matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]))]));
  for (const file of htmlFiles) {
    const html = fs.readFileSync(file, "utf8");
    const name = path.relative(out, file);
    for (const m of html.matchAll(/<img\b[^>]*>/g)) {
      if (!/\salt="/.test(m[0])) problems.push(`${name}: an image has no alt text: ${m[0]}`);
      if (/&quot;/.test(m[0].replace(/\salt="[^"]*"/, ""))) problems.push(`${name}: an image has a broken attribute: ${m[0]}`);
    }
    if (!/<html lang="/.test(html)) problems.push(`${name}: no lang on <html>`);
    for (const m of html.matchAll(/\s(href|src|srcset)="([^"]*)"/g)) {
      const url = m[2].replace(/&amp;/g, "&").split(/\s/)[0];
      if (/^[a-z][a-z0-9+.-]*:/i.test(url)) continue;
      const [p, frag] = url.split("#");
      const target = p ? path.resolve(path.dirname(file), decodeURIComponent(p)) : file;
      if (!target.startsWith(out + path.sep) || !fs.existsSync(target)) problems.push(`${name}: ${url} leads nowhere`);
      else if (frag !== undefined && target.endsWith(".html") && !ids.get(target).has(decodeURIComponent(frag))) {
        problems.push(`${name}: ${url} has no #${frag} there`);
      }
    }
  }
  if (problems.length) fail(`broken links or missing alt text:\n  ${problems.join("\n  ")}`);
  return htmlFiles.length;
}

try {
  const out = path.resolve(options.out ?? path.join(ROOT, "target/site"));
  if (fs.existsSync(out)) {
    if (fs.readdirSync(out).length && !fs.existsSync(path.join(out, MARKER))) fail(`${out} is not empty and was not made by this script`);
    fs.rmSync(out, { recursive: true });
  }
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(path.join(out, MARKER), "");

  const release = await loadRelease();
  const ref = gitRef();
  const pages = main(release, ref, out);
  const checked = checkLinks(out);
  console.log(`Built ${pages} pages for Cairn ${release.version} (${release.tag}, released ${release.released}) from ${ref} into ${out}; ${checked} pages checked.`);
} catch (e) {
  if (!(e instanceof BuildError)) throw e;
  console.error(`site/build.mjs: ${e.message}`);
  process.exit(1);
}
