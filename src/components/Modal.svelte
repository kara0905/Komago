<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    onclose,
    children,
    actions,
    wide = false,
  }: {
    title: string;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
    wide?: boolean;
  } = $props();

  let dialog: HTMLDialogElement;

  $effect(() => {
    dialog.showModal();
  });
</script>

<dialog bind:this={dialog} class:wide {onclose} aria-label={title}>
  <h2>{title}</h2>
  <div class="body">{@render children()}</div>
  {#if actions}
    <div class="actions">{@render actions()}</div>
  {/if}
</dialog>

<style>
  dialog {
    width: min(440px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    padding: 20px;
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.3);
  }

  dialog.wide {
    width: min(620px, calc(100vw - 32px));
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.35);
  }

  h2 {
    margin: 0 0 14px;
    font-size: 15px;
    font-weight: 650;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 18px;
  }
</style>
