<script lang="ts">
  import { app } from "../app.svelte";
  import Icon from "./Icon.svelte";

  /** A toast stays while it is pointed at or holds the focus; otherwise its time runs again. */
  function settle(el: HTMLElement, id: number, focused: EventTarget | null) {
    if (el.matches(":hover") || (focused instanceof Node && el.contains(focused))) app.holdToast(id);
    else app.releaseToast(id);
  }
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div
      class="toast {t.kind}"
      role={t.kind === "error" ? "alert" : "status"}
      onmouseenter={() => app.holdToast(t.id)}
      onmouseleave={(e) => settle(e.currentTarget, t.id, document.activeElement)}
      onfocusin={() => app.holdToast(t.id)}
      onfocusout={(e) => settle(e.currentTarget, t.id, e.relatedTarget)}
    >
      <span class="msg">{t.message}</span><button class="icon-btn close" title="Dismiss" onclick={() => app.dismissToast(t.id)}
        ><Icon name="x" size={14} /></button
      >
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 0;
    bottom: 0;
    /* Toasts sit 16px from the right and 34px from the bottom; the rest of the
       padding leaves room for their shadows, which overflow: hidden would cut. */
    padding: 24px 16px 34px 24px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 8px;
    z-index: 200;
    max-width: min(460px, 100vw);
    /* Never more than the lower half of the window: older toasts above that are cut off. */
    max-height: 50vh;
    overflow: hidden;
    /* Clicks between and around the toasts reach what is underneath. */
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    padding: 10px 8px 10px 14px;
    line-height: 1.4;
    user-select: text;
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .msg {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .close {
    flex: none;
    width: 22px;
    height: 22px;
  }
  /* Toasts sit above dialogs; while one is open, clicks go through them to the dialog. */
  :global(body:has([role="dialog"])) .toast {
    pointer-events: none;
  }
</style>
