// Dates written with the format letters of moment.js, which Obsidian uses too, so
// a vault's daily note names and template formats keep working: "YYYY-MM-DD",
// "dddd, MMMM Do YYYY", "gggg-[W]ww". Text in [brackets] stays as it is, and so
// does any letter that is not one of the formats below.
//
// Day and month names are English on every device, whatever its language, so two
// devices name the same day's note the same way.

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

const pad = (n: number, width: number) => String(Math.abs(n)).padStart(width, "0").replace(/^/, n < 0 ? "-" : "");

/** 1st, 2nd, 3rd, 4th, 11th, 21st... */
export function ordinal(n: number): string {
  const tens = n % 100;
  const suffix = tens >= 11 && tens <= 13 ? "th" : (["th", "st", "nd", "rd"][n % 10] ?? "th");
  return `${n}${suffix}`;
}

/** Days since 1970-01-01 of a calendar date. */
const dayNumber = (y: number, m: number, d: number) => Math.round(Date.UTC(y, m, d) / 86400000);
/** Day of the week of a day number (0 = Sunday). 1970-01-01 was a Thursday. */
const weekday = (n: number) => (((n + 4) % 7) + 7) % 7;

/**
 * The week of the year of a date and the year that week belongs to. Weeks start on
 * `start` (0 Sunday, 1 Monday), and week 1 is the one with January `anchor` in it:
 * ISO weeks are (1, 4), the English (US) weeks of moment.js are (0, 1).
 */
export function weekOf(date: Date, start: number, anchor: number): { week: number; year: number } {
  const day = dayNumber(date.getFullYear(), date.getMonth(), date.getDate());
  const firstWeek = (y: number) => {
    const a = dayNumber(y, 0, anchor);
    return a - ((weekday(a) - start + 7) % 7);
  };
  let year = date.getFullYear();
  if (day < firstWeek(year)) year--;
  else if (day >= firstWeek(year + 1)) year++;
  return { year, week: Math.floor((day - firstWeek(year)) / 7) + 1 };
}

const TOKEN =
  /\[([^\]]*)\]|\\(.)|YYYY|YY|Qo|Q|MMMM|MMM|MM|Mo|M|DDDD|DDDo|DDD|DD|Do|D|dddd|ddd|dd|do|d|E|e|GGGG|GG|gggg|gg|WW|Wo|W|ww|wo|w|HH|H|hh|h|kk|k|mm|m|ss|s|SSS|SS|S|A|a|X|x|ZZ|Z/g;

/** `date` (local time) written in `format`. */
export function formatDate(date: Date, format: string): string {
  const y = date.getFullYear();
  const mo = date.getMonth();
  const d = date.getDate();
  const wd = date.getDay();
  const h = date.getHours();
  const mi = date.getMinutes();
  const se = date.getSeconds();
  const ms = date.getMilliseconds();
  const yearDay = dayNumber(y, mo, d) - dayNumber(y, 0, 1) + 1;
  const iso = () => weekOf(date, 1, 4);
  const us = () => weekOf(date, 0, 1);
  const h12 = h % 12 || 12;
  const offset = () => {
    const m = -date.getTimezoneOffset();
    return [m < 0 ? "-" : "+", pad(Math.floor(Math.abs(m) / 60), 2), pad(Math.abs(m) % 60, 2)];
  };
  return format.replace(TOKEN, (token, literal: string | undefined, escaped: string | undefined) => {
    if (literal !== undefined) return literal;
    if (escaped !== undefined) return escaped;
    switch (token) {
      case "YYYY": return pad(y, 4);
      case "YY": return pad(y % 100, 2);
      case "Q": return String(Math.floor(mo / 3) + 1);
      case "Qo": return ordinal(Math.floor(mo / 3) + 1);
      case "M": return String(mo + 1);
      case "Mo": return ordinal(mo + 1);
      case "MM": return pad(mo + 1, 2);
      case "MMM": return MONTHS[mo].slice(0, 3);
      case "MMMM": return MONTHS[mo];
      case "D": return String(d);
      case "Do": return ordinal(d);
      case "DD": return pad(d, 2);
      case "DDD": return String(yearDay);
      case "DDDo": return ordinal(yearDay);
      case "DDDD": return pad(yearDay, 3);
      case "d": case "e": return String(wd);
      case "do": return ordinal(wd);
      case "dd": return DAYS[wd].slice(0, 2);
      case "ddd": return DAYS[wd].slice(0, 3);
      case "dddd": return DAYS[wd];
      case "E": return String(wd || 7);
      case "W": return String(iso().week);
      case "Wo": return ordinal(iso().week);
      case "WW": return pad(iso().week, 2);
      case "GG": return pad(iso().year % 100, 2);
      case "GGGG": return pad(iso().year, 4);
      case "w": return String(us().week);
      case "wo": return ordinal(us().week);
      case "ww": return pad(us().week, 2);
      case "gg": return pad(us().year % 100, 2);
      case "gggg": return pad(us().year, 4);
      case "H": return String(h);
      case "HH": return pad(h, 2);
      case "h": return String(h12);
      case "hh": return pad(h12, 2);
      case "k": return String(h || 24);
      case "kk": return pad(h || 24, 2);
      case "m": return String(mi);
      case "mm": return pad(mi, 2);
      case "s": return String(se);
      case "ss": return pad(se, 2);
      case "S": return String(Math.floor(ms / 100));
      case "SS": return pad(Math.floor(ms / 10), 2);
      case "SSS": return pad(ms, 3);
      case "A": return h < 12 ? "AM" : "PM";
      case "a": return h < 12 ? "am" : "pm";
      case "X": return String(Math.floor(date.getTime() / 1000));
      case "x": return String(date.getTime());
      case "Z": { const [sign, hh, mm] = offset(); return `${sign}${hh}:${mm}`; }
      case "ZZ": return offset().join("");
    }
    return token;
  });
}
