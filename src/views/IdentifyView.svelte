<script lang="ts">
  import { onMount } from "svelte";
  import Cover from "../components/Cover.svelte";
  import MetadataDialog from "../components/MetadataDialog.svelte";
  import {
    cancelIdentifyBatch,
    coverUrl,
    identifyBatch,
    listUnidentified,
    type BatchEntry,
    type SeriesCard,
  } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { plural } from "../lib/format";
  import { settings } from "../lib/settings.svelte";

  let series = $state<SeriesCard[]>([]);
  let selected = $state<Record<string, boolean>>({});
  let loaded = $state(false);
  let running = $state(false);
  let progress = $state({ done: 0, total: 0 });
  let results = $state<BatchEntry[]>([]);
  let choosing = $state<{ id: string; title: string } | null>(null);

  const chosen = $derived(series.filter((s) => selected[s.id]));
  const networkOn = $derived(settings.current?.networkEnabled ?? false);
  const OUTCOMES: Record<BatchEntry["outcome"], string> = {
    applied: "Associée",
    uncertain: "À vérifier",
    none: "Aucun résultat",
    failed: "Échec",
  };

  onMount(load);

  async function load() {
    const found = await app.attempt(listUnidentified);
    if (found) {
      series = found;
      selected = Object.fromEntries(found.map((s) => [s.id, true]));
    }
    loaded = true;
  }

  async function run() {
    running = true;
    results = [];
    progress = { done: 0, total: chosen.length };
    const ids = chosen.map((s) => s.id);
    results = (await app.attempt(() => identifyBatch(ids, (p) => (progress = p)))) ?? [];
    running = false;
    await app.changed();
    await load();
  }

  function setAll(value: boolean) {
    selected = Object.fromEntries(series.map((s) => [s.id, value]));
  }
</script>

<div class="page">
  <h1>Identification</h1>

  {#if !networkOn}
    <p class="note">
      Les connexions externes sont désactivées. Activez-les dans les paramètres pour identifier vos séries
      avec MangaDex et AniList.
    </p>
    <div class="buttons">
      <button class="button primary" onclick={() => app.attempt(() => settings.update({ networkEnabled: true }))}>
        Autoriser les connexions
      </button>
    </div>
  {:else}
    <p class="note">
      Komago cherche chaque série et n'associe que les correspondances très probables, selon votre mode de
      mise à jour. Les autres restent à vérifier, une par une.
    </p>
  {/if}

  {#if results.length > 0}
    <section>
      <h2>Résultat</h2>
      <ul>
        {#each results as entry (entry.seriesId)}
          <li>
            <span class="title">{entry.title}</span>
            <span class="outcome {entry.outcome}">{OUTCOMES[entry.outcome]}</span>
            <span class="detail">
              {#if entry.matchedTitle}{entry.providerName} · {entry.matchedTitle}{/if}
              {#if entry.message}{entry.message}{/if}
            </span>
            {#if entry.outcome !== "applied"}
              <button class="link" onclick={() => (choosing = { id: entry.seriesId, title: entry.title })}>Choisir…</button>
            {:else}
              <button class="link" onclick={() => app.go({ name: "series", id: entry.seriesId })}>Voir</button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if running}
    <div class="status" role="status">
      <progress max={progress.total} value={progress.done}></progress>
      <span>{progress.done} sur {plural(progress.total, "série")}</span>
      <button class="button" onclick={() => app.attempt(cancelIdentifyBatch)}>Arrêter</button>
    </div>
  {/if}

  {#if loaded && series.length === 0}
    <p class="note">Toutes vos séries sont associées à une fiche.</p>
  {:else if series.length > 0}
    <section>
      <div class="head">
        <h2>{plural(series.length, "série non identifiée", "séries non identifiées")}</h2>
        <button class="link" onclick={() => setAll(true)}>Tout</button>
        <button class="link" onclick={() => setAll(false)}>Aucune</button>
      </div>
      <ul>
        {#each series as item (item.id)}
          <li>
            <input type="checkbox" bind:checked={selected[item.id]} aria-label="Inclure {item.title}" />
            <span class="thumb"><Cover src={item.coverAssetId ? coverUrl(item.coverAssetId) : null} title="" /></span>
            <span class="title">{item.title}</span>
            <span class="detail">{plural(item.chapterCount, "chapitre")}</span>
            <button class="link" onclick={() => (choosing = { id: item.id, title: item.title })}>Choisir…</button>
          </li>
        {/each}
      </ul>
      <div class="buttons">
        <button class="button primary" disabled={!networkOn || running || chosen.length === 0} onclick={run}>
          Identifier {plural(chosen.length, "série")}
        </button>
      </div>
    </section>
  {/if}
</div>

{#if choosing}
  <MetadataDialog
    seriesId={choosing.id}
    seriesTitle={choosing.title}
    onclose={() => (choosing = null)}
    onapplied={async () => {
      await app.changed();
      await load();
    }}
  />
{/if}

<style>
  .page {
    width: 100%;
    max-width: 860px;
    margin: 0 auto;
    padding: 0 28px 40px;
  }

  h1 {
    margin: 0 0 12px;
    font-size: 22px;
    font-weight: 650;
  }

  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .note {
    margin: 0 0 14px;
    color: var(--text-muted);
  }

  section {
    margin: 18px 0;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-bottom: 6px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 40px;
    padding: 4px 0;
    border-bottom: 1px solid var(--border);
  }

  .thumb {
    flex: none;
    width: 26px;
  }

  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    font-weight: 550;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    color: var(--text-muted);
    font-size: 12px;
  }

  .outcome {
    flex: none;
    font-size: 12px;
    font-weight: 600;
  }

  .outcome.applied {
    color: var(--accent);
  }

  .outcome.failed {
    color: var(--danger);
  }

  .outcome.uncertain,
  .outcome.none {
    color: var(--text-muted);
  }

  .buttons {
    display: flex;
    gap: 8px;
    margin-top: 14px;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 12px 0;
  }

  progress {
    flex: 1;
    accent-color: var(--accent);
  }
</style>
