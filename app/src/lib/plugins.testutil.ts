// Test helper for the plugin host tests (mocked backend, fake Worker).
//
// A plugin starts only if the user turned it on on this device (see plugins.ts).
// Most host tests are about something else, so their mocked backend answers
// `pluginApprovals` with approveAll(): every plugin file in the mocked
// `.cairn/` config map counts as turned on, for its current content. Tests
// about consent itself record approvals with host.approve() instead.

import { parseManifest, sourceHash, type PluginApproval } from "./plugins";

/** Approvals for every `plugins/...` entry of a mocked config map, as Settings > Plugins records them. */
export function approveAll(config: Map<string, string>): Record<string, PluginApproval> {
  const out: Record<string, PluginApproval> = {};
  for (const [path, source] of config) {
    if (!path.startsWith("plugins/")) continue;
    const file = path.slice("plugins/".length);
    out[file] = { hash: sourceHash(source), permissions: parseManifest(file, source).permissions };
  }
  return out;
}
