// Calculs purs du lecteur, sans DOM : ils se testent et restent rapides même sur
// un chapitre de plusieurs milliers de pages.

export type Fit = "width" | "height" | "original";

export interface PageSize {
  width: number;
  height: number;
}

export const FALLBACK_SIZE: PageSize = { width: 2, height: 3 };

/** Taille affichée d'une page dans une fenêtre donnée. */
export function displaySize(page: PageSize, fit: Fit, viewportWidth: number, viewportHeight: number): PageSize {
  switch (fit) {
    case "width":
      return { width: viewportWidth, height: (viewportWidth * page.height) / page.width };
    case "height":
      return { width: (viewportHeight * page.width) / page.height, height: viewportHeight };
    case "original":
      return page;
  }
}

/** Position du haut de chaque page ; la dernière valeur est la hauteur totale. */
export function buildOffsets(heights: number[], gap = 0): number[] {
  const offsets = new Array<number>(heights.length + 1);
  offsets[0] = 0;
  for (let i = 0; i < heights.length; i += 1) {
    offsets[i + 1] = offsets[i] + heights[i] + (i < heights.length - 1 ? gap : 0);
  }
  return offsets;
}

/** Page située à une position de défilement. */
export function pageAt(offsets: number[], scrollTop: number): number {
  let low = 0;
  let high = offsets.length - 2;
  if (high < 0) return 0;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (offsets[middle] <= scrollTop) low = middle;
    else high = middle - 1;
  }
  return low;
}

export interface Anchor {
  page: number;
  /** Fraction de la page déjà défilée, indépendante de la taille de la fenêtre. */
  offset: number;
}

export function anchorAt(offsets: number[], scrollTop: number): Anchor {
  const page = pageAt(offsets, scrollTop);
  const height = (offsets[page + 1] ?? 0) - (offsets[page] ?? 0);
  if (height <= 0) return { page, offset: 0 };
  return { page, offset: Math.min(1, Math.max(0, (scrollTop - offsets[page]) / height)) };
}

export function scrollTopFor(offsets: number[], anchor: Anchor): number {
  const page = Math.min(anchor.page, Math.max(0, offsets.length - 2));
  const height = (offsets[page + 1] ?? 0) - (offsets[page] ?? 0);
  return (offsets[page] ?? 0) + anchor.offset * height;
}

/** Pages à garder dans le DOM : ce qui est visible et une marge de chaque côté. */
export function visibleRange(offsets: number[], scrollTop: number, viewportHeight: number, margin: number) {
  if (offsets.length < 2) return { start: 0, end: -1 };
  return {
    start: pageAt(offsets, Math.max(0, scrollTop - margin)),
    end: pageAt(offsets, scrollTop + viewportHeight + margin),
  };
}

export function verticalPercent(scrollTop: number, viewportHeight: number, total: number) {
  if (total <= 0) return 0;
  return Math.min(100, ((scrollTop + viewportHeight) / total) * 100);
}

/**
 * Regroupe les pages en doubles pages : la première reste seule, comme une
 * couverture, et une page déjà large occupe seule son écran. Une page isolée
 * avant une page large reste aussi seule.
 */
export function buildSpreads(pages: PageSize[], double: boolean, offset = false): number[][] {
  if (!double) return pages.map((_, index) => [index]);
  const spreads: number[][] = [];
  let index = 0;
  while (index < pages.length) {
    const next = index + 1;
    // Décalé, la première page s'apparie aussi : utile pour un tome sans couverture.
    const pairable =
      (index > 0 || offset) && next < pages.length && !isWide(pages[index]) && !isWide(pages[next]);
    if (pairable) {
      spreads.push([index, next]);
      index += 2;
    } else {
      spreads.push([index]);
      index += 1;
    }
  }
  return spreads;
}

function isWide(page: PageSize) {
  return page.width > page.height;
}

export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 5;

export function clampZoom(zoom: number) {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(zoom * 100) / 100));
}

export function spreadOf(spreads: number[][], page: number) {
  const found = spreads.findIndex((spread) => spread.includes(page));
  return found === -1 ? 0 : found;
}

export function pagedPercent(spread: number[] | undefined, pageCount: number) {
  if (!spread || pageCount === 0) return 0;
  return ((spread[spread.length - 1] + 1) / pageCount) * 100;
}

/** Pages à précharger, les suivantes d'abord, dans le sens de lecture. */
export function preloadOrder(current: number, pageCount: number, ahead: number, behind: number) {
  const order: number[] = [];
  for (let i = 1; i <= ahead; i += 1) if (current + i < pageCount) order.push(current + i);
  for (let i = 1; i <= behind; i += 1) if (current - i >= 0) order.push(current - i);
  return order;
}
