// sha256.ts against the FIPS 180-4 test vectors and Web Crypto.
//
// Run: cd app && npx vitest run src/lib/sha256.test.ts

import { describe, it, expect } from "vitest";
import { sha256Hex } from "./sha256";

const webCrypto = async (text: string) =>
  [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)))].map((b) => b.toString(16).padStart(2, "0")).join("");

describe("sha256Hex", () => {
  it("matches the standard test vectors", () => {
    expect(sha256Hex("")).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    expect(sha256Hex("abc")).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    expect(sha256Hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")).toBe(
      "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    );
    expect(sha256Hex("a".repeat(1_000_000))).toBe("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
  });

  it("matches Web Crypto around the block boundaries and for non-ASCII text", async () => {
    const texts = [55, 56, 57, 63, 64, 65, 119, 120, 128].map((n) => "x".repeat(n));
    texts.push("// @name Wörter zählen\n// @permissions editor\n", "日本語のノート 📝\r\n", "\u0000￿");
    for (const t of texts) expect(sha256Hex(t), JSON.stringify(t)).toBe(await webCrypto(t));
  });
});
