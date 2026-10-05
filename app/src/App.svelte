<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/app.svelte";
  import { backend } from "./lib/backend";
  import { back } from "./lib/back";
  import Welcome from "./lib/components/Welcome.svelte";
  import Workspace from "./lib/components/Workspace.svelte";
  import DialogHost from "./lib/components/DialogHost.svelte";
  import Toasts from "./lib/components/Toasts.svelte";

  let ready = $state(false);

  // The window title names the open note (screen readers say it when switching windows).
  $effect(() => {
    const title = app.vault && app.active ? `${app.active.title} - Cairn` : "Cairn";
    document.title = title;
    backend.setWindowTitle(title).catch(() => {});
  });

  onMount(() => {
    app.init().finally(() => {
      ready = true;
      // Two frames: the first paint of the workspace has happened.
      requestAnimationFrame(() =>
        requestAnimationFrame(() =>
          backend.uiReady().then((ms) => ((window as unknown as { __cairnStartupMs: number }).__cairnStartupMs = ms)),
        ),
      );
    });
    const beforeUnload = () => {
      app.saveSession();
    };
    window.addEventListener("beforeunload", beforeUnload);
    // Android Back (MainActivity asks here first): close the overlay opened
    // last. With none open, save pending edits, then let the app leave, so
    // leaving cannot end the process in the middle of a save.
    const w = window as unknown as { __cairnBack?: () => boolean; CairnAndroid?: { leave(): void } };
    let leaving = false;
    w.__cairnBack = () => {
      if (back()) return true;
      const android = w.CairnAndroid;
      if (!android) return false;
      if (!leaving) {
        leaving = true;
        void app.flushAll().finally(() => {
          leaving = false;
          android.leave();
        });
      }
      return true;
    };
    return () => {
      window.removeEventListener("beforeunload", beforeUnload);
      app.dispose();
    };
  });
</script>

{#if ready}
  {#if app.vault}
    {#key app.vault.root}
      <Workspace />
    {/key}
  {:else}
    <Welcome />
  {/if}
{/if}
<DialogHost />
<Toasts />
