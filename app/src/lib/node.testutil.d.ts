// Types for the Node built-ins that tests use: node:buffer and node:vm, which
// adv_verify_pl_09.test.ts uses to run the plugin host's real worker bootstrap
// under Vitest, and node:fs, which themeContrast.test.ts uses to read app.css
// (Vitest turns a CSS import into an empty string, even with ?raw).
//
// The app runs in a webview, so its tsconfig leaves out @types/node on purpose:
// a Node API used by mistake in app code should fail the type check. Only the
// signatures the tests call are declared here; other node: modules and
// globals such as `process` still fail the check. App code must not import
// these either: they do not exist in the webview.

declare module "node:buffer" {
  /** The Blob registered under a `blob:nodedata:...` URL by URL.createObjectURL. */
  export function resolveObjectURL(id: string): Blob | undefined;
}

declare module "node:vm" {
  /** Turns `sandbox` into the global object of a new context and returns it. */
  export function createContext(sandbox: object): object;
  /** Runs `code` as a script with the context's global object as its global. */
  export function runInContext(code: string, context: object): unknown;
}

declare module "node:fs" {
  /** The text of a file. */
  export function readFileSync(path: URL, encoding: "utf8"): string;
}
