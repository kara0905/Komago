<script lang="ts">
  import { onMount } from "svelte";
  import {
    clearCache,
    appInfo,
    getStorageInfo,
    revealDataFolder,
    listMetadataProviders,
    type MetadataMode,
    type ReaderFit,
    type ReaderMode,
    type StorageInfo,
    type Theme,
  } from "../lib/api";
  import BackupSettings from "../components/BackupSettings.svelte";
  import RootsSettings from "../components/RootsSettings.svelte";
  import { settings } from "../lib/settings.svelte";

  const themes: { value: Theme; label: string }[] = [
    { value: "system", label: "Système" },
    { value: "light", label: "Clair" },
    { value: "dark", label: "Sombre" },
  ];

  let providers = $state<{ id: string; name: string }[]>([]);
  let info = $state<{ version: string; databaseVersion: number } | null>(null);
  let storage = $state<StorageInfo | null>(null);
  let clearing = $state(false);
  let error = $state<string | null>(null);

  onMount(() => {
    run(async () => {
      storage = await getStorageInfo();
      providers = await listMetadataProviders();
      info = await appInfo();
    });
  });

  async function run(action: () => Promise<unknown>) {
    error = null;
    try {
      await action();
    } catch (e) {
      error = String(e);
    }
  }

  async function emptyCache() {
    clearing = true;
    await run(async () => {
      storage = await clearCache();
    });
    clearing = false;
  }

  function toggleProvider(id: string, enabled: boolean) {
    const others = (settings.current?.disabledProviders ?? []).filter((p) => p !== id);
    run(() => settings.update({ disabledProviders: enabled ? others : [...others, id] }));
  }

  function formatBytes(bytes: number) {
    if (bytes < 1024) return `${bytes} o`;
    const units = ["Ko", "Mo", "Go", "To"];
    let value = bytes;
    let unit = -1;
    do {
      value /= 1024;
      unit += 1;
    } while (value >= 1024 && unit < units.length - 1);
    return `${value.toLocaleString("fr-FR", { maximumFractionDigits: 1 })} ${units[unit]}`;
  }
</script>

