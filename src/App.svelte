<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import Modal from "./components/Modal.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { checkSources, createCollection, startupFailure } from "./lib/api";
  import { app } from "./lib/app.svelte";
  import { settings } from "./lib/settings.svelte";
  import IdentifyView from "./views/IdentifyView.svelte";
  import ImportView from "./views/ImportView.svelte";
  import ReaderView from "./views/ReaderView.svelte";
  import LibraryView from "./views/LibraryView.svelte";
  import SeriesView from "./views/SeriesView.svelte";
  import SettingsView from "./views/SettingsView.svelte";
  import komagoLogo from "./assets/komago-logo.png";

  let ready = $state(false);
  let fatal = $state<string | null>(null);
  let dropping = $state(false);
  let creating = $state(false);
  let newName = $state("");
  let searchInput = $state<HTMLInputElement>();

  const system = $derived(app.collections.filter((c) => c.systemKey));
  const personal = $derived(app.collections.filter((c) => !c.systemKey));
  const activeCollection = $derived(
    app.route.name === "library" ? (app.route.collectionId ?? null) : null,
  );

  onMount(() => {
    start();

    const unlisten = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (app.route.name === "reader") return;
      if (payload.type === "enter" || payload.type === "over") {
        dropping = true;
      } else if (payload.type === "leave") {
        dropping = false;
      } else if (payload.type === "drop") {
        dropping = false;
        if (payload.paths.length > 0) app.go({ name: "import", paths: payload.paths });
      }
    });

    const onKey = (event: KeyboardEvent) => {
      if (app.route.name !== "reader" && (event.metaKey || event.ctrlKey) && event.key === "f") {
        event.preventDefault();
        searchInput?.focus();
        searchInput?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unlisten.then((stop) => stop());
      window.removeEventListener("keydown", onKey);
    };
  });

  async function start() {
    try {
      fatal = await startupFailure();
      if (fatal) return;
      await Promise.all([settings.load(), app.loadCollections()]);
      ready = true;
    } catch (error) {
      fatal = String(error);
      return;
    }
    // Un disque débranché ne doit pas être bloquant : le résultat arrive après l'affichage.
    checkSources()
      .then((missing) => (missing > 0 ? app.changed() : undefined))
      .catch(() => {});
    app.scanRoots().catch(() => {});
  }

  function onSearch() {
    if (app.route.name !== "library") app.go({ name: "library" });
  }

  async function submitCollection(event: SubmitEvent) {
    event.preventDefault();
    const id = await app.attempt(() => createCollection(newName));
    creating = false;
    newName = "";
    if (id) {
      await app.changed();
      app.go({ name: "library", collectionId: id });
    }
  }
</script>

