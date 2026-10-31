<script lang="ts">
  import type { Snippet } from "svelte";
  import type { ChapterRow } from "../lib/api";
  import { chapterLabel, plural } from "../lib/format";

  let {
    chapters,
    showSeries = false,
    onopen,
    actions,
  }: {
    chapters: ChapterRow[];
    showSeries?: boolean;
    onopen?: (chapter: ChapterRow) => void;
    actions?: Snippet<[ChapterRow]>;
  } = $props();
</script>

<ul>
  {#each chapters as chapter (chapter.id)}
    <li>
      {#if onopen}
        <button class="main link" onclick={() => onopen(chapter)}>
          {@render label(chapter)}
        </button>
      {:else}
        <div class="main">{@render label(chapter)}</div>
      {/if}
      {#if chapter.sourceStatus && chapter.sourceStatus !== "available"}
        <span class="badge" title={chapter.sourcePath}>Fichier introuvable</span>
      {/if}
      {#if chapter.completed}
        <span class="read">Lu</span>
      {:else if chapter.percent !== null}
        <span class="read">{Math.round(chapter.percent)} %</span>
      {/if}
      <span class="pages">{plural(chapter.pageCount, "page")}</span>
      {#if actions}
        <div class="actions">{@render actions(chapter)}</div>
      {/if}
    </li>
  {/each}
</ul>

{#snippet label(chapter: ChapterRow)}
  {#if showSeries}<span class="series">{chapter.seriesTitle}</span>{/if}
  <span class="number">{chapterLabel(chapter)}</span>
  {#if chapter.title}<span class="title">{chapter.title}</span>{/if}
{/snippet}

<style>
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 36px;
    padding: 4px 0;
    border-bottom: 1px solid var(--border);
  }

  .main {
    display: flex;
    flex: 1;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
    padding: 0;
    text-align: left;
  }

  .link:hover .number {
    text-decoration: underline;
  }

  .series {
    font-weight: 550;
    white-space: nowrap;
  }

  .number {
    flex: none;
  }

  .title {
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pages {
    flex: none;
    color: var(--text-muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .read {
    flex: none;
    color: var(--accent);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .badge {
    flex: none;
    color: var(--danger);
    font-size: 12px;
  }

  .actions {
    display: flex;
    flex: none;
    gap: 10px;
  }
</style>
