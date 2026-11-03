<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import Cover from "../components/Cover.svelte";
  import {
    analyzeImport,
    cancelImport,
    commitImport,
    pickImportFiles,
    pickImportFolder,
    previewUrl,
    type AnalyzeProgress,
    type ImportPlan,
    type ImportResult,
  } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { formatNumber, parseNumber, plural } from "../lib/format";

  let { paths, fromRoots = false }: { paths: string[]; fromRoots?: boolean } = $props();

  interface ChapterDraft {
    id: string;
    seriesKey: string;
    displayName: string;
    sourcePath: string;
    volume: string;
    number: string;
    title: string;
    pageCount: number;
    duplicate: boolean;
    hasComicInfo: boolean;
    include: boolean;
  }

  interface SeriesDraft {
    key: string;
    title: string;
    existingSeriesId: string | null;
    mergeIntoExisting: boolean;
    coverChapterId: string | null;
  }

  let phase = $state<"pick" | "analyzing" | "review" | "saving" | "done">("pick");
  let progress = $state<AnalyzeProgress>({ done: 0, total: 0 });
  let plan = $state<ImportPlan | null>(null);
  let seriesDrafts = $state<SeriesDraft[]>([]);
  let chapterDrafts = $state<ChapterDraft[]>([]);
  let copyFiles = $state(false);
  let ignoreExcluded = $state(true);
  // Un nouvel import lancé depuis cet écran ne vient plus des dossiers suivis.
  let rootImport = $state(untrack(() => fromRoots));
  let result = $state<ImportResult | null>(null);
  let committed = false;

  const included = $derived(chapterDrafts.filter((c) => c.include));
  const invalid = $derived(
    included.some((c) => parseNumber(c.number) === undefined || parseNumber(c.volume) === undefined) ||
      seriesDrafts.some(
        (s) => !s.mergeIntoExisting && !s.title.trim() && included.some((c) => c.seriesKey === s.key),
      ),
  );

  // L'analyse part dès l'ouverture quand des chemins sont fournis (glisser-déposer).
  $effect(() => {
    if (paths.length > 0) analyze(paths);
  });

  onDestroy(() => {
    if (!committed) cancelImport().catch(() => {});
  });

  function text(value: number | null) {
    return value === null ? "" : formatNumber(value);
  }

  async function analyze(selection: string[]) {
    if (selection.length === 0) return;
    phase = "analyzing";
    progress = { done: 0, total: 0 };
    const found = await app.attempt(() => analyzeImport(selection, (p) => (progress = p)));
    if (app.error === "analyse annulée") app.error = null;
    if (!found) {
      phase = "pick";
      return;
    }
    plan = found;
    seriesDrafts = found.series.map((s) => ({
      key: s.key,
      title: s.title,
      existingSeriesId: s.existingSeriesId,
      mergeIntoExisting: s.existingSeriesId !== null,
      coverChapterId: s.coverChapterId,
    }));
    chapterDrafts = found.series.flatMap((s) =>
      s.chapters.map((c) => ({
        id: c.id,
        seriesKey: s.key,
        displayName: c.displayName,
        sourcePath: c.sourcePath,
        volume: text(c.volume),
        number: text(c.number),
        title: c.title ?? "",
        pageCount: c.pageCount,
        duplicate: c.duplicate,
        hasComicInfo: c.hasComicInfo,
        include: !c.duplicate,
      })),
    );
    phase = "review";
  }

  async function pick(picker: () => Promise<string[]>) {
    const selection = await app.attempt(picker);
    if (selection) await analyze(selection);
  }

  function cancelAnalysis() {
    cancelImport().catch(() => {});
    phase = "pick";
  }

  async function commit() {
    if (!plan || invalid) return;
    const sessionId = plan.sessionId;
    phase = "saving";
    const done = await app.attempt(() =>
      commitImport(sessionId, {
        copyFiles,
        ignoreExcluded: rootImport && ignoreExcluded,
        series: seriesDrafts.map((s) => ({
          key: s.key,
          title: s.title,
          existingSeriesId: s.mergeIntoExisting ? s.existingSeriesId : null,
        })),
        chapters: chapterDrafts.map((c) => ({
          id: c.id,
          seriesKey: c.seriesKey,
          volume: parseNumber(c.volume) ?? null,
          number: parseNumber(c.number) ?? null,
          title: c.title.trim() || null,
          include: c.include,
        })),
      }),
    );
    if (!done) {
      phase = "review";
      return;
    }
    committed = true;
    result = done;
    phase = "done";
    await app.changed();
    app.scanRoots().catch(() => {});
  }

  function reset() {
    rootImport = false;
    cancelImport().catch(() => {});
    plan = null;
    seriesDrafts = [];
    chapterDrafts = [];
    phase = "pick";
  }

  function setAll(key: string, include: boolean) {
    for (const chapter of chapterDrafts) {
      if (chapter.seriesKey === key && !chapter.duplicate) chapter.include = include;
    }
  }
