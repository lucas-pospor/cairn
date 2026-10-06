// What saveAttachment sends to the backend. The desktop sends the bytes raw.
// Tauri cannot send raw bytes to a command on Android (it turns them into a
// JSON array of numbers, which save_attachment refuses), so there they go as
// base64. The folder and the name travel in headers on both.
//
// Run: cd app && npx vitest run src/lib/saveAttachment.test.ts

import { beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({ android: false }));
const calls = vi.hoisted(() => [] as { cmd: string; args: unknown; options: unknown }[]);

vi.mock("./platform", () => ({
  get isAndroid() {
    return platform.android;
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: unknown, options: unknown) => {
    calls.push({ cmd, args, options });
    return "attachments/x";
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));

const { backend } = await import("./backend");

const headers = { headers: { "x-dir": "attachments", "x-name": "R%C3%A9sum%C3%A9%20%231.pdf" } };
/** `n` bytes holding every byte value. */
const bytes = (n: number) => Uint8Array.from({ length: n }, (_, i) => (i * 31 + (i >> 10)) & 255);
const decode = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

beforeEach(() => {
  calls.length = 0;
});

describe("saveAttachment", () => {
  it("sends the raw bytes on the desktop", async () => {
    platform.android = false;
    const b = bytes(1000);
    expect(await backend.saveAttachment("attachments", "Résumé #1.pdf", b)).toBe("attachments/x");
    expect(calls).toEqual([{ cmd: "save_attachment", args: b, options: headers }]);
    expect(calls[0].args).toBe(b);
  });

  it("sends base64 on Android, the same bytes at any size", async () => {
    platform.android = true;
    // Empty, small, and across the chunk boundaries of the encoder.
    for (const n of [0, 1, 2, 3, 256, 0x8000 - 1, 0x8000, 0x8000 + 1, 3 * 0x8000 + 7, 300 * 1024]) {
      calls.length = 0;
      const b = bytes(n);
      await backend.saveAttachment("attachments", "Résumé #1.pdf", b);
      expect(calls.length).toBe(1);
      const { cmd, args, options } = calls[0];
      expect(cmd).toBe("save_attachment");
      expect(options).toEqual(headers);
      expect(decode((args as { data: string }).data)).toEqual(b);
    }
    calls.length = 0;
    await backend.saveAttachment("", "a.bin", Uint8Array.of(0, 1, 2, 253, 254, 255));
    expect(calls[0].args).toEqual({ data: "AAEC/f7/" });
  });
});
