// Date formats for core plugins (app/src/lib/corePlugins/dates.ts).
//
// Run: cd app && npx vitest run src/lib/corePlugins/dates.test.ts

import { describe, expect, it } from "vitest";
import { formatDate, ordinal, weekOf } from "./dates";

/** A local date and time. */
const at = (y: number, m: number, d: number, h = 0, mi = 0, s = 0, ms = 0) => new Date(y, m - 1, d, h, mi, s, ms);
// Monday 5 October 2026, 14:07:09.045.
const MON = at(2026, 10, 5, 14, 7, 9, 45);

describe("formatDate", () => {
  it("writes the formats daily notes use", () => {
    expect(formatDate(MON, "YYYY-MM-DD")).toBe("2026-10-05");
    expect(formatDate(MON, "YYYYMMDDHHmm")).toBe("202610051407");
    expect(formatDate(MON, "YYYYMMDDHHmmss")).toBe("20261005140709");
    expect(formatDate(MON, "dddd, MMMM Do YYYY")).toBe("Monday, October 5th 2026");
    expect(formatDate(MON, "YYYY/MM/YYYY-MM-DD")).toBe("2026/10/2026-10-05");
    expect(formatDate(MON, "DD.MM.YY")).toBe("05.10.26");
  });

  it("writes every date part", () => {
    const cases: [string, string][] = [
      ["YYYY", "2026"], ["YY", "26"], ["Q", "4"], ["Qo", "4th"],
      ["M", "10"], ["Mo", "10th"], ["MM", "10"], ["MMM", "Oct"], ["MMMM", "October"],
      ["D", "5"], ["Do", "5th"], ["DD", "05"], ["DDD", "278"], ["DDDo", "278th"], ["DDDD", "278"],
      ["d", "1"], ["do", "1st"], ["dd", "Mo"], ["ddd", "Mon"], ["dddd", "Monday"], ["e", "1"], ["E", "1"],
      ["H", "14"], ["HH", "14"], ["h", "2"], ["hh", "02"], ["k", "14"], ["kk", "14"],
      ["m", "7"], ["mm", "07"], ["s", "9"], ["ss", "09"], ["S", "0"], ["SS", "04"], ["SSS", "045"],
      ["A", "PM"], ["a", "pm"],
    ];
    for (const [f, want] of cases) expect(formatDate(MON, f), f).toBe(want);
    expect(formatDate(at(2026, 2, 3), "M/D DDDD")).toBe("2/3 034");
  });

  it("writes midnight and noon on the 12-hour and 24-hour clocks", () => {
    const midnight = at(2026, 1, 1, 0, 5);
    expect(formatDate(midnight, "h:mm A, HH:mm, k")).toBe("12:05 AM, 00:05, 24");
    expect(formatDate(at(2026, 1, 1, 12), "h A a")).toBe("12 PM pm");
    expect(formatDate(at(2026, 1, 1, 23, 59), "hh:mm a")).toBe("11:59 pm");
  });

  it("numbers Sunday 0, or 7 as an ISO weekday", () => {
    const sun = at(2026, 10, 4);
    expect(formatDate(sun, "d E ddd dddd")).toBe("0 7 Sun Sunday");
  });

  it("gives English day and month names", () => {
    expect([...Array(12)].map((_, i) => formatDate(at(2026, i + 1, 1), "MMMM")).join(" ")).toBe(
      "January February March April May June July August September October November December",
    );
    expect([...Array(7)].map((_, i) => formatDate(at(2026, 10, 4 + i), "dddd")).join(" ")).toBe(
      "Sunday Monday Tuesday Wednesday Thursday Friday Saturday",
    );
  });

  it("keeps text in brackets, escaped letters and other characters as they are", () => {
    expect(formatDate(MON, "[Today is] dddd")).toBe("Today is Monday");
    expect(formatDate(MON, "[YYYY] YYYY")).toBe("YYYY 2026");
    expect(formatDate(MON, "YYYY-MM-DDTHH:mm")).toBe("2026-10-05T14:07");
    expect(formatDate(MON, "\\Y YYYY")).toBe("Y 2026");
    expect(formatDate(MON, "[Journal]: ")).toBe("Journal: ");
    // As in moment.js, a letter that is a format is one outside brackets too.
    expect(formatDate(MON, "Journal")).toBe("Journpml");
    expect(formatDate(MON, "")).toBe("");
    expect(formatDate(MON, "日記 YYYY年M月D日")).toBe("日記 2026年10月5日");
  });

  it("writes week numbers, ISO and US, with the year they belong to", () => {
    expect(formatDate(MON, "GGGG-[W]WW gggg-[W]ww W w Wo wo")).toBe("2026-W41 2026-W41 41 41 41st 41st");
    // [date, ISO week, ISO year, US week, US year]
    const cases: [Date, number, number, number, number][] = [
      [at(2026, 1, 1), 1, 2026, 1, 2026], // Thursday
      [at(2025, 12, 28), 52, 2025, 1, 2026], // Sunday: starts US week 1 of 2026
      [at(2026, 12, 31), 53, 2026, 1, 2027], // 2026 has 53 ISO weeks
      [at(2027, 1, 1), 53, 2026, 1, 2027],
      [at(2027, 1, 3), 53, 2026, 2, 2027],
      [at(2024, 12, 29), 52, 2024, 1, 2025],
      [at(2024, 12, 30), 1, 2025, 1, 2025],
      [at(2020, 12, 31), 53, 2020, 1, 2021],
      [at(2021, 1, 3), 53, 2020, 2, 2021],
    ];
    for (const [d, isoWeek, isoYear, usWeek, usYear] of cases) {
      const label = d.toDateString();
      expect(weekOf(d, 1, 4), label).toEqual({ week: isoWeek, year: isoYear });
      expect(weekOf(d, 0, 1), label).toEqual({ week: usWeek, year: usYear });
      expect(formatDate(d, "W GGGG GG w gggg gg"), label).toBe(
        `${isoWeek} ${isoYear} ${String(isoYear % 100).padStart(2, "0")} ${usWeek} ${usYear} ${String(usYear % 100).padStart(2, "0")}`,
      );
    }
  });

  it("writes Unix time and the time zone offset", () => {
    expect(formatDate(MON, "X")).toBe(String(Math.floor(MON.getTime() / 1000)));
    expect(formatDate(MON, "x")).toBe(String(MON.getTime()));
    const off = -MON.getTimezoneOffset();
    const hh = String(Math.floor(Math.abs(off) / 60)).padStart(2, "0");
    const mm = String(Math.abs(off) % 60).padStart(2, "0");
    expect(formatDate(MON, "Z")).toBe(`${off < 0 ? "-" : "+"}${hh}:${mm}`);
    expect(formatDate(MON, "ZZ")).toBe(`${off < 0 ? "-" : "+"}${hh}${mm}`);
  });
});

describe("ordinal", () => {
  it("adds English ordinal endings", () => {
    expect([1, 2, 3, 4, 10, 11, 12, 13, 14, 21, 22, 23, 24, 101, 111, 112, 113, 121].map(ordinal).join(" ")).toBe(
      "1st 2nd 3rd 4th 10th 11th 12th 13th 14th 21st 22nd 23rd 24th 101st 111th 112th 113th 121st",
    );
  });
});
