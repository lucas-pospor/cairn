// The core plugins, in the order Settings > Core plugins lists them (by name).

import type { CorePlugin } from "./core";
import { dailyNotes } from "./dailyNotes";
import { templates } from "./templates";
import { uniqueNote } from "./uniqueNote";

export const CORE_PLUGINS: CorePlugin[] = [dailyNotes, templates, uniqueNote];
