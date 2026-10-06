// Unique note creator: a new note named by the date and time (YYYYMMDDHHmm by
// default), in a chosen folder, from a template if one is set. A name that is
// taken gets a number ("202610051432 1"), so nothing is ever written over.

import { displayName, parent } from "../paths";
import { errorMessage, isCoreError } from "../types";
import { option, type CoreHost, type CorePlugin } from "./core";
import { formatDate } from "./dates";
import { folderCheck, noteOption, notePath, notePathProblem, templateCheck } from "./files";
import { fillTemplate, templateValues } from "./templates";

export const DEFAULT_FORMAT = "YYYYMMDDHHmm";

/** The path a unique note made at `date` would have if the name is free. */
export function uniqueName(date: Date, folder: string, format: string): string {
  return notePath(folder, formatDate(date, format.trim() || DEFAULT_FORMAT));
}

async function createUnique(host: CoreHost) {
  const wanted = uniqueName(host.now(), option(uniqueNote, "folder"), option(uniqueNote, "format"));
  const problem = notePathProblem(wanted);
  if (problem) return host.toast(`A new note cannot be called ${wanted}: ${problem} Change it in Settings > Core plugins.`, "error");
  const template = noteOption(option(uniqueNote, "template"));
  let text: string | null = null;
  if (template) {
    try {
      text = await host.readNote(template);
    } catch (e) {
      return host.toast(`Could not read the template ${template}: ${errorMessage(e)}`, "error");
    }
  }
  // A note that appears at the free name before it is created (sync, another app)
  // is left alone, and the next free name is taken.
  for (let attempt = 0; attempt < 5; attempt++) {
    const path = await host.uniquePath(parent(wanted), displayName(wanted));
    try {
      await host.createNote(path, text === null ? "" : fillTemplate(text, templateValues(host, path)));
    } catch (e) {
      if (isCoreError(e) && e.kind === "alreadyExists" && e.detail === path) continue;
      return host.toast(`Could not create ${path}: ${errorMessage(e)}`, "error");
    }
    return host.openNote(path, true);
  }
  host.toast(`Could not find a free name for ${wanted}.`, "error");
}

export const uniqueNote: CorePlugin = {
  id: "unique-note",
  name: "Unique note creator",
  description: "Create a note named by the date and time, so that each new note has a name of its own.",
  defaultOn: false,
  defaults: { folder: "", format: DEFAULT_FORMAT, template: "" },
  options: [
    {
      key: "folder",
      label: "Folder",
      description: "Where new notes go. Empty means the notebook folder.",
      placeholder: "Notebook folder",
      check: folderCheck,
    },
    {
      key: "format",
      label: "Name format",
      description: "YYYY is the year, MM the month, DD the day, HH the hour, mm the minutes. A taken name gets a number.",
      placeholder: DEFAULT_FORMAT,
      check(value, host, get) {
        const path = uniqueName(host.now(), get("folder"), value);
        const problem = notePathProblem(path);
        return problem ? { problem: `Not a valid name: ${problem}` } : { example: `A note made now: ${path}` };
      },
    },
    {
      key: "template",
      label: "Template",
      description: "A note that each new note starts from. Empty means none.",
      placeholder: "None",
      check: templateCheck,
    },
  ],
  commands: [{ id: "create", name: "Create unique note", run: createUnique }],
};
