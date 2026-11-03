<script lang="ts">
  import type { ReaderFit, ReaderMode, ReaderPageSpacing } from "../lib/api";
  import { settings } from "../lib/settings.svelte";

  let {
    mode,
    fit,
    double,
    vertical,
    spreadOffset,
    zoom,
    onmode,
    onfit,
    ondouble,
    onoffset,
    onzoom,
  }: {
    mode: ReaderMode;
    fit: ReaderFit;
    double: boolean;
    vertical: boolean;
    spreadOffset: boolean;
    zoom: number;
    onmode: (mode: ReaderMode) => void;
    onfit: (fit: ReaderFit) => void;
    ondouble: (value: boolean) => void;
    onoffset: (value: boolean) => void;
    onzoom: (zoom: number) => void;
  } = $props();

  const current = $derived(settings.current!);
  const pageWidthPercent = $derived(current.readerPageWidth);
  const background = $derived(current.readerBackground);
  const autoHide = $derived(current.readerAutoHide);
  const brightness = $derived(current.readerBrightness);
  const contrast = $derived(current.readerContrast);
  const saturation = $derived(current.readerSaturation);
  const grayscale = $derived(current.readerGrayscale);
  const nightMode = $derived(current.readerNightMode);
  const autoScrollSpeed = $derived(current.readerAutoScrollSpeed);
  const keyboardScrollStep = $derived(current.readerKeyboardScrollStep);
