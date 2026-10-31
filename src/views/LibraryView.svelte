<script lang="ts">
  import ChapterList from "../components/ChapterList.svelte";
  import Cover from "../components/Cover.svelte";
  import Modal from "../components/Modal.svelte";
  import {
    collectionChapters,
    coverUrl,
    deleteCollection,
    listSeries,
    removeChapterFromCollection,
    renameCollection,
    searchChapters,
    type ChapterRow,
    type SeriesCard,
    type SeriesSort,
  } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { KINDS, plural, STATUSES } from "../lib/format";
  import { settings } from "../lib/settings.svelte";

  let { collectionId }: { collectionId: string | null } = $props();

  let series = $state<SeriesCard[]>([]);
  let chapters = $state<ChapterRow[]>([]);
  let loaded = $state(false);
  let sort = $state<SeriesSort>("title");
  let kind = $state("");
  let status = $state("");
  let renaming = $state(false);
  let confirmingDelete = $state(false);
  let draftName = $state("");

  const query = $derived(app.search.trim());
  const collection = $derived(app.collections.find((c) => c.id === collectionId) ?? null);
  const layout = $derived(settings.current?.libraryView ?? "grid");
  const filtered = $derived(Boolean(query || kind || status));
  const heading = $derived(query ? `Résultats pour « ${query} »` : (collection?.name ?? "Bibliothèque"));

  $effect(() => {
    // Lecture des dépendances avant l'appel asynchrone.
    const filter = { query, kind: kind || null, status: status || null, collectionId, sort };
    const collection = collectionId;
    void app.revision;
    let current = true;
    app.attempt(async () => {
      const [foundSeries, foundChapters] = await Promise.all([
        listSeries(filter),
        filter.query
          ? searchChapters(filter.query)
          : collection
            ? collectionChapters(collection)
            : Promise.resolve([]),
      ]);
      if (current) {
        series = foundSeries;
        chapters = foundChapters;
        loaded = true;
      }
    });
    return () => {
      current = false;
    };
  });

  function startRename() {
    draftName = collection?.name ?? "";
    renaming = true;
  }

  async function submitRename(event: SubmitEvent) {
    event.preventDefault();
    await app.attempt(() => renameCollection(collectionId!, draftName));
    renaming = false;
    await app.changed();
  }

  async function confirmDelete() {
    await app.attempt(() => deleteCollection(collectionId!));
    confirmingDelete = false;
    await app.changed();
    app.go({ name: "library" });
  }

  async function removeChapter(chapter: ChapterRow) {
    await app.attempt(() => removeChapterFromCollection(collectionId!, chapter.id));
    await app.changed();
  }
</script>