<div class="page">
  <h1>Paramètres</h1>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if settings.current}
    <section>
      <h2><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.42 1.42m11.3 11.3 1.42 1.42M2 12h2m16 0h2M4.93 19.07l1.42-1.42m11.3-11.3 1.42-1.42"/></svg>Apparence</h2>
      <div class="row">
        <span id="theme-label">Thème</span>
        <div class="segmented" role="radiogroup" aria-labelledby="theme-label">
          {#each themes as theme (theme.value)}
            <button
              role="radio"
              aria-checked={settings.current.theme === theme.value}
              class:active={settings.current.theme === theme.value}
              onclick={() => run(() => settings.update({ theme: theme.value }))}
            >
              {theme.label}
            </button>
          {/each}
        </div>
      </div>
    </section>

    <section>
      <h2><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 5.5c3.6-.8 6.6-.2 9 2v12c-2.4-2.2-5.4-2.8-9-2V5.5Zm18 0c-3.6-.8-6.6-.2-9 2v12c2.4-2.2 5.4-2.8 9-2V5.5Z"/></svg>Lecteur</h2>
      <label class="row">
        <span>Mode par défaut</span>
        <select
          value={settings.current.readerMode}
          onchange={(e) => run(() => settings.update({ readerMode: e.currentTarget.value as ReaderMode }))}
        >
          <option value="ltr">Gauche à droite</option>
          <option value="rtl">Droite à gauche</option>
          <option value="vertical">Défilement vertical</option>
        </select>
      </label>
      <label class="row">
        <span>Ajustement en pagination</span>
        <select
          value={settings.current.readerFit}
          onchange={(e) => run(() => settings.update({ readerFit: e.currentTarget.value as ReaderFit }))}
        >
          <option value="height">Hauteur</option>
          <option value="width">Largeur</option>
          <option value="original">Taille originale</option>
        </select>
      </label>
      <label class="row">
        <span>Ajustement en défilement vertical</span>
        <select
          value={settings.current.readerVerticalFit}
          onchange={(e) => run(() => settings.update({ readerVerticalFit: e.currentTarget.value as ReaderFit }))}
        >
          <option value="width">Largeur</option>
          <option value="height">Hauteur</option>
          <option value="original">Taille originale</option>
        </select>
      </label>
      <label class="range-row">
        <span>Largeur des planches <output>{settings.current.readerPageWidth}%</output></span>
        <input type="range" min="40" max="100" step="5" value={settings.current.readerPageWidth}
          onchange={(e) => run(() => settings.update({ readerPageWidth: Number(e.currentTarget.value) }))} />
      </label>
      <label class="row">
        <span>Espacement entre les pages</span>
        <select
          value={settings.current.readerPageSpacing}
          onchange={(e) => run(() => settings.update({ readerPageSpacing: e.currentTarget.value as "none" | "normal" | "large" }))}
        >
          <option value="none">Aucun</option>
          <option value="normal">Normal</option>
          <option value="large">Large</option>
        </select>
      </label>
      <label class="range-row">
        <span>Luminosité des pages <output>{settings.current.readerBrightness}%</output></span>
        <input type="range" min="50" max="150" step="5" value={settings.current.readerBrightness}
          onchange={(e) => run(() => settings.update({ readerBrightness: Number(e.currentTarget.value) }))} />
      </label>
      <label class="range-row">
        <span>Contraste <output>{settings.current.readerContrast}%</output></span>
        <input type="range" min="50" max="150" step="5" value={settings.current.readerContrast}
          onchange={(e) => run(() => settings.update({ readerContrast: Number(e.currentTarget.value) }))} />
      </label>
      <label class="range-row">
        <span>Saturation <output>{settings.current.readerSaturation}%</output></span>
        <input type="range" min="0" max="200" step="10" value={settings.current.readerSaturation}
          onchange={(e) => run(() => settings.update({ readerSaturation: Number(e.currentTarget.value) }))} />
      </label>
      <label class="row">
        <span>Niveaux de gris</span>
        <input type="checkbox" role="switch" checked={settings.current.readerGrayscale}
          onchange={(e) => run(() => settings.update({ readerGrayscale: e.currentTarget.checked }))} />
      </label>
      <label class="row">
        <span>Teinte chaude pour la lecture nocturne</span>
        <input type="checkbox" role="switch" checked={settings.current.readerNightMode}
          onchange={(e) => run(() => settings.update({ readerNightMode: e.currentTarget.checked }))} />
      </label>
      <label class="range-row">
        <span>Vitesse du défilement automatique <output>{settings.current.readerAutoScrollSpeed} px/s</output></span>
        <input type="range" min="20" max="400" step="20" value={settings.current.readerAutoScrollSpeed}
          onchange={(e) => run(() => settings.update({ readerAutoScrollSpeed: Number(e.currentTarget.value) }))} />
      </label>
      <label class="range-row">
        <span>Pas de défilement au clavier <output>{settings.current.readerKeyboardScrollStep} px</output></span>
        <input type="range" min="40" max="1000" step="40" value={settings.current.readerKeyboardScrollStep}
          onchange={(e) => run(() => settings.update({ readerKeyboardScrollStep: Number(e.currentTarget.value) }))} />
      </label>
      <label class="row">
        <span>Double page</span>
        <input
          type="checkbox"
          role="switch"
          checked={settings.current.readerDoublePage}
          onchange={(e) => run(() => settings.update({ readerDoublePage: e.currentTarget.checked }))}
        />
      </label>
      <label class="row">
        <span>Masquer les contrôles automatiquement</span>
        <input
          type="checkbox"
          role="switch"
          checked={settings.current.readerAutoHide}
          onchange={(e) => run(() => settings.update({ readerAutoHide: e.currentTarget.checked }))}
        />
      </label>
      <label class="row">
        <span>Couleur de fond</span>
        <input
          type="color"
          value={settings.current.readerBackground}
          onchange={(e) => run(() => settings.update({ readerBackground: e.currentTarget.value }))}
        />
      </label>
    </section>

    <section>
      <h2><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M3.5 9h17M3.5 15h17M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18"/></svg>Réseau</h2>
      <label class="row">
        <span class="label">
          Autoriser les connexions externes
          <small>
            Uniquement quand vous identifiez ou actualisez une série : son titre est envoyé
            aux services activés. Komago reste entièrement utilisable sans.
          </small>
        </span>
        <input
          type="checkbox"
          role="switch"
          checked={settings.current.networkEnabled}
          onchange={(e) =>
            run(() => settings.update({ networkEnabled: e.currentTarget.checked }))}
        />
      </label>
      <label class="row">
        <span>Mise à jour des informations</span>
        <select
          value={settings.current.metadataMode}
          onchange={(e) => run(() => settings.update({ metadataMode: e.currentTarget.value as MetadataMode }))}
        >
          <option value="automatic">Automatique : compléter les champs vides</option>
          <option value="assisted">Assisté : proposer avant de modifier</option>
          <option value="manual">Manuel : ne rien modifier de lui-même</option>
        </select>
      </label>
      {#each providers as provider (provider.id)}
        <label class="row">
          <span>Utiliser {provider.name}</span>
          <input
            type="checkbox"
            role="switch"
            checked={!settings.current.disabledProviders.includes(provider.id)}
            onchange={(e) => toggleProvider(provider.id, e.currentTarget.checked)}
          />
        </label>
      {/each}
      <p class="credits">
        Informations et couvertures fournies par MangaDex (mangadex.org) et AniList (anilist.co).
        Komago n'est affilié à aucun de ces services.
      </p>
    </section>
  {/if}

  <RootsSettings />
  <BackupSettings />

  {#if storage}
    <section>
      <h2><svg viewBox="0 0 24 24" aria-hidden="true"><ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"/></svg>Stockage</h2>
      <dl>
        <div class="row"><dt>Base de données</dt><dd>{formatBytes(storage.databaseBytes)}</dd></div>
        <div class="row"><dt>Couvertures</dt><dd>{formatBytes(storage.coversBytes)}</dd></div>
        <div class="row"><dt>Fichiers copiés</dt><dd>{formatBytes(storage.libraryBytes)}</dd></div>
        <div class="row">
          <dt>
            Cache
            <small>Fichiers temporaires, recréés à la demande.</small>
          </dt>
          <dd>
            {formatBytes(storage.cacheBytes)}
            <button class="button" disabled={clearing || storage.cacheBytes === 0} onclick={emptyCache}>
              Vider
            </button>
          </dd>
        </div>
        <div class="row path">
          <dt>Données</dt>
          <dd>
            {storage.dataDir}
            <button class="button" onclick={() => run(revealDataFolder)}>Afficher</button>
          </dd>
        </div>
        <div class="row path"><dt>Cache</dt><dd>{storage.cacheDir}</dd></div>
      </dl>
    </section>
  {/if}

  {#if info}
    <section>
      <h2><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 11v5m0-8h.01"/></svg>À propos</h2>
      <div class="row"><span>Version de Komago</span><span class="muted">{info.version}</span></div>
      <div class="row"><span>Version de la base</span><span class="muted">{info.databaseVersion}</span></div>
      <p class="credits">
        Komago fonctionne sans compte et n'envoie aucune télémétrie. Aucune connexion n'est établie tant
        que vous ne l'autorisez pas.
      </p>
    </section>
  {/if}
</div>

<style>
  .page {
    width: 100%;
    max-width: 680px;
    margin: 0 auto;
    padding: 0 32px 40px;
  }

  h1 {
    margin: 0 0 20px;
    font-size: 22px;
    font-weight: 650;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
  }

  h2 svg { width: 16px; height: 16px; fill: none; stroke: var(--accent); stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }

  section {
    margin-bottom: 28px;
  }

  dl {
    margin: 0;
  }

  dt,
  .label {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  dd {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  small {
    max-width: 44ch;
    font-size: 12px;
    color: var(--text-muted);
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

  .range-row {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 9px 0;
    border-bottom: 1px solid var(--border);
  }

  .range-row span {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }

  .range-row output {
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .range-row input { width: 100%; accent-color: var(--accent); }

  .path dd {
    min-width: 0;
    overflow-wrap: anywhere;
    text-align: right;
    font-size: 12px;
    user-select: text;
  }

  .path dt {
    flex: none;
  }

  input[role="switch"] {
    flex: none;
    width: 34px;
    height: 20px;
    margin: 0;
    appearance: none;
    position: relative;
    border-radius: 10px;
    background: var(--border-strong);
    transition: background 120ms;
  }

  input[role="switch"]::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    transition: transform 120ms;
  }

  input[role="switch"]:checked {
    background: var(--accent);
  }

  input[role="switch"]:checked::after {
    transform: translateX(14px);
  }

  .muted {
    color: var(--text-muted);
  }

  .credits {
    margin: 8px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  select {
    max-width: 280px;
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  .error {
    color: var(--danger);
    user-select: text;
  }
</style>
