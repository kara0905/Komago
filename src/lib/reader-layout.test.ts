import { describe, expect, test } from "vitest";
import {
  anchorAt,
  buildOffsets,
  buildSpreads,
  clampZoom,
  displaySize,
  pageAt,
  pagedPercent,
  preloadOrder,
  scrollTopFor,
  spreadOf,
  verticalPercent,
  visibleRange,
} from "./reader-layout";

const tall = { width: 100, height: 150 };
const wide = { width: 300, height: 150 };

describe("tailles", () => {
  test("ajustements", () => {
    expect(displaySize(tall, "width", 400, 600)).toEqual({ width: 400, height: 600 });
    expect(displaySize(tall, "height", 400, 300)).toEqual({ width: 200, height: 300 });
    expect(displaySize(tall, "original", 400, 300)).toEqual(tall);
  });
});

describe("défilement", () => {
  const offsets = buildOffsets([100, 200, 300]);

  test("positions cumulées", () => {
    expect(offsets).toEqual([0, 100, 300, 600]);
  });

  test("page à une position", () => {
    expect(pageAt(offsets, 0)).toBe(0);
    expect(pageAt(offsets, 99)).toBe(0);
    expect(pageAt(offsets, 100)).toBe(1);
    expect(pageAt(offsets, 599)).toBe(2);
    expect(pageAt(offsets, 5000)).toBe(2);
    expect(pageAt([0], 0)).toBe(0);
  });

  test("la position survit à un changement de taille de fenêtre", () => {
    const anchor = anchorAt(offsets, 200);
    expect(anchor).toEqual({ page: 1, offset: 0.5 });
    const resized = buildOffsets([200, 400, 600]);
    expect(scrollTopFor(resized, anchor)).toBe(400);
  });

  test("une ancre hors limites reste dans le chapitre", () => {
    expect(scrollTopFor(offsets, { page: 50, offset: 0 })).toBe(300);
    expect(scrollTopFor([0], { page: 3, offset: 0.5 })).toBe(0);
  });

  test("seules les pages proches restent en mémoire", () => {
    const many = buildOffsets(new Array(5000).fill(1000));
    const range = visibleRange(many, 1_000_000, 800, 1600);
    expect(range.end - range.start).toBeLessThanOrEqual(4);
    expect(range.start).toBeGreaterThanOrEqual(998);
    expect(visibleRange([0], 0, 800, 100)).toEqual({ start: 0, end: -1 });
  });

  test("pourcentage", () => {
    expect(verticalPercent(0, 100, 1000)).toBe(10);
    expect(verticalPercent(900, 100, 1000)).toBe(100);
    expect(verticalPercent(0, 100, 0)).toBe(0);
  });
});

describe("doubles pages", () => {
  test("page simple", () => {
    expect(buildSpreads([tall, tall], false)).toEqual([[0], [1]]);
  });

  test("la couverture reste seule et les suivantes vont par deux", () => {
    expect(buildSpreads([tall, tall, tall, tall, tall], true)).toEqual([[0], [1, 2], [3, 4]]);
  });

  test("le décalage apparie aussi la première page", () => {
    expect(buildSpreads([tall, tall, tall, tall], true, true)).toEqual([[0, 1], [2, 3]]);
    expect(buildSpreads([tall, wide, tall], true, true)).toEqual([[0], [1], [2]]);
  });

  test("une page large reste seule", () => {
    expect(buildSpreads([tall, tall, wide, tall, tall], true)).toEqual([[0], [1], [2], [3, 4]]);
  });

  test("retrouver une page et calculer la progression", () => {
    const spreads = buildSpreads([tall, tall, tall], true);
    expect(spreadOf(spreads, 2)).toBe(1);
    expect(spreadOf(spreads, 99)).toBe(0);
    expect(pagedPercent(spreads[1], 3)).toBe(100);
    expect(pagedPercent(undefined, 3)).toBe(0);
  });
});

test("zoom borné et arrondi", () => {
  expect(clampZoom(10)).toBe(5);
  expect(clampZoom(0.01)).toBe(0.25);
  expect(clampZoom(1.234)).toBe(1.23);
});

test("préchargement borné aux limites du chapitre", () => {
  expect(preloadOrder(5, 7, 3, 1)).toEqual([6, 4]);
  expect(preloadOrder(0, 10, 2, 2)).toEqual([1, 2]);
  expect(preloadOrder(0, 0, 2, 2)).toEqual([]);
});
