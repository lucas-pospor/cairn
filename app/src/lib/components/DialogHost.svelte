<script lang="ts">
  import { app } from "../app.svelte";
  import { closeOnBack } from "../back";
  import { modal } from "../modal";

  let value = $state("");
  let input: HTMLInputElement | undefined = $state();
  let okButton: HTMLButtonElement | undefined = $state();
  let choices: HTMLDivElement | undefined = $state();

  // The element focused before the dialog opened, to give focus back on close.
  // A pre-effect runs before the editor drops its focus for the dialog. Read
  // once, when the dialog element mounts, so it need not be reactive.
  // svelte-ignore non_reactive_update
  let opener: Element | null = null;
  $effect.pre(() => {
    if (!app.dialog) opener = null;
    else opener ??= document.activeElement;
  });

  $effect(() => {
    const d = app.dialog;
    if (!d) return;
    if (d.kind === "prompt") {
      value = d.value;
      queueMicrotask(() => {
        input?.focus();
        if (d.selectStem) {
          const dot = d.value.lastIndexOf(".");
          input?.setSelectionRange(0, dot > 0 ? dot : d.value.length);
        } else input?.select();
      });
    } else if (d.kind === "confirm") {
      queueMicrotask(() => okButton?.focus());
    } else {
      queueMicrotask(() => choices?.querySelector<HTMLElement>(".choice")?.focus());
    }
  });

  $effect(() => {
    if (app.dialog) return closeOnBack(() => close(false));
  });

  function close(ok: boolean) {
    const d = app.dialog;
    if (!d) return;
    app.dialog = null;
    if (d.kind === "prompt") d.resolve(ok && value.trim() ? value : null);
    else if (d.kind === "confirm") d.resolve(ok);
    else d.resolve(null);
  }
</script>

{#if app.dialog}
  {@const d = app.dialog}
  <div
    class="backdrop"
    role="presentation"
    onmousedown={(e) => {
      if (e.target === e.currentTarget) close(false);
    }}
  >
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label={d.title}
      aria-describedby={d.kind === "confirm" ? "dialog-message" : undefined}
      tabindex="-1"
      use:modal={{ close: () => close(false), opener }}
    >
      <h3>{d.title}</h3>
      {#if d.kind === "prompt"}
        <form
          onsubmit={(e) => {
            e.preventDefault();
            close(true);
          }}
        >
          <input class="text-input" bind:this={input} bind:value aria-label={d.title} data-testid="dialog-input" />
          <div class="buttons">
            <button type="button" class="btn" onclick={() => close(false)}>Cancel</button>
            <button type="submit" class="btn primary" data-testid="dialog-ok">{d.okLabel}</button>
          </div>
        </form>
      {:else if d.kind === "choose"}
        <div class="choices" bind:this={choices}>
          {#each d.options as o}
            <button
              class="choice"
              onclick={() => {
                // `d` follows app.dialog: take resolve before clearing it.
                const resolve = d.resolve;
                app.dialog = null;
                resolve(o.value);
              }}>{o.label}</button
            >
          {/each}
        </div>
        <div class="buttons">
          <button class="btn" onclick={() => close(false)}>Cancel</button>
        </div>
      {:else}
        <p id="dialog-message">{d.message}</p>
        <div class="buttons">
          <button class="btn" onclick={() => close(false)}>Cancel</button>
          <button class="btn {d.danger ? 'danger' : 'primary'}" bind:this={okButton} onclick={() => close(true)} data-testid="dialog-ok"
            >{d.okLabel}</button
          >
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.25);
    display: grid;
    place-items: start center;
    padding-top: 18vh;
    z-index: 100;
  }
  .dialog {
    width: min(420px, calc(100vw - 32px));
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
    padding: 18px 20px;
  }
  h3 {
    margin: 0 0 12px;
    font-size: 15px;
  }
  p {
    margin: 0 0 4px;
    line-height: 1.5;
  }
  .choices {
    max-height: 50vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .choice {
    text-align: left;
    padding: 10px 12px;
    border-radius: 7px;
  }
  .choice:hover {
    background: var(--accent-soft);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
