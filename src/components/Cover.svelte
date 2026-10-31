<script lang="ts">
  let { src, title }: { src: string | null; title: string } = $props();
  let failedSrc = $state<string | null>(null);
</script>

<div class="cover">
  {#if src && failedSrc !== src}
    <img {src} alt="" loading="lazy" decoding="async" onerror={() => (failedSrc = src)} />
  {:else}
    <span>{title}</span>
  {/if}
</div>

<style>
  .cover {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    aspect-ratio: 2 / 3;
    overflow: hidden;
    border-radius: 6px;
    background: var(--hover);
    box-shadow: inset 0 0 0 1px var(--border);
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  span {
    display: -webkit-box;
    padding: 10px;
    overflow: hidden;
    color: var(--text-muted);
    font-size: 12px;
    text-align: center;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 5;
    line-clamp: 5;
  }
</style>
