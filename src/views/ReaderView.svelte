<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onDestroy, onMount, tick } from "svelte";
  import {
    openChapter,
    pageUrl,
    saveProgress,
    setSeriesDirection,
    type ReaderChapter,
    type ReaderFit,
    type ReaderMode,
  } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { chapterLabel } from "../lib/format";
  import {
    anchorAt,
    buildOffsets,
    buildSpreads,
    displaySize,
    FALLBACK_SIZE,
    pageAt,
    pagedPercent,
    preloadOrder,
    scrollTopFor,
    spreadOf,
    verticalPercent,
    visibleRange,
    type Anchor,
    type PageSize,
  } from "../lib/reader-layout";
  import { settings } from "../lib/settings.svelte";

  let { chapterId }: { chapterId: string } = $props();

  const SAVE_DELAY = 400;
  const HIDE_DELAY = 2500;
  const PRELOAD_AHEAD = 6;
  const PRELOAD_BEHIND = 2;

  let chapter = $state<ReaderChapter | null>(null);
  let sizes = $state.raw<PageSize[]>([]);
  let failed = $state<Record<number, number>>({});
  let loadError = $state<string | null>(null);

  let scroller = $state<HTMLDivElement>();
  let viewportWidth = $state(0);
  let viewportHeight = $state(0);
  let scrollTop = $state(0);

  let currentPage = $state(0);
  let currentSpread = $state(0);
  let restored = false;

  let controlsVisible = $state(true);
  let panelOpen = $state(false);
  let fullscreen = $state(false);
  let endMessage = $state<string | null>(null);
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingSave = false;
  let lastPercent = 0;
  let lastAnchor: Anchor = { page: 0, offset: 0 };
  // Les images préchargées doivent rester référencées pour ne pas être libérées.
  let preloaded = new Map<number, HTMLImageElement>();

  const mode = $derived<ReaderMode>(
    (chapter?.direction as ReaderMode | null) ?? settings.current?.readerMode ?? "ltr",
  );
  const fit = $derived<ReaderFit>(
    (mode === "vertical" ? settings.current?.readerVerticalFit : settings.current?.readerFit) ?? "height",
  );
  const double = $derived(settings.current?.readerDoublePage ?? false);
  const background = $derived(settings.current?.readerBackground ?? "#111111");
  const autoHide = $derived(settings.current?.readerAutoHide ?? true);
  const vertical = $derived(mode === "vertical");
  const pageCount = $derived(sizes.length);

  // Défilement vertical
  const displayed = $derived(
    vertical && viewportWidth > 0
      ? sizes.map((size) => displaySize(size, fit, viewportWidth, viewportHeight))
      : [],
  );
  const offsets = $derived(buildOffsets(displayed.map((size) => size.height)));
  const totalHeight = $derived(offsets[offsets.length - 1] ?? 0);
  const contentWidth = $derived(Math.max(viewportWidth, ...displayed.slice(0, 2000).map((s) => s.width)));
  const range = $derived(visibleRange(offsets, scrollTop, viewportHeight, viewportHeight * 2));
  const windowPages = $derived(
    displayed.length === 0
      ? []
      : Array.from({ length: Math.max(0, range.end - range.start + 1) }, (_, i) => range.start + i),
  );

  // Pagination
  const spreads = $derived(buildSpreads(sizes, double));
  const spread = $derived(spreads[currentSpread] ?? []);

  async function load() {
    try {
      const opened = await openChapter(chapterId);
      sizes = opened.pages.map((page) =>
        page.width && page.height ? { width: page.width, height: page.height } : FALLBACK_SIZE,
      );
      chapter = opened;
      lastAnchor = {
        page: opened.progress?.pageIndex ?? 0,
        offset: opened.progress?.pageOffset ?? 0,
      };
      lastPercent = opened.progress?.percent ?? 0;
      currentPage = lastAnchor.page;
      await restorePosition();
    } catch (error) {
      loadError = String(error);
    }
  }

  // Le chapitre a pu être terminé : on y revient au début plutôt qu'à la fin.
  function startAnchor() {
    return chapter?.progress?.completed ? { page: 0, offset: 0 } : lastAnchor;
  }

  async function restorePosition() {
    const anchor = startAnchor();
    lastAnchor = anchor;
    currentPage = anchor.page;
    currentSpread = spreadOf(spreads, anchor.page);
    await tick();
    if (vertical) applyAnchor(anchor);
    restored = true;
    // Ouvrir un chapitre compte comme une lecture, même sans tourner de page.
    if (chapter && chapter.sourceStatus !== "missing") {
      lastPercent = vertical
        ? verticalPercent(scrollTop, viewportHeight, totalHeight)
        : pagedPercent(spreads[currentSpread], pageCount);
      scheduleSave();
    }
  }

  function applyAnchor(anchor: Anchor) {
    if (!scroller) return;
    const top = scrollTopFor(offsets, anchor);
    scroller.scrollTop = top;
    scrollTop = scroller.scrollTop;
  }

  // Taille de fenêtre : la position relative à la page est conservée.
  function onResize() {
    if (!scroller) return;
    viewportWidth = scroller.clientWidth;
    viewportHeight = scroller.clientHeight;
  }

  let appliedLayout = "";
  $effect(() => {
    if (!vertical || !chapter || viewportWidth === 0) return;
    const signature = `${viewportWidth}x${viewportHeight}/${fit}/${totalHeight}`;
    if (signature === appliedLayout) return;
    appliedLayout = signature;
    const anchor = lastAnchor;
    tick().then(() => applyAnchor(anchor));
  });

  function onScroll() {
    if (!scroller || !vertical) return;
    scrollTop = scroller.scrollTop;
    if (!restored || offsets.length < 2) return;
    lastAnchor = anchorAt(offsets, scrollTop);
    currentPage = pageAt(offsets, scrollTop + viewportHeight / 2);
    lastPercent = verticalPercent(scrollTop, viewportHeight, totalHeight);
    scheduleSave();
  }

  // Une page dont la taille n'était pas connue la révèle au chargement.
  function onImageLoad(index: number, event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    const known = sizes[index];
    if (!known || !image.naturalWidth) return;
    if (known.width === image.naturalWidth && known.height === image.naturalHeight) return;
    const next = sizes.slice();
    next[index] = { width: image.naturalWidth, height: image.naturalHeight };
    sizes = next;
    if (!vertical) currentSpread = spreadOf(buildSpreads(next, double), currentPage);
  }

  function retry(index: number) {
    failed[index] = -(Math.abs(failed[index] ?? 0) + 1);
  }

  function srcFor(index: number) {
    const attempt = Math.abs(failed[index] ?? 0);
    const url = pageUrl(chapterId, index);
    return attempt > 0 ? `${url}?retry=${attempt}` : url;
  }

  function onImageError(index: number) {
    failed[index] = Math.abs(failed[index] ?? 0) || 1;
  }

  // Le préchargement suit la page courante, sans jamais tout charger.
  $effect(() => {
    if (!chapter) return;
    const wanted = preloadOrder(currentPage, pageCount, PRELOAD_AHEAD, PRELOAD_BEHIND);
    const kept = new Map<number, HTMLImageElement>();
    for (const index of wanted) {
      let image = preloaded.get(index);
      if (!image) {
        image = new Image();
        image.decoding = "async";
        image.src = pageUrl(chapterId, index);
      }
      kept.set(index, image);
    }
    preloaded = kept;
  });

  // Progression --------------------------------------------------------------

  function scheduleSave() {
    pendingSave = true;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(flushSave, SAVE_DELAY);
  }

  function flushSave() {
    clearTimeout(saveTimer);
    if (!pendingSave || !chapter) return;
    pendingSave = false;
    saveProgress({
      chapterId: chapter.id,
      historyId: chapter.historyId,
      pageIndex: lastAnchor.page,
      pageOffset: lastAnchor.offset,
      percent: lastPercent,
    }).catch((error) => (app.error = String(error)));
  }

  function setSpread(index: number) {
    currentSpread = Math.max(0, Math.min(spreads.length - 1, index));
    const shown = spreads[currentSpread] ?? [];
    currentPage = shown[0] ?? 0;
    lastAnchor = { page: currentPage, offset: 0 };
    lastPercent = pagedPercent(shown, pageCount);
    scheduleSave();
    if (scroller) scroller.scrollTop = 0;
  }

  // Navigation ---------------------------------------------------------------

  function goToChapter(id: string | null | undefined) {
    if (!id) return;
    flushSave();
    app.go({ name: "reader", chapterId: id });
  }

  function atChapterEnd() {
    if (chapter?.nextChapterId) goToChapter(chapter.nextChapterId);
    else endMessage = "Dernier chapitre de la série";
  }

  function nextPage() {
    endMessage = null;
    if (vertical) {
      if (!scroller) return;
      if (scrollTop + viewportHeight >= totalHeight - 1) atChapterEnd();
      else scroller.scrollBy({ top: viewportHeight * 0.9 });
    } else if (currentSpread >= spreads.length - 1) {
      atChapterEnd();
    } else {
      setSpread(currentSpread + 1);
    }
  }

  function previousPage() {
    endMessage = null;
    if (vertical) {
      if (scrollTop <= 0) goToChapter(chapter?.previousChapterId);
      else scroller?.scrollBy({ top: -viewportHeight * 0.9 });
    } else if (currentSpread <= 0) {
      goToChapter(chapter?.previousChapterId);
    } else {
      setSpread(currentSpread - 1);
    }
  }

  function goToPage(page: number) {
    if (vertical) {
      lastAnchor = { page, offset: 0 };
      applyAnchor(lastAnchor);
    } else {
      setSpread(spreadOf(spreads, page));
    }
  }

  function exit() {
    flushSave();
    app.go(chapter ? { name: "series", id: chapter.seriesId } : { name: "library" });
  }

  async function toggleFullscreen() {
    const target = !fullscreen;
    await app.attempt(() => getCurrentWindow().setFullscreen(target));
    fullscreen = target;
  }

  function onKey(event: KeyboardEvent) {
    if (event.target instanceof HTMLInputElement && event.target.type !== "range") return;
    const rtl = mode === "rtl";
    switch (event.key) {
      case "ArrowRight":
        event.preventDefault();
        if (rtl) previousPage();
        else nextPage();
        break;
      case "ArrowLeft":
        event.preventDefault();
        if (rtl) nextPage();
        else previousPage();
        break;
      case "ArrowDown":
      case "PageDown":
      case " ":
        event.preventDefault();
        if (event.shiftKey) previousPage();
        else nextPage();
        break;
      case "ArrowUp":
      case "PageUp":
        event.preventDefault();
        previousPage();
        break;
      case "Home":
        event.preventDefault();
        goToPage(0);
        break;
      case "End":
        event.preventDefault();
        goToPage(pageCount - 1);
        break;
      case "f":
        toggleFullscreen();
        break;
      case "Escape":
        if (panelOpen) panelOpen = false;
        else if (fullscreen) toggleFullscreen();
        else exit();
        break;
    }
    showControls();
  }

  function onZoneClick(event: MouseEvent) {
    if (panelOpen) {
      panelOpen = false;
      return;
    }
    const third = (event.currentTarget as HTMLElement).clientWidth / 3;
    if (vertical || (event.clientX > third && event.clientX < third * 2)) {
      controlsVisible = !controlsVisible;
      if (controlsVisible) showControls();
      return;
    }
    const onRight = event.clientX >= third * 2;
    if (onRight === (mode === "rtl")) previousPage();
    else nextPage();
  }

  // Contrôles ----------------------------------------------------------------

  function showControls() {
    controlsVisible = true;
    clearTimeout(hideTimer);
    if (autoHide && !panelOpen) {
      hideTimer = setTimeout(() => (controlsVisible = false), HIDE_DELAY);
    }
  }

  function setFit(next: ReaderFit) {
    app.attempt(() => settings.update(vertical ? { readerVerticalFit: next } : { readerFit: next }));
  }

  async function changeMode(next: ReaderMode) {
    if (!chapter) return;
    const anchor = { page: currentPage, offset: vertical ? lastAnchor.offset : 0 };
    const seriesId = chapter.seriesId;
    await app.attempt(() => setSeriesDirection(seriesId, next));
    chapter.direction = next;
    lastAnchor = anchor;
    appliedLayout = "";
    currentSpread = spreadOf(spreads, anchor.page);
    await tick();
    onResize();
    if (next === "vertical") {
      await tick();
      applyAnchor(anchor);
    }
  }

  const unreadable = $derived(
    chapter?.sourceStatus === "missing" || chapter?.sourceStatus === "corrupt",
  );

  onMount(() => {
    load();
    showControls();
    const observer = new ResizeObserver(onResize);
    if (scroller) observer.observe(scroller);
    onResize();
    const onHidden = () => {
      if (document.visibilityState === "hidden") flushSave();
    };
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pagehide", flushSave);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pagehide", flushSave);
    };
  });

  onDestroy(() => {
    flushSave();
    clearTimeout(hideTimer);
    preloaded.clear();
    if (fullscreen) getCurrentWindow().setFullscreen(false).catch(() => {});
    app.changed().catch(() => {});
  });
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="reader"
  class:hide-cursor={!controlsVisible}
  style:background
  onmousemove={showControls}
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    bind:this={scroller}
    class="scroller"
    class:paged={!vertical}
    onscroll={onScroll}
    onclick={onZoneClick}
  >
    {#if vertical}
      <div class="strip" style:height="{totalHeight}px" style:width="{contentWidth}px">
        {#each windowPages as index (index)}
          {@const size = displayed[index]}
          <div
            class="strip-page"
            style:top="{offsets[index]}px"
            style:left="{(contentWidth - size.width) / 2}px"
            style:width="{size.width}px"
            style:height="{size.height}px"
          >
            {#if (failed[index] ?? 0) > 0}
              {@render pageError(index)}
            {:else}
              <img
                src={srcFor(index)}
                alt="Page {index + 1}"
                decoding="async"
                draggable="false"
                onload={(e) => onImageLoad(index, e)}
                onerror={() => onImageError(index)}
              />
            {/if}
          </div>
        {/each}
      </div>
    {:else if chapter && spread.length > 0}
      <div
        class="spread fit-{fit}"
        class:rtl={mode === "rtl"}
        class:two={spread.length === 2}
      >
        {#each spread as index (index)}
          {#if (failed[index] ?? 0) > 0}
            {@render pageError(index)}
          {:else}
            <img
              src={srcFor(index)}
              alt="Page {index + 1}"
              draggable="false"
              onload={(e) => onImageLoad(index, e)}
              onerror={() => onImageError(index)}
            />
          {/if}
        {/each}
      </div>
    {/if}
  </div>

  {#if loadError}
    <div class="message" role="alert">
      <p>{loadError}</p>
      <button class="button" onclick={exit}>Retour</button>
    </div>
  {:else if chapter && pageCount === 0}
    <div class="message"><p>Ce chapitre ne contient aucune page.</p><button class="button" onclick={exit}>Retour</button></div>
  {/if}

  {#if unreadable && controlsVisible}
    <p class="notice" role="alert">Le fichier de ce chapitre est introuvable. Rebranchez le disque ou réimportez le fichier déplacé.</p>
  {/if}
  {#if endMessage}
    <p class="notice" role="status">{endMessage}</p>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="bar top" class:gone={!controlsVisible} data-tauri-drag-region>
    <button class="ctl" onclick={exit}>Retour</button>
    <div class="titles" data-tauri-drag-region>
      {#if chapter}
        <span class="series">{chapter.seriesTitle}</span>
        <span class="chapter">{chapterLabel(chapter)}{chapter.title ? ` · ${chapter.title}` : ""}</span>
      {/if}
    </div>
    <button class="ctl" aria-expanded={panelOpen} onclick={() => (panelOpen = !panelOpen)}>Affichage</button>
    <button class="ctl" onclick={toggleFullscreen}>{fullscreen ? "Quitter le plein écran" : "Plein écran"}</button>
  </header>

  {#if panelOpen && settings.current}
    <div class="panel" role="group" aria-label="Affichage">
      <div class="row">
        <span>Mode</span>
        <div class="seg">
          <button class:on={mode === "vertical"} onclick={() => changeMode("vertical")}>Vertical</button>
          <button class:on={mode === "ltr"} onclick={() => changeMode("ltr")}>Gauche à droite</button>
          <button class:on={mode === "rtl"} onclick={() => changeMode("rtl")}>Droite à gauche</button>
        </div>
      </div>
      <div class="row">
        <span>Ajustement</span>
        <div class="seg">
          <button class:on={fit === "width"} onclick={() => setFit("width")}>Largeur</button>
          <button class:on={fit === "height"} onclick={() => setFit("height")}>Hauteur</button>
          <button class:on={fit === "original"} onclick={() => setFit("original")}>Originale</button>
        </div>
      </div>
      <label class="row" class:muted={vertical}>
        <span>Double page</span>
        <input
          type="checkbox"
          checked={double}
          disabled={vertical}
          onchange={(e) => {
            const page = currentPage;
            settings.update({ readerDoublePage: e.currentTarget.checked }).then(() => {
              currentSpread = spreadOf(spreads, page);
            });
          }}
        />
      </label>
      <label class="row">
        <span>Fond</span>
        <input
          type="color"
          value={background}
          onchange={(e) => settings.update({ readerBackground: e.currentTarget.value })}
        />
      </label>
      <label class="row">
        <span>Masquer les contrôles</span>
        <input
          type="checkbox"
          checked={autoHide}
          onchange={(e) => settings.update({ readerAutoHide: e.currentTarget.checked })}
        />
      </label>
      <p class="hint">Le mode est retenu pour cette série.</p>
    </div>
  {/if}

  <footer class="bar bottom" class:gone={!controlsVisible}>
    <button class="ctl" disabled={!chapter?.previousChapterId} onclick={() => goToChapter(chapter?.previousChapterId)}>Chapitre précédent</button>
    <input
      type="range"
      class:flip={mode === "rtl"}
      min="0"
      max={Math.max(0, pageCount - 1)}
      value={currentPage}
      disabled={pageCount <= 1}
      aria-label="Page"
      oninput={(e) => goToPage(Number(e.currentTarget.value))}
    />
    <span class="count">{pageCount > 0 ? currentPage + 1 : 0} / {pageCount}</span>
    <button class="ctl" disabled={!chapter?.nextChapterId} onclick={() => goToChapter(chapter?.nextChapterId)}>Chapitre suivant</button>
  </footer>
</div>

{#snippet pageError(index: number)}
  <div class="page-error">
    <p>Page {index + 1} illisible</p>
    <button
      class="ctl"
      onclick={(e) => {
        e.stopPropagation();
        retry(index);
      }}
    >
      Réessayer
    </button>
  </div>
{/snippet}

<style>
  .reader {
    position: fixed;
    inset: 0;
    overflow: hidden;
    color: #eee;
    color-scheme: dark;
  }

  .hide-cursor {
    cursor: none;
  }

  .scroller {
    position: absolute;
    inset: 0;
    overflow: auto;
    overscroll-behavior: contain;
  }

  .strip {
    position: relative;
    min-width: 100%;
  }

  .strip-page {
    position: absolute;
  }

  .strip-page img {
    display: block;
    width: 100%;
    height: 100%;
  }

  .scroller.paged {
    display: flex;
  }

  .spread {
    display: flex;
    flex: none;
    justify-content: center;
    min-width: 100%;
    min-height: 100%;
    margin: auto;
  }

  .spread.rtl {
    flex-direction: row-reverse;
  }

  .spread img {
    display: block;
    flex: none;
    align-self: center;
  }

  .fit-height {
    height: 100vh;
  }

  .fit-height img {
    max-width: 100vw;
    height: 100vh;
    object-fit: contain;
  }

  .fit-height.two img {
    max-width: 50vw;
  }

  .fit-width img {
    width: 100vw;
    height: auto;
  }

  .fit-width.two img {
    width: 50vw;
  }

  .fit-width {
    align-items: flex-start;
  }

  .page-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 100%;
    min-width: 240px;
    height: 100%;
    min-height: 200px;
    align-self: center;
    color: #aaa;
  }

  .page-error p {
    margin: 0;
  }

  .message {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    user-select: text;
    -webkit-user-select: text;
  }

  .message .button {
    color: #eee;
  }

  .notice {
    position: absolute;
    left: 50%;
    bottom: 64px;
    margin: 0;
    padding: 6px 12px;
    border-radius: 6px;
    background: rgb(20 20 20 / 0.9);
    transform: translateX(-50%);
  }

  .bar {
    position: absolute;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    background: rgb(20 20 20 / 0.88);
    -webkit-backdrop-filter: blur(16px);
    backdrop-filter: blur(16px);
    transition:
      transform 160ms ease,
      opacity 160ms ease;
  }

  .top {
    top: 0;
    padding-left: var(--window-controls-width);
  }

  .bottom {
    bottom: 0;
  }

  .top.gone {
    opacity: 0;
    pointer-events: none;
    transform: translateY(-100%);
  }

  .bottom.gone {
    opacity: 0;
    pointer-events: none;
    transform: translateY(100%);
  }

  .titles {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }

  .series,
  .chapter {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .series {
    font-weight: 600;
  }

  .chapter {
    color: #aaa;
    font-size: 12px;
  }

  .ctl {
    flex: none;
    padding: 4px 10px;
    border-radius: 6px;
    color: #eee;
  }

  .ctl:hover:not(:disabled) {
    background: rgb(255 255 255 / 0.12);
  }

  .ctl:disabled {
    opacity: 0.35;
  }

  input[type="range"] {
    flex: 1;
    min-width: 60px;
    accent-color: var(--accent);
  }

  input[type="range"].flip {
    direction: rtl;
  }

  .count {
    flex: none;
    min-width: 66px;
    color: #bbb;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .panel {
    position: absolute;
    top: 52px;
    right: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(390px, calc(100vw - 28px));
    padding: 14px;
    border-radius: 10px;
    background: rgb(28 28 28 / 0.97);
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.5);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-height: 26px;
  }

  .row.muted {
    opacity: 0.45;
  }

  .seg {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: rgb(255 255 255 / 0.08);
  }

  .seg button {
    padding: 3px 8px;
    border-radius: 5px;
    color: #bbb;
    font-size: 12px;
  }

  .seg button.on {
    background: rgb(255 255 255 / 0.18);
    color: #fff;
  }

  input[type="color"] {
    width: 34px;
    height: 22px;
    padding: 0;
    border: 1px solid rgb(255 255 255 / 0.3);
    border-radius: 4px;
    background: none;
  }

  .hint {
    margin: 0;
    color: #999;
    font-size: 12px;
  }

  @media (max-width: 820px) {
    .bottom .ctl {
      display: none;
    }
  }
</style>
