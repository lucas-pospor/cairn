// Word and character counts for the status bar (app/src/lib/wordCount.ts).
//
// Run: cd app && npx vitest run src/lib/wordCount.test.ts

import { describe, expect, it } from "vitest";
import { countText } from "./wordCount";

const words = (t: string) => countText(t).words;
const chars = (t: string) => countText(t).characters;

describe("words", () => {
  it("counts Latin text by its words, with apostrophes and hyphens inside them", () => {
    expect(words("Hello, world! It's a well-known fact.")).toBe(6);
    expect(words("don’t stop - ever")).toBe(3);
    expect(words("Call 555 1234 at 9:30")).toBe(6);
    expect(words("# Heading\n\n- [ ] task one\n- **bold** and _italic_")).toBe(6);
    expect(words("")).toBe(0);
    expect(words("   \n\t ... --- !!!")).toBe(0);
  });

  it("counts every Han, Hiragana and Katakana character as a word", () => {
    expect(words("我喜欢猫。")).toBe(4);
    expect(words("東京タワーに行きました")).toBe(11);
    // Latin next to Japanese: each is counted its own way.
    expect(words("Windows版をダウンロード")).toBe(9);
    expect(words("ｶﾀｶﾅ ｰ")).toBe(5);
    // Punctuation is not a word.
    expect(words("「こんにちは」、・。")).toBe(5);
  });

  it("counts Korean by its spaces", () => {
    expect(words("안녕하세요 세계")).toBe(2);
    // The same written as separate jamo (NFD).
    expect(words("안녕하세요 세계".normalize("NFD"))).toBe(2);
  });

  it("finds the words of Thai and Lao, which are written without spaces", () => {
    expect(words("สวัสดีครับ")).toBe(2);
    expect(words("ฉันรักแมว")).toBe(3);
    expect(words("ພາສາລາວ")).toBe(2);
    expect(words("I said สวัสดีครับ twice")).toBe(5);
  });

  it("counts a Thai run as one word where there is no Intl.Segmenter", () => {
    expect(countText("ฉันรักแมว and more", null).words).toBe(3);
  });

  it("keeps letters with combining marks in one word", () => {
    expect(words("étude café")).toBe(2);
    expect(words("नमस्ते दुनिया")).toBe(2);
    expect(words("Привет, мир")).toBe(2);
    expect(words("Γειά σου κόσμε")).toBe(3);
  });

  it("does not count emoji as words", () => {
    expect(words("I ❤️ cats 🐈‍⬛ 👍🏽")).toBe(2);
  });
});

describe("characters", () => {
  it("counts what a reader sees as one character, spaces included and line breaks not", () => {
    expect(chars("Hello, world!")).toBe(13);
    expect(chars("a\nb\r\nc")).toBe(3);
    expect(chars("")).toBe(0);
    expect(chars("我喜欢猫。")).toBe(5);
    expect(chars("안녕하세요 세계")).toBe(8);
    expect(chars("안녕하세요 세계".normalize("NFD"))).toBe(8);
  });

  it("counts an emoji with its skin tone, or joined emoji, as one character", () => {
    expect(chars("I ❤️ cats 🐈‍⬛ 👍🏽")).toBe(12);
    expect(chars("👨‍👩‍👧")).toBe(1);
    expect(chars("🇫🇷🇯🇵")).toBe(2);
  });

  it("counts a letter with combining marks as one character", () => {
    expect(chars("étude café")).toBe(10);
    expect(chars("étude café")).toBe(10);
  });

  it("counts code points where there is no Intl.Segmenter", () => {
    expect(countText("é", null).characters).toBe(2);
    expect(countText("👍🏽", null).characters).toBe(2);
    expect(countText("plain", null).characters).toBe(5);
  });
});
