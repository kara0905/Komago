<script lang="ts">
  import { onMount } from "svelte";
  import {
    applyMetadata,
    candidateCoverUrl,
    previewMetadata,
    refreshMetadata,
    rejectMetadata,
    searchMetadata,
    type FieldChange,
    type Match,
    type MetadataPreview,
    type ProviderFailure,
  } from "../lib/api";
  import { KINDS, STATUSES } from "../lib/format";
  import { settings } from "../lib/settings.svelte";
  import Cover from "./Cover.svelte";
  import Modal from "./Modal.svelte";

  let {
    seriesId,
    seriesTitle,
    refreshProvider = null,
    onclose,
    onapplied,
  }: {
    seriesId: string;
    seriesTitle: string;
    /** Actualisation d'une fiche déjà associée plutôt que recherche. */
    refreshProvider?: string | null;
    onclose: () => void;
    onapplied: () => void;
  } = $props();

  const FIELD_LABELS: Record<string, string> = {
    title: "Titre",
    alt_titles: "Titres alternatifs",
    description: "Description",
    kind: "Type",
    status: "Statut",
    publisher: "Éditeur",
    original_language: "Langue originale",
    published_from: "Début de publication",
    published_to: "Fin de publication",
    authors: "Auteurs",
    artists: "Artistes",
    genres: "Genres",
    tags: "Tags",
    cover: "Couverture",
  };

  let phase = $state<"loading" | "offline" | "results" | "preview" | "applying" | "done">("loading");
  let query = $state("");
  let token = $state("");
  let matches = $state<Match[]>([]);
  let failures = $state<ProviderFailure[]>([]);
  let preview = $state<MetadataPreview | null>(null);
  let chosen = $state<Record<string, boolean>>({});
  let error = $state<string | null>(null);
  let notes = $state<string[]>([]);

  const networkOn = $derived(settings.current?.networkEnabled ?? false);
  const visibleChanges = $derived(preview?.changes.filter((c) => c.kind !== "same") ?? []);
  const sameCount = $derived((preview?.changes.length ?? 0) - visibleChanges.length);
  const chosenCount = $derived(visibleChanges.filter((c) => chosen[c.field]).length);

  onMount(() => {
    query = seriesTitle;
    start();
  });

  function start() {
    if (!networkOn) {
      phase = "offline";
    } else if (refreshProvider) {
      refresh(refreshProvider);
    } else {
      search();
    }
  }

  async function guarded(action: () => Promise<void>, fallback: typeof phase) {
    error = null;
    try {
      await action();
    } catch (cause) {
      error = String(cause);
      phase = fallback;
    }
  }

  function search() {
    phase = "loading";
    return guarded(async () => {
      const outcome = await searchMetadata(seriesId, query);
      token = outcome.token;
      matches = outcome.matches;
      failures = outcome.failures;
      phase = "results";
    }, "results");
  }

  function refresh(providerId: string) {
    phase = "loading";
    return guarded(async () => {
      const result = await refreshMetadata(seriesId, providerId);
      token = result.token;
      await showPreview(result);
    }, "results");
  }

  function select(index: number) {
    phase = "loading";
    return guarded(async () => {
      await showPreview(await previewMetadata(token, index));
    }, "results");
  }

  async function showPreview(result: MetadataPreview) {
    // En mode automatique, seuls les champs vides sont complétés, sans étape de plus.
    if (result.mode === "automatic") {
      preview = result;
      await apply(null);
      return;
    }
    preview = result;
    chosen = Object.fromEntries(result.changes.map((c) => [c.field, c.selected]));
    phase = "preview";
  }

  function apply(fields: string[] | null) {
    if (!preview) return;
    const current = preview;
    phase = "applying";
    return guarded(async () => {
      const outcome = await applyMetadata(token, current.index, fields);
      notes = [
        outcome.applied.length > 0
          ? `Champs mis à jour : ${outcome.applied.map((f) => FIELD_LABELS[f] ?? f).join(", ")}.`
          : refreshProvider
            ? "Rien à changer : les informations sont à jour."
            : "Série associée, sans changement de ses informations.",
      ];
      if (outcome.coverFailed) notes.push("La couverture n'a pas pu être récupérée.");
      onapplied();
      phase = "done";
    }, "preview");
  }

  function reject(match: Match) {
    return guarded(async () => {
      await rejectMetadata(token, match.index);
      matches = matches.filter((m) => m.index !== match.index);
    }, "results");
  }

  async function enableNetwork() {
    await guarded(() => settings.update({ networkEnabled: true }), "offline");
    if (settings.current?.networkEnabled) start();
  }

  function display(change: FieldChange, value: string) {
    if (change.field === "kind") return KINDS[value] ?? value;
    if (change.field === "status") return STATUSES[value] ?? value;
    return value;
  }
</script>

