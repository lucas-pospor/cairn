// Turn the flat entry list from the backend into a sorted folder tree.

import type { FileStat } from "./types";
import { fileName, parent } from "./paths";

export interface TreeNode {
  path: string;
  name: string;
  kind: "file" | "dir";
  children: TreeNode[];
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function compareNodes(a: TreeNode, b: TreeNode): number {
  if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
  return collator.compare(a.name, b.name) || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
}

export function buildTree(entries: FileStat[]): TreeNode {
  const root: TreeNode = { path: "", name: "", kind: "dir", children: [] };
  const dirs = new Map<string, TreeNode>([["", root]]);
  const sorted = [...entries].sort((a, b) => a.path.length - b.path.length);
  const getDir = (p: string): TreeNode => {
    let d = dirs.get(p);
    if (!d) {
      // Parent missing from the list (should not happen): synthesize it.
      d = { path: p, name: fileName(p), kind: "dir", children: [] };
      dirs.set(p, d);
      getDir(parent(p)).children.push(d);
    }
    return d;
  };
  for (const e of sorted) {
    if (e.kind === "dir") {
      if (dirs.has(e.path)) continue;
      const node: TreeNode = { path: e.path, name: fileName(e.path), kind: "dir", children: [] };
      dirs.set(e.path, node);
      getDir(parent(e.path)).children.push(node);
    } else {
      getDir(parent(e.path)).children.push({ path: e.path, name: fileName(e.path), kind: "file", children: [] });
    }
  }
  const sortRec = (n: TreeNode) => {
    n.children.sort(compareNodes);
    n.children.forEach(sortRec);
  };
  sortRec(root);
  return root;
}

/** A visible row; `pos` (1-based) and `size` place it among its siblings. */
export interface TreeRow {
  node: TreeNode;
  depth: number;
  pos: number;
  size: number;
}

/** Visible rows in display order, given the set of expanded folders. */
export function flatten(root: TreeNode, expanded: Set<string>): TreeRow[] {
  const out: TreeRow[] = [];
  const walk = (n: TreeNode, depth: number) => {
    n.children.forEach((c, i) => {
      out.push({ node: c, depth, pos: i + 1, size: n.children.length });
      if (c.kind === "dir" && expanded.has(c.path)) walk(c, depth + 1);
    });
  };
  walk(root, 0);
  return out;
}
