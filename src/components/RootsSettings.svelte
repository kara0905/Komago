<script lang="ts">
  import { onMount } from "svelte";
  import {
    addRoot,
    clearIgnoredSources,
    listRoots,
    pickImportFolder,
    removeRoot,
    type LibraryRoot,
  } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { plural } from "../lib/format";

  let roots = $state<LibraryRoot[]>([]);
  let ignored = $state(0);
  let scanning = $state(false);

  onMount(load);

  async function load() {
    const overview = await app.attempt(listRoots);
    if (overview) {
      roots = overview.roots;
      ignored = overview.ignored;
    }
  }

  async function add() {
    const [path] = (await app.attempt(pickImportFolder)) ?? [];
    if (!path) return;
    const id = await app.attempt(() => addRoot(path));
    if (!id) return;
    await load();
    // Premier passage : tout le dossier passe par le plan d'import habituel.
    app.go({ name: "import", paths: [path], fromRoots: true });
  }

  async function remove(root: LibraryRoot) {
    await app.attempt(() => removeRoot(root.id));
    await load();
    await app.scanRoots().catch(() => {});
  }

  async function scan() {
    scanning = true;
    await app.attempt(() => app.scanRoots());
    await load();
    scanning = false;
  }

  async function resetIgnored() {
    await app.attempt(clearIgnoredSources);
    await scan();
  }
</script>

<section>
  <h2>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/><path d="M12 11v4m-2-2h4"/></svg>
    Dossiers suivis
  </h2>
  <p class="intro">
    Komago y repère les nouveaux chapitres au démarrage. Rien n'est importé sans votre validation, et
    vos fichiers restent là où ils sont.
  </p>

  {#each roots as root (root.id)}
    <div class="row">
      <span class="root">
        <span class="path">{root.path}</span>
        <small>
          {#if root.status === "missing"}
            <span class="danger">Indisponible : disque débranché ou dossier déplacé.</span>
          {:else}
            {plural(root.sourceCount, "fichier importé", "fichiers importés")}
          {/if}
        </small>
      </span>
      <button class="button" onclick={() => remove(root)}>Ne plus suivre</button>
    </div>
  {/each}

  <div class="actions">
    <button class="button" onclick={add}>Ajouter un dossier</button>
    {#if roots.length > 0}
      <button class="button" disabled={scanning} onclick={scan}>
        {scanning ? "Recherche…" : "Rechercher les nouveautés"}
      </button>
    {/if}
    {#if app.newInRoots.length > 0}
      <button class="button primary" onclick={() => app.go({ name: "import", paths: app.newInRoots, fromRoots: true })}>
        Importer {plural(app.newInRoots.length, "nouveauté")}
      </button>
    {/if}
  </div>
  {#if ignored > 0}
    <p class="intro">
      {plural(ignored, "élément écarté", "éléments écartés")} lors d'un import précédent.
      <button class="link" onclick={resetIgnored}>Les proposer à nouveau</button>
    </p>
  {/if}
</section>

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

  .root {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .path {
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }

  small {
    color: var(--text-muted);
    font-size: 12px;
  }

  .danger {
    color: var(--danger);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 12px 0 8px;
  }
</style>
