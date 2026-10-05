// Generate a synthetic vault for performance testing.
//   node scripts/gen-vault.mjs <dir> [notes=10000]
// Deterministic: the same arguments always give the same vault.

import fs from "node:fs";
import path from "node:path";

const [dir, countArg] = process.argv.slice(2);
if (!dir) {
  console.error("usage: node scripts/gen-vault.mjs <dir> [notes]");
  process.exit(1);
}
const N = Number(countArg ?? 10000);

let seed = 42;
const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
const pick = (a) => a[Math.floor(rand() * a.length)];

const words = (
  "garden river stone cairn mountain path note idea project meeting draft review summary " +
  "coffee bread soup recipe travel train city map book chapter author quote memory sleep " +
  "health running music piano guitar code rust svelte editor index search graph link tag " +
  "budget invoice tax plan goal habit week month year spring summer autumn winter light " +
  "dark ocean forest bird tree leaf root branch seed harvest market friend family letter"
).split(" ");
const folders = ["Projects", "Areas", "Resources", "Archive", "Journal", "People", "Reading", "Work/Clients", "Work/Internal", "Ideas"];

const titles = [];
for (let i = 0; i < N; i++) {
  const t = `${pick(words)} ${pick(words)} ${i}`;
  titles.push({ title: t[0].toUpperCase() + t.slice(1), folder: rand() < 0.15 ? "" : pick(folders) });
}

fs.mkdirSync(dir, { recursive: true });
let bytes = 0;
for (let i = 0; i < N; i++) {
  const { title, folder } = titles[i];
  const lines = [];
  if (rand() < 0.4) lines.push("---", `tags: [${pick(words)}, ${pick(words)}]`, `status: ${pick(["draft", "done", "active"])}`, "---");
  lines.push(`# ${title}`, "");
  const paras = 2 + Math.floor(rand() * 6);
  for (let p = 0; p < paras; p++) {
    const sentence = [];
    const len = 20 + Math.floor(rand() * 60);
    for (let w = 0; w < len; w++) {
      const r = rand();
      if (r < 0.02) sentence.push(`[[${titles[Math.floor(rand() * N)].title}]]`);
      else if (r < 0.025) sentence.push(`#${pick(words)}`);
      else sentence.push(pick(words));
    }
    lines.push(sentence.join(" ") + ".", "");
    if (rand() < 0.2) lines.push(`- [ ] ${pick(words)} ${pick(words)}`, `- [x] ${pick(words)}`, "");
  }
  const content = lines.join("\n");
  const p = path.join(dir, folder, `${title}.md`);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
  bytes += content.length;
}
console.log(`wrote ${N} notes, ${(bytes / 1e6).toFixed(1)} MB, to ${dir}`);
