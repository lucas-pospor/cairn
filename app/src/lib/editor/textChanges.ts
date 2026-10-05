// The smallest changes that turn one text into another, so that text applied
// from disk (a reload, or another device's edit merged into unsaved typing)
// leaves the cursor and the undo history alone wherever the text is the same.

export interface TextChange {
  from: number;
  to: number;
  insert: string;
}

/** Line diffs with more changed lines than this are applied as one change. */
const MAX_DIFF = 200;

/**
 * Changes to `a` (positions in `a`, in order) that turn it into `b`: one per
 * run of changed lines, trimmed to the characters that differ. When the two
 * differ in too many places, one change from the first difference to the
 * last.
 */
export function textChanges(a: string, b: string): TextChange[] {
  const [p, q] = common(a, 0, a.length, b, 0, b.length);
  const endA = a.length - q;
  const endB = b.length - q;
  if (p === endA && p === endB) return [];
  const whole = [{ from: p, to: endA, insert: b.slice(p, endB) }];
  // Whole lines around the part that differs (the same in both texts, as
  // the text before and after it is).
  const s = p === 0 ? 0 : a.lastIndexOf("\n", p - 1) + 1;
  const nl = a.indexOf("\n", endA);
  const ea = nl < 0 ? a.length : nl + 1;
  const eb = endB + (ea - endA);
  const linesA = splitLines(a.slice(s, ea));
  const linesB = splitLines(b.slice(s, eb));
  if (linesA.length <= 1 && linesB.length <= 1) return whole;
  const runs = diffLines(linesA, linesB, MAX_DIFF);
  if (!runs) return whole;
  const at = (lines: string[]) => {
    const pos = [s];
    for (const l of lines) pos.push(pos[pos.length - 1] + l.length);
    return pos;
  };
  const posA = at(linesA);
  const posB = at(linesB);
  return runs.map(([ai, aj, bi, bj]) => {
    const [from, to, bf, bt] = [posA[ai], posA[aj], posB[bi], posB[bj]];
    const [rp, rq] = common(a, from, to, b, bf, bt);
    return { from: from + rp, to: to - rq, insert: b.slice(bf + rp, bt - rq) };
  });
}

/** Lengths of the common start and end of a[af, at) and b[bf, bt); a surrogate pair is never split. */
function common(a: string, af: number, at: number, b: string, bf: number, bt: number): [number, number] {
  const max = Math.min(at - af, bt - bf);
  let p = 0;
  while (p < max && a.charCodeAt(af + p) === b.charCodeAt(bf + p)) p++;
  let q = 0;
  while (q < max - p && a.charCodeAt(at - 1 - q) === b.charCodeAt(bt - 1 - q)) q++;
  if (p > 0 && /[\ud800-\udbff]/.test(a[af + p - 1])) p--;
  if (q > 0 && /[\udc00-\udfff]/.test(a[at - q])) q--;
  return [p, q];
}

/** Lines with their line breaks. */
function splitLines(text: string): string[] {
  return text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
}

/**
 * Myers' diff of two lists of lines: the runs of lines that differ, as
 * [aFrom, aTo, bFrom, bTo]. Null when more than `max` lines were added or
 * removed.
 */
function diffLines(a: string[], b: string[], max: number): [number, number, number, number][] | null {
  const n = a.length;
  const m = b.length;
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  const trace: Int32Array[] = [];
  let d = 0;
  search: for (; d <= max; d++) {
    trace.push(v.slice());
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) x++, y++;
      v[off + k] = x;
      if (x >= n && y >= m) break search;
    }
  }
  if (d > max) return null;
  // Walk back through the rounds, marking the lines removed and added.
  const removed = new Uint8Array(n);
  const added = new Uint8Array(m);
  let x = n;
  let y = m;
  for (; d > 0; d--) {
    const prev = trace[d];
    const k = x - y;
    const pk = k === -d || (k !== d && prev[off + k - 1] < prev[off + k + 1]) ? k + 1 : k - 1;
    const px = prev[off + pk];
    const py = px - pk;
    while (x > px && y > py) x--, y--;
    if (x === px) added[py] = 1;
    else removed[px] = 1;
    x = px;
    y = py;
  }
  // The lines left over pair up in order.
  const runs: [number, number, number, number][] = [];
  for (let i = 0, j = 0; i < n || j < m; ) {
    if (i < n && j < m && !removed[i] && !added[j]) {
      i++, j++;
      continue;
    }
    const [ai, bj] = [i, j];
    while (i < n && removed[i]) i++;
    while (j < m && added[j]) j++;
    if (i === ai && j === bj) return null; // cannot happen
    runs.push([ai, i, bj, j]);
  }
  return runs;
}