<Modal title={refreshProvider ? "Actualiser les informations" : "Identifier la série"} wide {onclose}>
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if phase === "offline"}
    <p>Les connexions externes sont désactivées. Komago reste entièrement utilisable sans elles.</p>
    <p class="note">
      Pour chercher cette série, Komago enverra son titre à MangaDex et AniList. Aucune autre information
      de votre bibliothèque n'est transmise.
    </p>
  {:else if phase === "loading" || phase === "applying"}
    <p class="note" role="status">{phase === "loading" ? "Recherche en cours…" : "Enregistrement…"}</p>
  {:else if phase === "done"}
    {#each notes as note, index (index)}<p>{note}</p>{/each}
  {:else if phase === "results"}
    <form class="search" onsubmit={(e) => { e.preventDefault(); search(); }}>
      <input class="field" bind:value={query} aria-label="Titre à rechercher" />
      <button class="button" type="submit" disabled={!query.trim()}>Rechercher</button>
    </form>

    {#each failures as failure, index (index)}
      <p class="error">{failure.providerName} : {failure.message}</p>
    {/each}

    {#if matches.length === 0 && !error}
      <p class="note">Aucun résultat. Essayez un autre titre, par exemple le titre original.</p>
    {/if}

    <ul class="results">
      {#each matches as match (match.index)}
        <li>
          <div class="thumb">
            <Cover src={match.hasCover ? candidateCoverUrl(token, match.index) : null} title="" />
          </div>
          <div class="info">
            <span class="title">{match.series.title}</span>
            <span class="meta">
              {match.providerName}
              {#if match.series.authors.length > 0} · {match.series.authors.join(", ")}{/if}
              {#if match.series.publishedFrom} · {match.series.publishedFrom.slice(0, 4)}{/if}
              {#if match.series.kind} · {KINDS[match.series.kind] ?? match.series.kind}{/if}
            </span>
            {#if match.series.description}<span class="summary">{match.series.description}</span>{/if}
            <span class="score" class:certain={match.certain}>
              {match.certain ? "Très probable" : "À vérifier"} · {Math.round(match.score * 100)} %
            </span>
          </div>
          <div class="actions">
            <button class="button" class:primary={match.certain} onclick={() => select(match.index)}>Choisir</button>
            <button class="link" onclick={() => reject(match)}>Écarter</button>
          </div>
        </li>
      {/each}
    </ul>
  {:else if phase === "preview" && preview}
    <p class="note">
      Source : {preview.providerName}.
      {#if preview.mode === "manual"}
        Mode manuel : rien n'est coché, seuls les champs que vous choisissez changeront.
      {:else}
        Les champs vides sont cochés ; le reste attend votre décision.
      {/if}
      {#if sameCount > 0}{sameCount} champ{sameCount > 1 ? "s" : ""} déjà identique{sameCount > 1 ? "s" : ""}.{/if}
    </p>

    {#if visibleChanges.length === 0}
      <p>Rien à changer : les informations sont déjà à jour.</p>
    {/if}

    <div class="changes">
      {#each visibleChanges as change (change.field)}
        <label class="change" class:conflict={change.kind === "conflict"}>
          <input type="checkbox" bind:checked={chosen[change.field]} />
          <span class="field-name">{FIELD_LABELS[change.field] ?? change.field}</span>
          <span class="values">
            {#if change.locked}<span class="locked">Protégé : modifié à la main. Cochez seulement si vous voulez le remplacer.</span>{/if}
            {#if change.field === "cover"}
              <span class="cover-preview"><Cover src={candidateCoverUrl(token, preview.index)} title="" /></span>
              {#if change.kind === "conflict"}<span class="origin">Remplacerait la couverture actuelle</span>{/if}
            {:else}
              {#if change.current}
                <span class="current">{display(change, change.current)}</span>
                {#if change.currentSource}<span class="origin">Actuellement : {change.currentSource}</span>{/if}
              {/if}
              <span class="proposed">{display(change, change.proposed)}</span>
            {/if}
          </span>
        </label>
      {/each}
    </div>
  {/if}

  {#snippet actions()}
    {#if phase === "offline"}
      <button class="button" onclick={onclose}>Annuler</button>
      <button class="button primary" onclick={enableNetwork}>Autoriser les connexions</button>
    {:else if phase === "preview" && preview}
      {#if !refreshProvider}<button class="button" onclick={() => (phase = "results")}>Retour</button>{/if}
      <button class="button" onclick={onclose}>Annuler</button>
      <button
        class="button primary"
        onclick={() => apply(visibleChanges.filter((c) => chosen[c.field]).map((c) => c.field))}
      >
        {chosenCount > 0 ? `Appliquer ${chosenCount} champ${chosenCount > 1 ? "s" : ""}` : "Associer sans rien changer"}
      </button>
    {:else}
      <button class="button" onclick={onclose}>Fermer</button>
    {/if}
  {/snippet}
</Modal>

<style>
  p {
    margin: 0;
  }

  .note,
  .meta,
  .origin {
    color: var(--text-muted);
  }

  .error {
    color: var(--danger);
    user-select: text;
    -webkit-user-select: text;
  }

  .search {
    display: flex;
    gap: 8px;
  }

  .results {
    display: flex;
    flex-direction: column;
    max-height: 52vh;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  li {
    display: flex;
    gap: 12px;
    padding: 10px 0;
    border-top: 1px solid var(--border);
  }

  .thumb {
    flex: none;
    width: 58px;
  }

  .info {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title {
    font-weight: 600;
  }

  .meta,
  .score {
    font-size: 12px;
  }

  .summary {
    display: -webkit-box;
    overflow: hidden;
    font-size: 12px;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .score {
    color: var(--text-muted);
  }

  .score.certain {
    color: var(--accent);
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex: none;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
  }

  .changes {
    display: flex;
    flex-direction: column;
    max-height: 52vh;
    overflow-y: auto;
  }

  .change {
    display: grid;
    grid-template-columns: 18px 130px minmax(0, 1fr);
    gap: 8px;
    padding: 8px 0;
    border-top: 1px solid var(--border);
  }

  .field-name {
    font-weight: 550;
  }

  .values {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-wrap: anywhere;
    max-height: 150px;
    overflow-y: auto;
  }

  .current {
    color: var(--text-muted);
    text-decoration: line-through;
  }

  .origin,
  .locked {
    font-size: 12px;
  }

  .locked {
    color: var(--danger);
  }

  .cover-preview {
    width: 70px;
  }
</style>