</script>

  <div class="panel" role="group" aria-label="Affichage">
    <section class="panel-section">
      <h3><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 5.5c3.6-.8 6.6-.2 9 2v12c-2.4-2.2-5.4-2.8-9-2V5.5Zm18 0c-3.6-.8-6.6-.2-9 2v12c2.4-2.2 5.4-2.8 9-2V5.5Z"/></svg>Mode de lecture</h3>
      <label class="row">
        <span>Mode par défaut</span>
        <select value={current.readerMode}
          onchange={(e) => settings.update({ readerMode: e.currentTarget.value as ReaderMode })}>
          <option value="vertical">Défilement vertical</option>
          <option value="ltr">Gauche à droite</option>
          <option value="rtl">Droite à gauche</option>
        </select>
      </label>
      <div class="row">
        <span>Pour cette série</span>
        <div class="seg">
          <button class:on={mode === "vertical"} onclick={() => onmode("vertical")}>Continu</button>
          <button class:on={mode === "ltr"} onclick={() => onmode("ltr")}>← à →</button>
          <button class:on={mode === "rtl"} onclick={() => onmode("rtl")}>→ à ←</button>
        </div>
      </div>
      <div class="row">
        <span>Ajustement {vertical ? "continu" : "pagination"}</span>
        <div class="seg">
          <button class:on={fit === "width"} onclick={() => onfit("width")}>Largeur</button>
          <button class:on={fit === "height"} onclick={() => onfit("height")}>Hauteur</button>
          <button class:on={fit === "original"} onclick={() => onfit("original")}>Originale</button>
        </div>
      </div>
      <label class="row" class:muted={vertical}>
        <span>Double page</span>
        <input type="checkbox" checked={double} disabled={vertical}
          onchange={(e) => ondouble(e.currentTarget.checked)} />
      </label>
      {#if !vertical && double}
        <label class="row">
          <span>Décaler d'une page <small>Pour un tome sans couverture.</small></span>
          <input type="checkbox" checked={spreadOffset} onchange={(e) => onoffset(e.currentTarget.checked)} />
        </label>
      {/if}
      <div class="row">
        <span>Zoom <small>Ctrl + molette, pincement, touches + − 0</small></span>
        <div class="seg">
          <button aria-label="Dézoomer" onclick={() => onzoom(zoom / 1.25)}>−</button>
          <button class:on={zoom !== 1} onclick={() => onzoom(1)}>{Math.round(zoom * 100)} %</button>
          <button aria-label="Zoomer" onclick={() => onzoom(zoom * 1.25)}>+</button>
        </div>
      </div>
      {#if vertical}
        <label class="row">
          <span>Espacement des pages</span>
          <select value={current.readerPageSpacing}
            onchange={(e) => settings.update({ readerPageSpacing: e.currentTarget.value as ReaderPageSpacing })}>
            <option value="none">Aucun</option><option value="normal">Normal</option><option value="large">Large</option>
          </select>
        </label>
      {/if}
    </section>

    <section class="panel-section">
      <h3><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/></svg>Affichage</h3>
      <label class="range-row"><span>Largeur des planches <output>{pageWidthPercent}%</output></span><input type="range" min="40" max="100" step="5" value={pageWidthPercent}
        onchange={(e) => settings.update({ readerPageWidth: Number(e.currentTarget.value) })} /></label>
      <label class="row"><span>Fond</span><input type="color" value={background}
        onchange={(e) => settings.update({ readerBackground: e.currentTarget.value })} /></label>
      <label class="row"><span>Masquer les contrôles automatiquement</span><input type="checkbox" checked={autoHide}
        onchange={(e) => settings.update({ readerAutoHide: e.currentTarget.checked })} /></label>
    </section>

    <section class="panel-section">
      <h3><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16"/><circle cx="9" cy="6" r="2"/><circle cx="15" cy="12" r="2"/><circle cx="11" cy="18" r="2"/></svg>Correction d’image</h3>
      <label class="range-row"><span>Luminosité <output>{brightness}%</output></span><input type="range" min="50" max="150" step="5" value={brightness}
        onchange={(e) => settings.update({ readerBrightness: Number(e.currentTarget.value) })} /></label>
      <label class="range-row"><span>Contraste <output>{contrast}%</output></span><input type="range" min="50" max="150" step="5" value={contrast}
        onchange={(e) => settings.update({ readerContrast: Number(e.currentTarget.value) })} /></label>
      <label class="range-row"><span>Saturation <output>{saturation}%</output></span><input type="range" min="0" max="200" step="10" value={saturation}
        onchange={(e) => settings.update({ readerSaturation: Number(e.currentTarget.value) })} /></label>
      <label class="row"><span>Niveaux de gris</span><input type="checkbox" checked={grayscale}
        onchange={(e) => settings.update({ readerGrayscale: e.currentTarget.checked })} /></label>
      <label class="row"><span>Teinte chaude nocturne</span><input type="checkbox" checked={nightMode}
        onchange={(e) => settings.update({ readerNightMode: e.currentTarget.checked })} /></label>
    </section>

    {#if vertical}
      <section class="panel-section">
        <h3><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v18m-6-6 6 6 6-6M6 5h12"/></svg>Navigation continue</h3>
        <label class="range-row"><span>Vitesse auto <output>{autoScrollSpeed} px/s</output></span><input type="range" min="20" max="400" step="20" value={autoScrollSpeed}
          onchange={(e) => settings.update({ readerAutoScrollSpeed: Number(e.currentTarget.value) })} /></label>
        <label class="range-row"><span>Pas clavier <output>{keyboardScrollStep} px</output></span><input type="range" min="40" max="1000" step="40" value={keyboardScrollStep}
          onchange={(e) => settings.update({ readerKeyboardScrollStep: Number(e.currentTarget.value) })} /></label>
      </section>
    {/if}
    <p class="hint">Le sens de lecture est retenu pour cette série ; les autres réglages sont globaux.</p>
  </div>

<style>
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

  .range-row { display: flex; flex-direction: column; gap: 4px; }

  .range-row > span { display: flex; justify-content: space-between; color: #ccc; font-size: 12px; }

  .range-row output { color: var(--accent); font-variant-numeric: tabular-nums; }

  .range-row input { width: 100%; accent-color: var(--accent); }

  .panel { max-height: calc(100vh - 76px); overflow-y: auto; }

  .panel-section { display: flex; flex-direction: column; gap: 9px; padding-bottom: 12px; border-bottom: 1px solid rgb(255 255 255 / 0.1); }

  .panel-section:last-of-type { border-bottom: 0; }

  .panel-section h3 { display: flex; align-items: center; gap: 8px; margin: 0 0 2px; color: var(--accent); font-size: 12px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; }

  .panel-section h3 svg { width: 17px; height: 17px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }

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

  small {
    display: block;
    color: #999;
    font-size: 11px;
  }

  .panel {
    color: #eee;
  }
</style>
