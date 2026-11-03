<script lang="ts">
  import { onMount } from "svelte";
  import ChapterList from "../components/ChapterList.svelte";
  import Cover from "../components/Cover.svelte";
  import MetadataDialog from "../components/MetadataDialog.svelte";
  import Modal from "../components/Modal.svelte";
  import {
    addChapterToCollection,
    addSeriesToCollection,
    coverUrl,
    createVolume,
    deleteChapter,
    deleteSeries,
    deleteVolume,
    getSeries,
    removeSeriesFromCollection,
    setChapterRead,
    setSeriesMetadataMode,
    fillChapterTitles,
    openSeriesLink,
    unlinkMetadata,
    type MetadataMode,
    setCoverFromChapter,
    updateChapter,
    updateSeries,
    updateVolume,
    type ChapterRow,
    type SeriesDetail,
    type VolumeRow,
  } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { formatNumber, KINDS, parseNumber, plural, splitList, STATUSES, volumeLabel } from "../lib/format";

  let { id, identify = false }: { id: string; identify?: boolean } = $props();

  let series = $state<SeriesDetail | null>(null);
  let missing = $state(false);

  // Une seule fenêtre à la fois.
  type Dialog =
    | { type: "series" }
    | { type: "metadata"; refreshProvider: string | null }
    | { type: "delete-series" }
    | { type: "collections" }
    | { type: "chapter"; chapter: ChapterRow }
    | { type: "chapter-collection"; chapter: ChapterRow }
    | { type: "delete-chapter"; chapter: ChapterRow }
    | { type: "volume"; volume: VolumeRow | null }
    | { type: "delete-volume"; volume: VolumeRow };
  let dialog = $state<Dialog | null>(null);

  let form = $state({
    title: "",
    kind: "",
    status: "",
    description: "",
    publisher: "",
    publishedFrom: "",
    publishedTo: "",
    originalLanguage: "",
    altTitles: "",
    authors: "",
    artists: "",
    genres: "",
    tags: "",
  });
  let number = $state("");
  let itemTitle = $state("");
  let itemVolume = $state("");

  const numberValue = $derived(parseNumber(number));

  const looseChapters = $derived(series?.chapters.filter((c) => !c.volumeId) ?? []);

  onMount(() => {
    if (identify) dialog = { type: "metadata", refreshProvider: null };
  });

  async function load() {
    const found = await app.attempt(() => getSeries(id));
    if (found) series = found;
    else missing = true;
  }

  $effect(() => {
    void app.revision;
    load();
  });

  const FIELD_NAMES: Record<string, string> = {
    title: "titre",
    alt_titles: "titres alternatifs",
    description: "description",
    kind: "type",
    status: "statut",
    publisher: "éditeur",
    original_language: "langue",
    published_from: "début",
    published_to: "fin",
    authors: "auteurs",
    artists: "artistes",
    genres: "genres",
    tags: "tags",
    cover: "couverture",
  };

  const groupedSources = $derived.by(() => {
    const groups = new Map<string, string[]>();
    for (const source of series?.fieldSources ?? []) {
      const list = groups.get(source.sourceName) ?? [];
      list.push(FIELD_NAMES[source.field] ?? source.field);
      groups.set(source.sourceName, list);
    }
    return [...groups];
  });

  async function changeMode(value: string) {
    await app.attempt(() => setSeriesMetadataMode(id, (value || null) as MetadataMode | null));
    await app.changed();
  }

  let titlesNote = $state<string | null>(null);

  async function fillTitles(sourceId: string) {
    titlesNote = "Recherche des titres…";
    const filled = await app.attempt(() => fillChapterTitles(id, sourceId));
    titlesNote =
      filled === undefined
        ? null
        : filled === 0
          ? "Aucun titre à compléter : les chapitres ont déjà un titre, ou MangaDex n'en connaît pas."
          : `${plural(filled, "titre ajouté", "titres ajoutés")}.`;
    if (filled) await app.changed();
  }

  async function unlink(sourceId: string) {
    await app.attempt(() => unlinkMetadata(id, sourceId));
    await app.changed();
  }

  function openSeriesForm() {
    if (!series) return;
    form = {
      title: series.title,
      kind: series.kind,
      status: series.status,
      description: series.description ?? "",
      publisher: series.publisher ?? "",
      publishedFrom: series.publishedFrom ?? "",
      publishedTo: series.publishedTo ?? "",
      originalLanguage: series.originalLanguage ?? "",
      altTitles: series.altTitles.join(", "),
      authors: series.authors.join(", "),
      artists: series.artists.join(", "),
      genres: series.genres.join(", "),
      tags: series.tags.join(", "),
    };
    dialog = { type: "series" };
  }

  async function saveSeries(event: SubmitEvent) {
    event.preventDefault();
    const saved = await app.attempt(() =>
      updateSeries(id, {
        title: form.title,
        kind: form.kind,
        status: form.status,
        description: form.description,
        publisher: form.publisher,
        publishedFrom: form.publishedFrom,
        publishedTo: form.publishedTo,
        originalLanguage: form.originalLanguage,
        altTitles: splitList(form.altTitles),
        authors: splitList(form.authors),
        artists: splitList(form.artists),
        genres: splitList(form.genres),
        tags: splitList(form.tags),
      }),
    );
    dialog = null;
    if (saved) await app.changed();
  }

  async function removeSeries() {
    await app.attempt(() => deleteSeries(id));
    dialog = null;
    await app.changed();
    app.go({ name: "library" });
  }

  async function toggleCollection(collectionId: string, inside: boolean) {
    await app.attempt(() =>
      inside ? removeSeriesFromCollection(collectionId, id) : addSeriesToCollection(collectionId, id),
    );
    await app.changed();
  }

  function openChapter(chapter: ChapterRow) {
    number = chapter.number === null ? "" : formatNumber(chapter.number);
    itemTitle = chapter.title ?? "";
    itemVolume = chapter.volumeId ?? "";
    dialog = { type: "chapter", chapter };
  }

  async function saveChapter(event: SubmitEvent, chapter: ChapterRow) {
    event.preventDefault();
    if (numberValue === undefined) return;
    await app.attempt(() =>
      updateChapter(chapter.id, {
        number: numberValue ?? null,
        title: itemTitle,
        volumeId: itemVolume || null,
        seriesId: chapter.seriesId,
      }),
    );
    dialog = null;
    await app.changed();
  }

  async function removeChapter(chapter: ChapterRow) {
    await app.attempt(() => deleteChapter(chapter.id));
    dialog = null;
    await app.changed();
  }

  async function toggleRead(chapter: ChapterRow) {
    await app.attempt(() => setChapterRead(chapter.id, !chapter.completed));
    await app.changed();
  }

  async function useAsCover(chapter: ChapterRow) {
    await app.attempt(() => setCoverFromChapter(id, chapter.id));
    await app.changed();
  }

  async function addChapter(chapter: ChapterRow, collectionId: string) {
    await app.attempt(() => addChapterToCollection(collectionId, chapter.id));
    dialog = null;
    await app.changed();
  }

  function openVolume(volume: VolumeRow | null) {
    number = volume?.number == null ? "" : formatNumber(volume.number);
    itemTitle = volume?.title ?? "";
    dialog = { type: "volume", volume };
  }

  async function saveVolume(event: SubmitEvent, volume: VolumeRow | null) {
    event.preventDefault();
    if (numberValue === undefined) return;
    const patch = {
      number: numberValue ?? null,
      title: itemTitle,
      description: volume?.description ?? null,
      publishedOn: volume?.publishedOn ?? null,
    };
    await app.attempt(async () => {
      if (volume) await updateVolume(volume.id, patch);
      else await createVolume(id, patch);
    });
    dialog = null;
    await app.changed();
  }

  async function removeVolume(volume: VolumeRow) {
    await app.attempt(() => deleteVolume(volume.id));
    dialog = null;
    await app.changed();
  }
