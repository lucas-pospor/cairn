// Extra vaults for the performance tests (e2e/adv_perf.test.mjs).
//   node scripts/adv-perf-gen.mjs bignotes <dir>   -> long-3000.md, long-20000.md (+ a few small notes)
//   node scripts/adv-perf-gen.mjs flat <dir> [n=20000] -> Inbox/ with n notes, plus a few other folders
// Deterministic (fixed seed).

import fs from "node:fs";
import path from "node:path";

const [kind, dir, nArg] = process.argv.slice(2);
if (!kind || !dir) {
  console.error("usage: node scripts/adv-perf-gen.mjs bignotes|flat <dir> [n]");
  process.exit(1);
}

let seed = 7;
const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
const pick = (a) => a[Math.floor(rand() * a.length)];
const words = (
  "garden river stone cairn mountain path note idea project meeting draft review summary " +
  "coffee bread soup recipe travel train city map book chapter author quote memory sleep " +
  "health running music piano guitar code rust svelte editor index search graph link tag"
).split(" ");
const sentence = (n) => Array.from({ length: n }, () => pick(words)).join(" ");

/** A long note with the usual mix of Markdown that Live Preview decorates. */
function longNote(lines) {
  const out = ["---", "tags: [long, perf]", "status: draft", "---", "# Long note", ""];
  let i = 0;
  while (out.length < lines) {
    i++;
    const r = rand();
    if (i % 40 === 0) out.push(`## Section ${i / 40}`, "");
    else if (r < 0.25) out.push(`${sentence(8)} **${pick(words)}** and *${pick(words)}* with \`${pick(words)}\` ${sentence(6)}.`);
    else if (r < 0.4) out.push(`- ${sentence(5)} [[${pick(words)} ${pick(words)}]] #${pick(words)}`);
    else if (r < 0.5) out.push(`- [${rand() < 0.5 ? " " : "x"}] ${sentence(4)}`);
    else if (r < 0.58) out.push(`> ${sentence(10)}`);
    else if (r < 0.63) out.push(`1. ${sentence(6)} [link](https://example.com/${pick(words)})`);
    else if (r < 0.66) {
      out.push("| a | b | c |", "|---|---|---|", `| ${pick(words)} | ${pick(words)} | ${pick(words)} |`, "");
    } else if (r < 0.68) out.push("```js", `const ${pick(words)} = "${pick(words)}";`, "```");
    else if (r < 0.72) out.push("");
    else out.push(`${sentence(12)} ~~${pick(words)}~~ ${sentence(4)}.`);
  }
  return out.slice(0, lines).join("\n") + "\n";
}

fs.mkdirSync(dir, { recursive: true });
if (kind === "bignotes") {
  fs.writeFileSync(path.join(dir, "long-3000.md"), longNote(3000));
  fs.writeFileSync(path.join(dir, "long-20000.md"), longNote(20000));
  for (const w of words.slice(0, 10)) fs.writeFileSync(path.join(dir, `${w} ${w}.md`), `# ${w}\n\n${sentence(30)}\n`);
  console.log(`wrote long-3000.md, long-20000.md to ${dir}`);
} else if (kind === "flat") {
  const n = Number(nArg ?? 20000);
  fs.mkdirSync(path.join(dir, "Inbox"), { recursive: true });
  for (let i = 0; i < n; i++) {
    const name = `${pick(words)} ${pick(words)} ${String(i).padStart(5, "0")}.md`;
    fs.writeFileSync(path.join(dir, "Inbox", name), `# ${name}\n\n${sentence(20)} [[${pick(words)}]]\n`);
  }
  for (const f of ["Areas", "Projects", "Zettel"]) {
    fs.mkdirSync(path.join(dir, f), { recursive: true });
    fs.writeFileSync(path.join(dir, f, `${f} index.md`), `# ${f}\n`);
  }
  fs.writeFileSync(path.join(dir, "Start here.md"), "# Start here\n");
  console.log(`wrote ${n} notes in Inbox/ to ${dir}`);
} else {
  console.error("unknown kind " + kind);
  process.exit(1);
}
