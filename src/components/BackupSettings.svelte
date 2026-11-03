<script lang="ts">
  import { discardRestore, exportBackup, restartApp, stageRestore, type StagedRestore } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { plural } from "../lib/format";
  import Modal from "./Modal.svelte";

  let includeLibrary = $state(false);
  let working = $state(false);
  let saved = $state<string | null>(null);
  let staged = $state<StagedRestore | null>(null);

  async function backup() {
    working = true;
    saved = null;
    saved = (await app.attempt(() => exportBackup(includeLibrary))) ?? null;
    working = false;
  }

  async function prepareRestore() {
    working = true;
    staged = (await app.attempt(stageRestore)) ?? null;
    working = false;
  }

  async function cancelRestore() {
    staged = null;
    await app.attempt(discardRestore);
  }

  function created(date: string) {
    return new Date(date).toLocaleString("fr-FR", { dateStyle: "long", timeStyle: "short" });
  }
</script>

<section>
  <h2>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m0 0-4-4m4 4 4-4"/><path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/></svg>
    Sauvegarde
  </h2>
  <p class="intro">
    Un seul fichier contient votre bibliothèque : séries, progression, collections, métadonnées et
    couvertures. Vos fichiers de mangas d'origine n'y sont pas inclus.
  </p>
  <label class="row">
    <span>
      Inclure les fichiers copiés dans Komago
      <small>Peut rendre la sauvegarde très volumineuse.</small>
    </span>
    <input type="checkbox" bind:checked={includeLibrary} />
  </label>
  <div class="actions">
    <button class="button" disabled={working} onclick={backup}>Créer une sauvegarde</button>
    <button class="button" disabled={working} onclick={prepareRestore}>Restaurer…</button>
  </div>
  {#if working}<p class="intro" role="status">En cours…</p>{/if}
  {#if saved}<p class="intro" role="status">Sauvegarde enregistrée : <span class="path">{saved}</span></p>{/if}
</section>

{#if staged}
  <Modal title="Restaurer cette sauvegarde ?" onclose={cancelRestore}>
    <p>
      Sauvegarde du {created(staged.manifest.createdAt)} :
      {plural(staged.seriesCount, "série")}, {plural(staged.chapterCount, "chapitre")}.
    </p>
    <p class="intro">
      Komago va redémarrer. Votre bibliothèque actuelle n'est pas effacée : elle est mise de côté dans le
      dossier de données (« backups »).
    </p>
    {#snippet actions()}
      <button class="button" onclick={cancelRestore}>Annuler</button>
      <button class="button primary" onclick={() => app.attempt(restartApp)}>Restaurer et redémarrer</button>
    {/snippet}
  </Modal>
{/if}

<style>
  section {
    margin-bottom: 28px;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 4px;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
  }

  h2 svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  p {
    margin: 0;
  }

  .intro {
    margin: 0 0 8px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    min-height: 40px;
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
  }

  .row span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  small {
    color: var(--text-muted);
    font-size: 12px;
  }

  .path {
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 12px 0 8px;
  }
</style>