<div class="page">
  <header>
    <div class="heading">
      <h1>{heading}</h1>
      {#if loaded}<span class="count">{plural(series.length, "série")}</span>{/if}
      {#if collection && !collection.systemKey && !query}
        <button class="link" onclick={startRename}>Renommer</button>
        <button class="link" onclick={() => (confirmingDelete = true)}>Supprimer</button>
      {/if}
    </div>
    <div class="tools">
      <select bind:value={kind} aria-label="Filtrer par type">
        <option value="">Tous les types</option>
        {#each Object.entries(KINDS) as [value, label] (value)}
          <option {value}>{label}</option>
        {/each}
      </select>
      <select bind:value={status} aria-label="Filtrer par statut">
        <option value="">Tous les statuts</option>
        {#each Object.entries(STATUSES) as [value, label] (value)}
          <option {value}>{label}</option>
        {/each}
      </select>
      <select bind:value={sort} aria-label="Trier">
        <option value="title">Titre</option>
        <option value="read">Lecture récente</option>
        <option value="added">Ajout récent</option>
        <option value="updated">Modification récente</option>
        <option value="chapters">Nombre de chapitres</option>
      </select>
      <div class="segmented" role="radiogroup" aria-label="Affichage">
        <button
          role="radio"
          aria-checked={layout === "grid"}
          class:active={layout === "grid"}
          onclick={() => app.attempt(() => settings.update({ libraryView: "grid" }))}
        >
          Grille
        </button>
        <button
          role="radio"
          aria-checked={layout === "list"}
          class:active={layout === "list"}
          onclick={() => app.attempt(() => settings.update({ libraryView: "list" }))}
        >
          Liste
        </button>
      </div>
    </div>
  </header>

  {#if series.length > 0}
    {#if layout === "grid"}
      <div class="grid">
        {#each series as item (item.id)}
          <button class="card" onclick={() => app.go({ name: "series", id: item.id })}>
            <Cover src={item.coverAssetId ? coverUrl(item.coverAssetId) : null} title={item.title} />
            {#if item.readCount > 0}
              <span class="bar"><span style:width="{(item.readCount / Math.max(1, item.chapterCount)) * 100}%"></span></span>
            {/if}
            <span class="card-title">{item.title}</span>
            <span class="card-meta">
              {plural(item.chapterCount, "chapitre")}
              {#if item.missingCount > 0}<span class="danger"> · {item.missingCount} introuvable{item.missingCount > 1 ? "s" : ""}</span>{/if}
            </span>
          </button>
        {/each}
      </div>
    {:else}
      <div class="list">
        {#each series as item (item.id)}
          <button class="row" onclick={() => app.go({ name: "series", id: item.id })}>
            <div class="thumb">
              <Cover src={item.coverAssetId ? coverUrl(item.coverAssetId) : null} title="" />
            </div>
            <span class="row-title">{item.title}</span>
            <span class="row-meta">{item.kind === "unknown" ? "" : KINDS[item.kind]}</span>
            <span class="row-meta">{item.volumeCount > 0 ? plural(item.volumeCount, "volume") : ""}</span>
            <span class="row-meta">{item.readCount > 0 ? `${item.readCount} / ` : ""}{plural(item.chapterCount, "chapitre")}</span>
            {#if item.missingCount > 0}<span class="row-meta danger">{item.missingCount} introuvable{item.missingCount > 1 ? "s" : ""}</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
  {/if}

  {#if chapters.length > 0}
    <section>
      <h2>Chapitres</h2>
      <ChapterList {chapters} showSeries onopen={(c) => app.go({ name: "reader", chapterId: c.id })}>
        {#snippet actions(chapter)}
          {#if collectionId && !query}
            <button class="link" onclick={() => removeChapter(chapter)}>Retirer</button>
          {/if}
        {/snippet}
      </ChapterList>
    </section>
  {/if}

  {#if loaded && series.length === 0 && chapters.length === 0}
    <div class="empty">
      {#if filtered}
        <h2>Aucun résultat</h2>
        <p>Essayez d'autres mots ou retirez des filtres.</p>
      {:else if collection}
        <h2>Cette collection est vide</h2>
        <p>Ajoutez-y des séries ou des chapitres depuis la page d'une série.</p>
      {:else}
        <h2>Votre bibliothèque est vide</h2>
        <p>Importez des CBZ, des ZIP ou des dossiers d'images, ou déposez-les dans cette fenêtre.</p>
        <button class="button primary" onclick={() => app.go({ name: "import", paths: [] })}>Importer</button>
      {/if}
    </div>
  {/if}
</div>

{#if renaming}
  <Modal title="Renommer la collection" onclose={() => (renaming = false)}>
    <form id="rename-collection" onsubmit={submitRename}>
      <input class="field" bind:value={draftName} aria-label="Nom de la collection" required />
    </form>
    {#snippet actions()}
      <button class="button" onclick={() => (renaming = false)}>Annuler</button>
      <button class="button primary" type="submit" form="rename-collection" disabled={!draftName.trim()}>Renommer</button>
    {/snippet}
  </Modal>
{/if}

{#if confirmingDelete}
  <Modal title="Supprimer cette collection ?" onclose={() => (confirmingDelete = false)}>
    <p class="note">Les séries et les chapitres qu'elle contient restent dans la bibliothèque.</p>
    {#snippet actions()}
      <button class="button" onclick={() => (confirmingDelete = false)}>Annuler</button>
      <button class="button danger-button" onclick={confirmDelete}>Supprimer</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .page {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 18px;
    padding: 0 28px 32px;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px 20px;
  }

  .heading {
    display: flex;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }

  h1 {
    margin: 0;
    overflow: hidden;
    font-size: 22px;
    font-weight: 650;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count,
  .note {
    flex: none;
    margin: 0;
    color: var(--text-muted);
  }

  .tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  select {
    max-width: 170px;
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(136px, 1fr));
    gap: 22px 18px;
  }

  .card {
    /* Une bibliothèque de milliers de séries ne dessine que ce qui est visible. */
    content-visibility: auto;
    contain-intrinsic-size: auto 260px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 0;
    text-align: left;
  }

  .card:hover :global(.cover) {
    box-shadow:
      inset 0 0 0 1px var(--border),
      0 4px 14px rgb(0 0 0 / 0.22);
  }

  .bar {
    display: block;
    height: 3px;
    margin-top: 5px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--hover);
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .card-title {
    display: -webkit-box;
    margin-top: 6px;
    overflow: hidden;
    font-weight: 550;
    line-height: 1.3;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .card-meta,
  .row-meta {
    color: var(--text-muted);
    font-size: 12px;
  }

  .danger {
    color: var(--danger);
  }

  .row {
    display: grid;
    grid-template-columns: 30px minmax(0, 1fr) 90px 80px 90px auto;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 5px 4px;
    border-bottom: 1px solid var(--border);
    text-align: left;
  }

  .row:hover {
    background: var(--hover);
  }

  .row-title {
    overflow: hidden;
    font-weight: 550;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  section h2,
  .empty h2 {
    margin: 0 0 6px;
    font-size: 14px;
    font-weight: 600;
  }

  .empty {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    padding-bottom: var(--titlebar-height);
    text-align: center;
  }

  .empty h2 {
    font-size: 17px;
  }

  .empty p {
    margin: 0 0 12px;
    color: var(--text-muted);
  }
</style>