{#if fatal}
  <div class="failure" data-tauri-drag-region>
    <h1>Komago ne peut pas démarrer</h1>
    <p role="alert">{fatal}</p>
    <p class="hint">Vos données n'ont pas été modifiées.</p>
    <button class="button" onclick={() => getCurrentWindow().close()}>Quitter</button>
  </div>
{:else if !ready}
  <!-- Chargement : quelques millisecondes, sans contenu à montrer. -->
{:else if app.route.name === "reader"}
  {#key app.route.chapterId}
    <ReaderView chapterId={app.route.chapterId} />
  {/key}
{:else}
<div class="shell">
  <aside class="sidebar">
    <div class="drag" data-tauri-drag-region></div>
    <div class="brand">
      <img class="brand-mark" src={komagoLogo} alt="" />
      <span class="brand-copy"><strong>Komago</strong></span>
    </div>
    <div class="search">
      <input
        bind:this={searchInput}
        type="search"
        placeholder="Rechercher"
        aria-label="Rechercher dans la bibliothèque"
        bind:value={app.search}
        oninput={onSearch}
      />
    </div>
    <nav aria-label="Navigation principale">
      <button
        class="nav-item"
        class:active={app.route.name === "library" && !activeCollection}
        aria-current={app.route.name === "library" && !activeCollection ? "page" : undefined}
        onclick={() => app.go({ name: "library" })}
      >
        Bibliothèque
      </button>
      <button
        class="nav-item"
        class:active={app.route.name === "import"}
        aria-current={app.route.name === "import" ? "page" : undefined}
        onclick={() => app.go({ name: "import", paths: [] })}
      >
        Importer
      </button>
      {#if app.newInRoots.length > 0}
        <button
          class="nav-item row news"
          onclick={() => app.go({ name: "import", paths: app.newInRoots, fromRoots: true })}
        >
          <span class="name">Nouveautés</span>
          <span class="count">{app.newInRoots.length}</span>
        </button>
      {/if}
      <button
        class="nav-item"
        class:active={app.route.name === "identify"}
        aria-current={app.route.name === "identify" ? "page" : undefined}
        onclick={() => app.go({ name: "identify" })}
      >
        Identification
      </button>

      <h2>Collections</h2>
      {#each system as collection (collection.id)}
        {@render collectionLink(collection)}
      {/each}
      {#if personal.length > 0}<div class="gap"></div>{/if}
      {#each personal as collection (collection.id)}
        {@render collectionLink(collection)}
      {/each}
      <button class="nav-item" onclick={() => (creating = true)}>Nouvelle collection</button>
    </nav>
    <div class="bottom">
      <div class="local-status"><span></span>Bibliothèque locale</div>
      <button
        class="nav-item"
        class:active={app.route.name === "settings"}
        aria-current={app.route.name === "settings" ? "page" : undefined}
        onclick={() => app.go({ name: "settings" })}
      >
        Paramètres
      </button>
    </div>
  </aside>

  <main class="content">
    <div class="drag" data-tauri-drag-region></div>
    {#if app.error}
      <div class="banner" role="alert">
        <span>{app.error}</span>
        <button onclick={() => (app.error = null)}>Fermer</button>
      </div>
    {/if}
    {#if app.route.name === "library"}
      {#key app.route.collectionId}
        <LibraryView collectionId={app.route.collectionId ?? null} />
      {/key}
    {:else if app.route.name === "series"}
      {#key app.route.id}
        <SeriesView id={app.route.id} identify={app.route.identify ?? false} />
      {/key}
    {:else if app.route.name === "import"}
      {#key app.route.paths}
        <ImportView paths={app.route.paths} fromRoots={app.route.fromRoots ?? false} />
      {/key}
    {:else if app.route.name === "identify"}
      <IdentifyView />
    {:else}
      <SettingsView />
    {/if}
  </main>

  {#if dropping}
    <div class="drop-overlay"><p>Déposez pour analyser ces fichiers</p></div>
  {/if}
</div>
{/if}

{#snippet collectionLink(collection: (typeof app.collections)[number])}
  <button
    class="nav-item row"
    class:active={activeCollection === collection.id}
    aria-current={activeCollection === collection.id ? "page" : undefined}
    onclick={() => app.go({ name: "library", collectionId: collection.id })}
  >
    <span class="name">{collection.name}</span>
    {#if collection.itemCount > 0}<span class="count">{collection.itemCount}</span>{/if}
  </button>
{/snippet}

{#if creating}
  <Modal title="Nouvelle collection" onclose={() => (creating = false)}>
    <form id="new-collection" onsubmit={submitCollection}>
      <input class="field" bind:value={newName} placeholder="Nom" aria-label="Nom de la collection" required />
    </form>
    {#snippet actions()}
      <button class="button" onclick={() => (creating = false)}>Annuler</button>
      <button class="button primary" type="submit" form="new-collection" disabled={!newName.trim()}>Créer</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: 216px minmax(0, 1fr);
    height: 100vh;
  }

  .drag {
    height: var(--titlebar-height);
    flex: none;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-sidebar);
    border-right: 1px solid var(--border);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 58px;
    padding: 2px 18px 12px;
  }

  .brand-mark {
    display: block;
    width: 34px;
    height: 34px;
    border-radius: 10px;
    object-fit: cover;
    box-shadow: 0 2px 12px rgb(16 12 20 / 0.16);
  }

  .brand-copy { display: flex; flex-direction: column; line-height: 1.1; }
  .brand-copy strong { font-size: 15px; letter-spacing: -0.02em; }

  .search {
    padding: 0 10px 8px;
  }

  .search input {
    width: 100%;
    padding: 5px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--hover);
    color: var(--text);
    font: inherit;
    outline-offset: 0;
    user-select: text;
    -webkit-user-select: text;
  }

  nav {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    overflow-y: auto;
    padding: 0 10px 8px;
  }

  h2 {
    margin: 16px 10px 4px;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }

  .gap {
    height: 8px;
    flex: none;
  }

  .bottom {
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }

  .local-status {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 8px;
    padding: 9px 10px;
    border: 1px solid rgb(67 177 138 / 0.2);
    border-radius: 9px;
    background: rgb(67 177 138 / 0.08);
    color: #4eaa87;
    font-size: 11px;
    font-weight: 600;
  }

  .local-status span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #62caa1;
    box-shadow: 0 0 9px rgb(98 202 161 / 0.35);
  }

  .nav-item {
    display: block;
    flex: none;
    width: 100%;
    padding: 5px 10px;
    border-radius: 6px;
    color: var(--text-muted);
    text-align: left;
  }

  .nav-item.row {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }

  .nav-item:hover {
    background: var(--hover);
  }

  .nav-item.active {
    background: var(--selected);
    color: var(--accent);
    font-weight: 550;
  }

  .nav-item.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--accent);
  }

  .nav-item { position: relative; transition: background 120ms ease, color 120ms ease; }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .news .name {
    color: var(--accent);
    font-weight: 600;
  }

  .count {
    flex: none;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 400;
    font-variant-numeric: tabular-nums;
  }

  .content {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
  }

  .banner {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    margin: 0 32px 12px;
    padding: 8px 12px;
    border-radius: 6px;
    background: var(--hover);
    color: var(--danger);
    user-select: text;
    -webkit-user-select: text;
  }

  .banner button {
    flex: none;
    color: var(--text);
  }

  .failure {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    height: 100vh;
    padding: 32px;
    text-align: center;
  }

  .failure h1 {
    margin: 0;
    font-size: 18px;
  }

  .failure p {
    max-width: 52ch;
    margin: 0;
    user-select: text;
    -webkit-user-select: text;
  }

  .failure .hint {
    color: var(--text-muted);
  }

  .drop-overlay {
    position: fixed;
    inset: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: 12px;
    background: color-mix(in srgb, var(--surface) 85%, transparent);
    font-size: 15px;
    font-weight: 600;
    pointer-events: none;
  }

  @media (max-width: 860px) {
    .shell {
      grid-template-columns: 172px minmax(0, 1fr);
    }
  }
</style>
