import { render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { Settings } from "../lib/api";
import { app } from "../lib/app.svelte";
import { settings } from "../lib/settings.svelte";
import { backend } from "../test/setup";
import LibraryView from "./LibraryView.svelte";

const chapter = {
  id: "c2", seriesId: "s", seriesTitle: "Berserk", volumeId: null, number: 2, title: null, pageCount: 20,
  language: null, publishedOn: null, sourcePath: null, sourceStatus: "available", percent: 40, completed: false,
};

test("l'accueil propose de reprendre la lecture", async () => {
  settings.current = { libraryView: "grid" } as Settings;
  backend({
    list_series: () => [],
    continue_reading: () => [{ chapter, coverAssetId: null, lastReadAt: "2026-11-01" }],
  });
  render(LibraryView, { collectionId: null });
  expect(await screen.findByText("Continuer la lecture")).toBeTruthy();
  expect(screen.getByText(/Chapitre 2 · 40 %/)).toBeTruthy();
});

test("une collection ne montre pas la reprise", async () => {
  settings.current = { libraryView: "grid" } as Settings;
  backend({
    list_series: () => [],
    collection_chapters: () => [],
    continue_reading: () => [{ chapter, coverAssetId: null, lastReadAt: "2026-11-01" }],
  });
  app.collections = [{ id: "system-favorites", systemKey: "favorites", name: "Favoris", itemCount: 0 }];
  render(LibraryView, { collectionId: "system-favorites" });
  expect(await screen.findByText("Cette collection est vide")).toBeTruthy();
  expect(screen.queryByText("Continuer la lecture")).toBeNull();
});
