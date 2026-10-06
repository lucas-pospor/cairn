// FINDING-164, FINDING-083: sync setup asks before it creates a vault the
// server does not have (a mistyped or differently-cased name would start a
// second, empty vault), and creates and connects only on yes.
//
// Run: cd app && npx vitest run src/lib/syncSetup.test.ts

import { describe, it, expect, vi, beforeEach } from "vitest";

const calls: string[] = [];
let exists = false;

vi.mock("./backend", () => ({
  backend: {
    syncVaultExists: async (a: { vaultId: string }) => {
      calls.push(`exists ${a.vaultId}`);
      return exists;
    },
    syncSetup: async (a: { vaultId: string }) => {
      calls.push(`setup ${a.vaultId}`);
      return { root: "", configured: true, state: "idle", vaultId: a.vaultId, conflicts: [] };
    },
    syncCancel: async () => {
      calls.push("cancel");
    },
  },
}));

import { app } from "./app.svelte";

const ARGS = { server: "https://notes.example.com", token: "t", vaultId: "Notes", device: "phone", passphrase: "long enough" };
const tick = () => new Promise((r) => setTimeout(r, 0));

/** Start a setup and wait until it asks; returns the question and the running setup. */
async function asked() {
  const run = app.setupSync(ARGS);
  await tick();
  const d = app.dialog;
  return { d, run };
}

beforeEach(() => {
  calls.length = 0;
  app.dialog = null;
  app.syncSetup = null;
});

describe("sync setup with a vault name the server does not have", () => {
  it("asks, and on no creates nothing and leaves the form as it was", async () => {
    exists = false;
    const { d, run } = await asked();
    expect(d?.kind).toBe("confirm");
    expect(d && "message" in d ? d.message : null).toBe("There's no notebook called Notes on this server. Create it?");
    expect(app.syncSetup?.busy).toBe(true);
    if (d?.kind === "confirm") d.resolve(false);
    await run;
    expect(calls).toEqual(["exists Notes"]);
    expect(app.syncSetup).toMatchObject({ busy: false, error: null, vaultId: "Notes", server: ARGS.server });
  });

  it("creates and connects on yes", async () => {
    exists = false;
    const { d, run } = await asked();
    if (d?.kind === "confirm") d.resolve(true);
    await run;
    expect(calls).toEqual(["exists Notes", "setup Notes"]);
    expect(app.syncSetup).toBeNull();
  });

  it("does not create the vault when the setup was cancelled while asking", async () => {
    exists = false;
    const { d, run } = await asked();
    app.cancelSyncSetup();
    if (d?.kind === "confirm") d.resolve(true);
    await run;
    expect(calls).toEqual(["exists Notes", "cancel"]);
  });

  it("connects without asking when the server has the vault", async () => {
    exists = true;
    await app.setupSync(ARGS);
    expect(app.dialog).toBeNull();
    expect(calls).toEqual(["exists Notes", "setup Notes"]);
  });
});
