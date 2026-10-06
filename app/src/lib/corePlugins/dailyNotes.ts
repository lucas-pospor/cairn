// Daily notes: open today's note, named by the date in a chosen folder, and
// create it (from a template, if one is set) when there is none. It is never
// created over a file: if a note is already there, that note opens.

import { errorMessage } from "../types";
import { option, type CoreHost, type CorePlugin } from "./core";
import { formatDate } from "./dates";
import { createOrOpen, folderCheck, noteOption, notePath, notePathProblem, templateCheck } from "./files";
import { fillTemplate, templateValues } from "./templates";

export const DEFAULT_FORMAT = "YYYY-MM-DD";

/** The path of the daily note of `date`, with the folder and format given. */
export function dailyPath(date: Date, folder: string, format: string): string {
  return notePath(folder, formatDate(date, format.trim() || DEFAULT_FORMAT));
}

async function openToday(host: CoreHost) {
  const path = dailyPath(host.now(), option(dailyNotes, "folder"), option(dailyNotes, "format"));
  const problem = notePathProblem(path);
  if (problem) return host.toast(`Today's note cannot be called ${path}: ${problem} Change it in Settings > Core plugins.`, "error");
  if (host.files().includes(path)) return host.openNote(path);
  let content = "";
  const template = noteOption(option(dailyNotes, "template"));
  if (template) {
    try {
      content = fillTemplate(await host.readNote(template), templateValues(host, path));
    } catch (e) {
      return host.toast(`Could not read the daily note template ${template}: ${errorMessage(e)}`, "error");
    }
  }
  await createOrOpen(host, path, content);
}

export const dailyNotes: CorePlugin = {
  id: "daily-notes",
  name: "Daily notes",
  description: "Open today's note, named by the date. It is created, from a template if you choose one, when there is none.",
  defaultOn: true,
  defaults: { folder: "", format: DEFAULT_FORMAT, template: "" },
  options: [
    {
      key: "folder",
      label: "Folder",
      description: "Where new daily notes go. Empty means the notebook folder.",
      placeholder: "Notebook folder",
      check: folderCheck,
    },
    {
      key: "format",
      label: "Date format",
      description: "The name of each daily note. YYYY is the year, MM the month, DD the day. A slash makes a folder.",
      placeholder: DEFAULT_FORMAT,
      check(value, host, get) {
        const path = dailyPath(host.now(), get("folder"), value);
        const problem = notePathProblem(path);
        return problem ? { problem: `Not a valid name: ${problem}` } : { example: `Today's note: ${path}` };
      },
    },
    {
      key: "template",
      label: "Template",
      description: "A note that each new daily note starts from. Empty means none.",
      placeholder: "None",
      check: templateCheck,
    },
  ],
  commands: [{ id: "today", name: "Open today's note", run: openToday }],
};