</script>

<div class="page">
  <h1>Importer</h1>

  {#if phase === "pick"}
    <div class="pick">
      <p>Choisissez des archives CBZ, CB7, CBR ou ZIP, des images ou un dossier complet. Vous pouvez aussi les déposer dans cette fenêtre.</p>
      <div class="buttons">
        <button class="button primary" onclick={() => pick(pickImportFolder)}>Choisir un dossier</button>
        <button class="button" onclick={() => pick(pickImportFiles)}>Choisir des fichiers</button>
      </div>
      <p class="note">Rien n'est importé avant votre validation, et vos fichiers ne sont jamais modifiés.</p>
    </div>
  {:else if phase === "analyzing"}
    <div class="pick" role="status">
      <p>Analyse en cours…</p>
      {#if progress.total > 0}
        <progress max={progress.total} value={progress.done}></progress>
        <p class="note">{progress.done.toLocaleString("fr-FR")} sur {plural(progress.total, "élément")}</p>
      {/if}
      <div class="buttons"><button class="button" onclick={cancelAnalysis}>Annuler</button></div>
    </div>
  {:else if phase === "saving"}
    <div class="pick" role="status"><p>Import en cours…</p></div>
  {:else if phase === "done" && result}
    {@const firstSeries = result.firstSeriesId}
    <div class="pick">
      <p class="done">
        {plural(result.chaptersAdded, "chapitre ajouté", "chapitres ajoutés")}{#if result.seriesCreated > 0}, {plural(result.seriesCreated, "série créée", "séries créées")}{/if}.
      </p>
      {#if result.relocated > 0}
        <p class="note">{plural(result.relocated, "fichier déplacé retrouvé", "fichiers déplacés retrouvés")} : la progression est conservée.</p>
      {/if}
      {#if result.coversFailed > 0}
        <p class="note">{plural(result.coversFailed, "couverture n'a pas pu être créée", "couvertures n'ont pas pu être créées")}.</p>
      {/if}
      <div class="buttons">
        {#if firstSeries}
          <button class="button primary" onclick={() => app.go({ name: "series", id: firstSeries })}>Voir la série</button>
          {#if result.seriesCreated > 0}
            <button class="button" onclick={() => app.go({ name: "series", id: firstSeries, identify: true })}>Identifier la série</button>
          {/if}
        {/if}
        <button class="button" onclick={() => app.go({ name: "library" })}>Bibliothèque</button>
        <button class="button" onclick={reset}>Importer autre chose</button>
      </div>
    </div>
  {:else if phase === "review" && plan}
    {@const sessionId = plan.sessionId}
    {#if plan.relocated > 0}
      <p class="note">{plural(plan.relocated, "fichier déplacé retrouvé", "fichiers déplacés retrouvés")} : l'emplacement sera mis à jour, sans perte de progression.</p>
    {/if}

    {#if plan.issues.length > 0}
      <details class="issues">
        <summary>{plural(plan.issues.length, "élément ignoré ou signalé", "éléments ignorés ou signalés")}</summary>
        <ul>
          {#each plan.issues as issue, index (index)}
            <li><span class="path">{issue.path}</span> {issue.message}</li>
          {/each}
        </ul>
      </details>
    {/if}

    {#if seriesDrafts.length === 0}
      <div class="pick">
        <p>Aucun chapitre à importer dans cette sélection.</p>
        <div class="buttons"><button class="button" onclick={reset}>Choisir autre chose</button></div>
      </div>
    {/if}

    {#each seriesDrafts as series (series.key)}
      {@const chapters = chapterDrafts.filter((c) => c.seriesKey === series.key)}
      <section>
        <div class="series-head">
          <div class="cover">
            <Cover src={series.coverChapterId ? previewUrl(sessionId, series.coverChapterId) : null} title={series.title} />
          </div>
          <div class="series-fields">
            <label class="title-label">
              Série
              <input class="field title-input" bind:value={series.title} disabled={series.mergeIntoExisting} />
            </label>
            {#if series.existingSeriesId}
              <label class="check">
                <input type="checkbox" bind:checked={series.mergeIntoExisting} />
                Ajouter à la série déjà présente dans la bibliothèque
              </label>
            {/if}
            <p class="note">
              {plural(chapters.filter((c) => c.include).length, "chapitre sélectionné", "chapitres sélectionnés")} sur {chapters.length}
              · <button class="link" onclick={() => setAll(series.key, true)}>Tout</button>
              · <button class="link" onclick={() => setAll(series.key, false)}>Aucun</button>
            </p>
          </div>
        </div>

        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th class="narrow"><span class="sr">Inclure</span></th>
                <th class="num">Vol.</th>
                <th class="num">N°</th>
                <th>Titre</th>
                {#if seriesDrafts.length > 1}<th class="move">Série</th>{/if}
                <th class="pages">Pages</th>
                <th>Source</th>
              </tr>
            </thead>
            <tbody>
              {#each chapters as chapter (chapter.id)}
                <tr class:off={!chapter.include}>
                  <td class="narrow">
                    <input type="checkbox" bind:checked={chapter.include} disabled={chapter.duplicate} aria-label="Inclure {chapter.displayName}" />
                  </td>
                  <td class="num">
                    <input class="cell" class:bad={parseNumber(chapter.volume) === undefined} bind:value={chapter.volume} inputmode="decimal" aria-label="Volume" />
                  </td>
                  <td class="num">
                    <input class="cell" class:bad={parseNumber(chapter.number) === undefined} bind:value={chapter.number} inputmode="decimal" aria-label="Numéro" />
                  </td>
                  <td><input class="cell" bind:value={chapter.title} aria-label="Titre" /></td>
                  {#if seriesDrafts.length > 1}
                    <td class="move">
                      <select class="cell" bind:value={chapter.seriesKey} aria-label="Série">
                        {#each seriesDrafts as target (target.key)}
                          <option value={target.key}>{target.title}</option>
                        {/each}
                      </select>
                    </td>
                  {/if}
                  <td class="pages">{chapter.pageCount}</td>
                  <td class="source" title={chapter.sourcePath}>
                    <span class="source-name">{chapter.displayName}</span>
                    {#if chapter.duplicate}<span class="tag">Déjà importé</span>{/if}
                    {#if chapter.hasComicInfo}<span class="tag">ComicInfo</span>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    {/each}

    {#if seriesDrafts.length > 0}
      <footer>
        <label class="check">
          <input type="checkbox" bind:checked={copyFiles} />
          Copier les fichiers dans la bibliothèque Komago
        </label>
        {#if rootImport}
          <label class="check">
            <input type="checkbox" bind:checked={ignoreExcluded} />
            Ne plus proposer les éléments décochés
          </label>
        {/if}
        <div class="buttons">
          <button class="button" onclick={reset}>Annuler</button>
          <button class="button primary" disabled={invalid || (included.length === 0 && plan.relocated === 0)} onclick={commit}>
            Importer {plural(included.length, "chapitre")}
          </button>
        </div>
      </footer>
    {/if}
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex: 1;
    flex-direction: column;
    padding: 0 28px;
  }

  h1 {
    margin: 0 0 14px;
    font-size: 22px;
    font-weight: 650;
  }

  .pick {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 480px;
    margin: 32px 0;
  }

  .pick p {
    margin: 0;
  }

  .done {
    font-size: 15px;
    font-weight: 600;
  }

  .note {
    margin: 0;
    color: var(--text-muted);
  }

  .pick .note {
    margin: 0;
  }

  progress {
    width: 100%;
    accent-color: var(--accent);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .issues {
    margin-bottom: 14px;
    color: var(--text-muted);
  }

  .issues summary {
    color: var(--danger);
  }

  .issues ul {
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
    max-height: 180px;
    overflow-y: auto;
    user-select: text;
    -webkit-user-select: text;
  }

  .issues li {
    padding: 3px 0;
  }

  .path {
    display: block;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  section {
    margin-bottom: 26px;
  }

  .series-head {
    display: flex;
    gap: 14px;
    margin-bottom: 10px;
  }

  .cover {
    flex: none;
    width: 76px;
  }

  .series-fields {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    max-width: 560px;
  }

  .title-label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .title-input {
    font-size: 14px;
    font-weight: 600;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    min-width: 640px;
    border-collapse: collapse;
  }

  th {
    padding: 4px 6px;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 500;
    text-align: left;
  }

  td {
    padding: 2px 6px;
    border-top: 1px solid var(--border);
  }

  tr.off td {
    opacity: 0.55;
  }

  .narrow {
    width: 24px;
  }

  .num {
    width: 60px;
  }

  .pages {
    width: 44px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .move {
    width: 150px;
  }

  .cell {
    width: 100%;
    padding: 3px 5px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: none;
    color: inherit;
    font: inherit;
    user-select: text;
    -webkit-user-select: text;
  }

  .cell:hover {
    border-color: var(--border);
  }

  .cell:focus {
    border-color: var(--accent);
    outline: none;
  }

  .cell.bad {
    border-color: var(--danger);
  }

  .source {
    max-width: 240px;
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tag {
    margin-left: 6px;
    color: var(--text);
    font-size: 11px;
  }

  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }

  footer {
    position: sticky;
    bottom: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: auto;
    padding: 12px 0;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
</style>