</script>

{#if series}
  <div class="page">
    <button class="link back" onclick={() => app.go({ name: "library" })}>Bibliothèque</button>

    <header>
      <div class="cover">
        <Cover src={series.coverAssetId ? coverUrl(series.coverAssetId) : null} title={series.title} />
      </div>
      <div class="info">
        <h1>{series.title}</h1>
        {#if series.altTitles.length > 0}<p class="alt">{series.altTitles.join(" · ")}</p>{/if}
        <p class="facts">
          {#if series.kind !== "unknown"}<span>{KINDS[series.kind]}</span>{/if}
          {#if series.status !== "unknown"}<span>{STATUSES[series.status]}</span>{/if}
          {#if series.volumes.length > 0}<span>{plural(series.volumes.length, "volume")}</span>{/if}
          <span>{plural(series.chapters.length, "chapitre")}</span>
        </p>
        {#if series.authors.length > 0 || series.artists.length > 0 || series.publisher || series.genres.length > 0 || series.tags.length > 0}
          <dl>
            {#if series.authors.length > 0}<dt>Auteur</dt><dd>{series.authors.join(", ")}</dd>{/if}
            {#if series.artists.length > 0}<dt>Artiste</dt><dd>{series.artists.join(", ")}</dd>{/if}
            {#if series.publisher}<dt>Éditeur</dt><dd>{series.publisher}</dd>{/if}
            {#if series.genres.length > 0}<dt>Genres</dt><dd>{series.genres.join(", ")}</dd>{/if}
            {#if series.tags.length > 0}<dt>Tags</dt><dd>{series.tags.join(", ")}</dd>{/if}
          </dl>
        {/if}
        {#if series.description}<p class="description">{series.description}</p>{/if}
        <div class="buttons">
          {#if series.resumeChapterId}
            {@const resume = series.resumeChapterId}
            <button class="button primary" onclick={() => app.go({ name: "reader", chapterId: resume })}>
              {series.hasProgress ? "Reprendre" : "Lire"}
            </button>
          {/if}
          <button class="button" onclick={() => (dialog = { type: "metadata", refreshProvider: null })}>Identifier</button>
          <button class="button" onclick={openSeriesForm}>Modifier</button>
          <button class="button" onclick={() => (dialog = { type: "collections" })}>Collections</button>
          <button class="button" onclick={() => (dialog = { type: "delete-series" })}>Retirer</button>
        </div>
      </div>
    </header>

    <section class="metadata">
      <h2>Informations</h2>
      <div class="meta-row">
        <span class="note">Mises à jour</span>
        <select
          class="mode"
          value={series.metadataMode ?? ""}
          aria-label="Mode de mise à jour des informations"
          onchange={(e) => changeMode(e.currentTarget.value)}
        >
          <option value="">Réglage général</option>
          <option value="automatic">Automatique : compléter les champs vides</option>
          <option value="assisted">Assisté : proposer avant de modifier</option>
          <option value="manual">Manuel : ne rien modifier de lui-même</option>
        </select>
      </div>
      {#each series.externalLinks as link (link.sourceId)}
        <div class="meta-row">
          <span class="source-name">{link.sourceName}</span>
          {#if link.url}<button class="link" title={link.url} onclick={() => app.attempt(() => openSeriesLink(id, link.sourceId))}>Voir la fiche</button>{/if}
          <button class="link" onclick={() => (dialog = { type: "metadata", refreshProvider: link.sourceId })}>Actualiser</button>
          {#if link.sourceId === "mangadex"}
            <button class="link" onclick={() => fillTitles(link.sourceId)}>Titres des chapitres</button>
          {/if}
          <button class="link" onclick={() => unlink(link.sourceId)}>Dissocier</button>
        </div>
      {/each}
      {#if titlesNote}<p class="note provenance" role="status">{titlesNote}</p>{/if}
      {#each groupedSources as [source, fields] (source)}
        <p class="note provenance">{source} : {fields.join(", ")}</p>
      {/each}
    </section>

    <div class="section-head">
      <h2>Contenu</h2>
      <button class="link" onclick={() => openVolume(null)}>Ajouter un volume</button>
    </div>

    {#each series.volumes as volume (volume.id)}
      {@const chapters = series.chapters.filter((c) => c.volumeId === volume.id)}
      <section>
        <div class="volume-head">
          <h3>{volumeLabel(volume)}</h3>
          <button class="link" onclick={() => openVolume(volume)}>Modifier</button>
          <button class="link" onclick={() => (dialog = { type: "delete-volume", volume })}>Supprimer</button>
        </div>
        {#if chapters.length > 0}
          {@render list(chapters)}
        {:else}
          <p class="note">Aucun chapitre dans ce volume.</p>
        {/if}
      </section>
    {/each}

    {#if looseChapters.length > 0}
      <section>
        {#if series.volumes.length > 0}
          <div class="volume-head"><h3>Hors volume</h3></div>
        {/if}
        {@render list(looseChapters)}
      </section>
    {/if}
  </div>
{:else if missing}
  <div class="page"><p class="note">Cette série n'existe plus.</p></div>
{/if}

{#snippet list(chapters: ChapterRow[])}
  <ChapterList {chapters} onopen={(chapter) => app.go({ name: "reader", chapterId: chapter.id })}>
    {#snippet actions(chapter)}
      <button class="link" onclick={() => toggleRead(chapter)}>{chapter.completed ? "Non lu" : "Lu"}</button>
      <button class="link" onclick={() => openChapter(chapter)}>Modifier</button>
      <button class="link" onclick={() => (dialog = { type: "chapter-collection", chapter })}>Collection</button>
      <button class="link" onclick={() => useAsCover(chapter)}>Couverture</button>
      <button class="link" onclick={() => (dialog = { type: "delete-chapter", chapter })}>Retirer</button>
    {/snippet}
  </ChapterList>
{/snippet}

{#if dialog?.type === "metadata" && series}
  <MetadataDialog
    seriesId={id}
    seriesTitle={series.title}
    refreshProvider={dialog.refreshProvider}
    onclose={() => (dialog = null)}
    onapplied={() => app.changed()}
  />
{:else if dialog?.type === "series"}
  <Modal title="Modifier la série" wide onclose={() => (dialog = null)}>
    <form id="series-form" class="form" onsubmit={saveSeries}>
      <label class="span">Titre <input class="field" bind:value={form.title} required /></label>
      <label class="span">Titres alternatifs <input class="field" bind:value={form.altTitles} placeholder="Séparés par des virgules" /></label>
      <label>Type
        <select class="field" bind:value={form.kind}>
          {#each Object.entries(KINDS) as [value, label] (value)}<option {value}>{label}</option>{/each}
        </select>
      </label>
      <label>Statut
        <select class="field" bind:value={form.status}>
          {#each Object.entries(STATUSES) as [value, label] (value)}<option {value}>{label}</option>{/each}
        </select>
      </label>
      <label>Auteurs <input class="field" bind:value={form.authors} placeholder="Séparés par des virgules" /></label>
      <label>Artistes <input class="field" bind:value={form.artists} placeholder="Séparés par des virgules" /></label>
      <label>Genres <input class="field" bind:value={form.genres} /></label>
      <label>Tags <input class="field" bind:value={form.tags} /></label>
      <label>Éditeur <input class="field" bind:value={form.publisher} /></label>
      <label>Langue originale <input class="field" bind:value={form.originalLanguage} placeholder="ja, fr, en…" /></label>
      <label>Début de publication <input class="field" bind:value={form.publishedFrom} placeholder="2019-04" /></label>
      <label>Fin de publication <input class="field" bind:value={form.publishedTo} /></label>
      <label class="span">Description <textarea class="field" rows="4" bind:value={form.description}></textarea></label>
    </form>
    <p class="note">Vos modifications ne seront jamais écrasées par une actualisation automatique.</p>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
      <button class="button primary" type="submit" form="series-form" disabled={!form.title.trim()}>Enregistrer</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "delete-series"}
  <Modal title="Retirer cette série ?" onclose={() => (dialog = null)}>
    <p class="note">
      La série, sa progression et ses métadonnées quittent la bibliothèque. Vos fichiers d'origine ne
      sont pas supprimés ; seules les copies gérées par Komago sont effacées.
    </p>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
      <button class="button danger-button" onclick={removeSeries}>Retirer</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "collections" && series}
  {@const current = series}
  <Modal title="Collections" onclose={() => (dialog = null)}>
    <div class="checks">
      {#each app.collections as collection (collection.id)}
        {@const inside = current.collectionIds.includes(collection.id)}
        <label>
          <input type="checkbox" checked={inside} onchange={() => toggleCollection(collection.id, inside)} />
          {collection.name}
        </label>
      {/each}
    </div>
    <p class="note">En cours, Terminés et Abandonnés s'excluent mutuellement.</p>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Fermer</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "chapter" && series}
  {@const chapter = dialog.chapter}
  {@const volumes = series.volumes}
  <Modal title="Modifier le chapitre" onclose={() => (dialog = null)}>
    <form id="chapter-form" class="form single" onsubmit={(e) => saveChapter(e, chapter)}>
      <label>Numéro <input class="field" bind:value={number} inputmode="decimal" /></label>
      {#if numberValue === undefined}<p class="error">Saisissez un nombre positif, ou rien.</p>{/if}
      <label>Titre <input class="field" bind:value={itemTitle} /></label>
      <label>Volume
        <select class="field" bind:value={itemVolume}>
          <option value="">Hors volume</option>
          {#each volumes as volume (volume.id)}<option value={volume.id}>{volumeLabel(volume)}</option>{/each}
        </select>
      </label>
    </form>
    {#if chapter.sourcePath}<p class="path">{chapter.sourcePath}</p>{/if}
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
      <button class="button primary" type="submit" form="chapter-form" disabled={numberValue === undefined}>Enregistrer</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "chapter-collection"}
  {@const chapter = dialog.chapter}
  <Modal title="Ajouter le chapitre à une collection" onclose={() => (dialog = null)}>
    <div class="choices">
      {#each app.collections as collection (collection.id)}
        <button class="choice" onclick={() => addChapter(chapter, collection.id)}>{collection.name}</button>
      {/each}
    </div>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "delete-chapter"}
  {@const chapter = dialog.chapter}
  <Modal title="Retirer ce chapitre ?" onclose={() => (dialog = null)}>
    <p class="note">Il quitte la bibliothèque avec sa progression. Votre fichier d'origine n'est pas supprimé.</p>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
      <button class="button danger-button" onclick={() => removeChapter(chapter)}>Retirer</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "volume"}
  {@const volume = dialog.volume}
  <Modal title={volume ? "Modifier le volume" : "Ajouter un volume"} onclose={() => (dialog = null)}>
    <form id="volume-form" class="form single" onsubmit={(e) => saveVolume(e, volume)}>
      <label>Numéro <input class="field" bind:value={number} inputmode="decimal" /></label>
      {#if numberValue === undefined}<p class="error">Saisissez un nombre positif, ou rien.</p>{/if}
      <label>Titre <input class="field" bind:value={itemTitle} /></label>
    </form>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
      <button class="button primary" type="submit" form="volume-form" disabled={numberValue === undefined}>Enregistrer</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "delete-volume"}
  {@const volume = dialog.volume}
  <Modal title="Supprimer ce volume ?" onclose={() => (dialog = null)}>
    <p class="note">Ses chapitres restent dans la série, hors volume.</p>
    {#snippet actions()}
      <button class="button" onclick={() => (dialog = null)}>Annuler</button>
      <button class="button danger-button" onclick={() => removeVolume(volume)}>Supprimer</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .page {
    width: 100%;
    max-width: 980px;
    margin: 0 auto;
    padding: 0 28px 40px;
  }

  .back {
    margin-bottom: 12px;
  }

  header {
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    gap: 24px;
    margin-bottom: 28px;
  }

  h1 {
    margin: 0;
    font-size: 24px;
    font-weight: 680;
    line-height: 1.2;
    user-select: text;
    -webkit-user-select: text;
  }

  .alt {
    margin: 4px 0 0;
    color: var(--text-muted);
    user-select: text;
    -webkit-user-select: text;
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 14px;
    margin: 10px 0;
    color: var(--text-muted);
  }

  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 2px 12px;
    margin: 0 0 10px;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    user-select: text;
    -webkit-user-select: text;
  }

  .description {
    margin: 0 0 14px;
    white-space: pre-line;
    user-select: text;
    -webkit-user-select: text;
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .metadata {
    margin-bottom: 24px;
  }

  .metadata h2 {
    margin-bottom: 6px;
  }

  .meta-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 12px;
    min-height: 28px;
  }

  .source-name {
    font-weight: 550;
  }

  .mode {
    max-width: 100%;
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  .provenance {
    font-size: 12px;
  }

  .section-head,
  .volume-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  .section-head {
    justify-content: space-between;
    margin-bottom: 8px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 650;
  }

  section {
    margin-bottom: 20px;
  }

  h3 {
    margin: 0 0 2px;
    font-size: 13px;
    font-weight: 600;
  }

  .note {
    margin: 0;
    color: var(--text-muted);
  }

  .path {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 12px;
  }

  .form {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    max-height: 55vh;
    overflow-y: auto;
    padding: 2px;
  }

  .form.single {
    grid-template-columns: 1fr;
  }

  .form label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .form .span {
    grid-column: 1 / -1;
  }

  .checks {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .checks label {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .choices {
    display: flex;
    flex-direction: column;
    max-height: 50vh;
    overflow-y: auto;
  }

  .choice {
    padding: 6px 8px;
    border-radius: 6px;
    text-align: left;
  }

  .choice:hover {
    background: var(--hover);
  }

  @media (max-width: 720px) {
    header {
      grid-template-columns: 120px minmax(0, 1fr);
      gap: 16px;
    }
  }
</style>
