// Shapes returned by the Rust side (see crates/cairn-core).

export type EntryKind = "file" | "dir";

export interface FileStat {
  path: string;
  kind: EntryKind;
  size: number;
  mtime: number;
}

export type Change =
  | { type: "created"; entry: FileStat }
  | { type: "modified"; entry: FileStat }
  | { type: "deleted"; path: string; kind: EntryKind }
  | { type: "renamed"; from: string; entry: FileStat };

export interface VaultInfo {
  root: string;
  name: string;
  entries: FileStat[];
}

export interface NoteContent {
  content: string;
  hash: string;
}

export interface WriteResult {
  entry: FileStat;
  hash: string;
  changes: Change[];
}

export interface Segment {
  text: string;
  hit: boolean;
}

export interface Snippet {
  line: number;
  segments: Segment[];
}

export interface SearchHit {
  path: string;
  score: number;
  snippets: Snippet[];
}

export interface BacklinkItem {
  line: number;
  context: string;
}

export interface Backlinks {
  source: string;
  items: BacklinkItem[];
}

/** A [[wikilink]] or a [Markdown](link); they resolve differently. */
export type LinkKind = "wiki" | "markdown";

export interface OutgoingLink {
  target: string;
  resolved: string | null;
  line: number;
  embed: boolean;
  kind: LinkKind;
}

export interface CoreError {
  kind:
    | "notFound"
    | "alreadyExists"
    | "invalidPath"
    | "invalidName"
    | "conflict"
    | "notANote"
    | "moveIntoSelf"
    | "io";
  detail: string;
}

export function isCoreError(e: unknown): e is CoreError {
  return typeof e === "object" && e !== null && "kind" in e && "detail" in e;
}

export function errorMessage(e: unknown): string {
  if (isCoreError(e)) {
    switch (e.kind) {
      case "notFound":
        return `Not found: ${e.detail}`;
      case "alreadyExists":
        return `Something named "${e.detail}" already exists.`;
      case "invalidName":
        return `"${e.detail}" is not a valid name. Avoid / \\ : * ? " < > | [ ] # ^, leading dots and names Windows keeps for devices, such as CON or NUL.`;
      case "invalidPath":
        return `Invalid path: ${e.detail}`;
      case "conflict":
        return `${e.detail} changed on disk.`;
      case "notANote":
        return `${e.detail} is not a Markdown note.`;
      case "moveIntoSelf":
        return `Cannot move a folder into itself.`;
      case "io":
        return e.detail;
    }
  }
  return String(e);
}

export interface TagCount {
  tag: string;
  count: number;
}

export interface GraphNode {
  id: string;
  kind: "note" | "unresolved";
  degree: number;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: [number, number][];
}

export interface Heading {
  level: number;
  text: string;
  line: number;
}

export interface NoteInfo {
  frontmatter: Record<string, unknown> | null;
  /** The note has a `---` block that is not valid YAML. */
  frontmatterInvalid: boolean;
  tags: string[];
  headings: Heading[];
}

export interface SyncStatus {
  /** The vault this is for. */
  root: string;
  configured: boolean;
  state: "off" | "idle" | "syncing" | "error";
  server: string | null;
  vaultId: string | null;
  device: string | null;
  lastSync: number | null;
  lastError: string | null;
  lastPulled: number;
  lastPushed: number;
  conflicts: string[];
  /** Files the last successful sync left out, and why. */
  skipped: { path: string; reason: string }[];
}

export interface HistoryEntry {
  seq: number;
  created: number;
  device: string;
  deleted: boolean;
  size: number;
}
