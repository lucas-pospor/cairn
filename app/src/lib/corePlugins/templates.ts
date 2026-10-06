// Templates: insert a note from the template folder at the cursor, with
// {{title}}, {{date}} and {{time}} filled in. {{date:FORMAT}} and
// {{time:FORMAT}} take a format of their own (see dates.ts). The template goes
// in as it is (frontmatter too), as one edit that one undo takes back, and
// autosave writes it like typing.

import { displayName } from "../paths";
import { errorMessage } from "../types";
import { option, type CoreHost, type CorePlugin } from "./core";
import { formatDate } from "./dates";
import { cleanFolder, notesIn } from "./files";

export const DEFAULT_DATE = "YYYY-MM-DD";
export const DEFAULT_TIME = "HH:mm";

export interface TemplateValues {
  /** The name of the note the template goes into. */
  title: string;
  now: Date;
  dateFormat: string;
  timeFormat: string;
}

/** `text` with {{title}}, {{date}}, {{time}}, {{date:FORMAT}} and {{time:FORMAT}} filled in; anything else in braces stays. */
export function fillTemplate(text: string, v: TemplateValues): string {
  return text.replace(/\{\{\s*(title|date|time)\s*(?::([^}]*))?\}\}/gi, (all, name: string, format: string | undefined) => {
    switch (name.toLowerCase()) {
      case "title":
        return format === undefined ? v.title : all;
      case "date":
        return formatDate(v.now, format?.trim() || v.dateFormat || DEFAULT_DATE);
      default:
        return formatDate(v.now, format?.trim() || v.timeFormat || DEFAULT_TIME);
    }
  });
}

/** The values for a template going into the note at `path`, with the formats set in Settings. */
export function templateValues(host: CoreHost, path: string): TemplateValues {
  return {
    title: displayName(path),
    now: host.now(),
    dateFormat: option(templates, "dateFormat").trim(),
    timeFormat: option(templates, "timeFormat").trim(),
  };
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

async function insertTemplate(host: CoreHost) {
  const target = host.activeNote();
  if (!target || !host.canInsert()) return host.toast("Open a note to insert a template.");
  const folder = cleanFolder(option(templates, "folder"));
  if (!folder) return host.toast("Choose a template folder in Settings > Core plugins.");
  const list = notesIn(folder, host.files());
  if (!list.length) {
    return host.toast(
      host.folders().includes(folder)
        ? `There are no notes in ${folder}. Add one to use it as a template.`
        : `There is no folder named ${folder}. Create it and add notes to use them as templates.`,
    );
  }
  const chosen = await host.choose(
    "Insert template",
    // By path in the folder, so two templates with the same name in subfolders are told apart.
    list.map((p) => ({ value: p, label: p.slice(folder.length + 1).replace(/\.(md|markdown)$/i, "") })),
  );
  if (!chosen) return;
  let text: string;
  try {
    text = await host.readNote(chosen);
  } catch (e) {
    return host.toast(`Could not read ${displayName(chosen)}: ${errorMessage(e)}`, "error");
  }
  if (!host.insert(target, fillTemplate(text, templateValues(host, target)))) {
    host.toast(`${displayName(target)} is no longer open for editing. The template was not inserted.`);
  }
}

export const templates: CorePlugin = {
  id: "templates",
  name: "Templates",
  description: "Insert a note from your template folder at the cursor, with the title, date and time filled in.",
  defaultOn: true,
  defaults: { folder: "Templates", dateFormat: DEFAULT_DATE, timeFormat: DEFAULT_TIME },
  options: [
    {
      key: "folder",
      label: "Template folder",
      description: "Every note in this folder and its subfolders is a template.",
      placeholder: "Templates",
      check(value, host) {
        const folder = cleanFolder(value);
        if (!folder) return { problem: "Choose a folder to use templates." };
        if (folder.split("/").some((s) => s.startsWith("."))) return { problem: "Folders whose names start with a dot are hidden. Choose another folder." };
        if (!host.folders().includes(folder)) return { example: `There is no folder named ${folder} yet.` };
        return { example: plural(notesIn(folder, host.files()).length, "template", "templates") };
      },
    },
    {
      key: "dateFormat",
      label: "Date format",
      description: "For {{date}}. YYYY is the year, MM the month, DD the day.",
      placeholder: DEFAULT_DATE,
      check: (value, host) => ({ example: `Today: ${formatDate(host.now(), value || DEFAULT_DATE)}` }),
    },
    {
      key: "timeFormat",
      label: "Time format",
      description: "For {{time}}. HH is the hour, mm the minutes.",
      placeholder: DEFAULT_TIME,
      check: (value, host) => ({ example: `Now: ${formatDate(host.now(), value || DEFAULT_TIME)}` }),
    },
  ],
  commands: [{ id: "insert", name: "Insert template", run: insertTemplate, available: (host) => host.canInsert() }],
};
