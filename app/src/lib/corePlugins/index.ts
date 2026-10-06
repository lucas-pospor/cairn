// The core plugins, in the order Settings > Core plugins lists them.

import type { CorePlugin } from "./core";
import { templates } from "./templates";

export const CORE_PLUGINS: CorePlugin[] = [templates];
