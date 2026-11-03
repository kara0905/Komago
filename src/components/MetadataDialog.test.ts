import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test } from "vitest";
import type { MetadataPreview, SearchOutcome, Settings } from "../lib/api";
import { settings } from "../lib/settings.svelte";
import { backend, calls } from "../test/setup";
import MetadataDialog from "./MetadataDialog.svelte";

const outcome: SearchOutcome = {
  token: "t",
  failures: [{ providerName: "AniList", message: "limite de requêtes atteinte" }],
  matches: [
    match(0, "Vinland Saga", 1, true),
    match(1, "Animalland Saga", 0.67, false),
  ],
};

function match(index: number, title: string, score: number, certain: boolean) {
  return {
    index,
    providerId: "mangadex",
    providerName: "MangaDex",
    score,
    certain,
    hasCover: false,
    series: { externalId: `id${index}`, url: null, title, description: null, authors: [], artists: [], kind: null, status: null, publishedFrom: null },
  };
}

const preview: MetadataPreview = {
  token: "t",
  index: 0,
  mode: "assisted",
  providerName: "MangaDex",
  changes: [
    { field: "description", current: "Ma description", proposed: "Vikings", kind: "conflict", currentSource: "Modification manuelle", locked: true, selected: false },
    { field: "status", current: "", proposed: "completed", kind: "fill", currentSource: null, locked: false, selected: true },
    { field: "title", current: "Vinland Saga", proposed: "Vinland Saga", kind: "same", currentSource: null, locked: false, selected: false },
  ],
};

function useNetwork(enabled: boolean) {
  settings.current = { networkEnabled: enabled } as Settings;
}

function open() {
  render(MetadataDialog, { seriesId: "s", seriesTitle: "Vinland Saga", onclose: () => {}, onapplied: () => {} });
}

describe("identification d'une série", () => {
  beforeEach(() => {
    backend({
      search_metadata: () => outcome,
      preview_metadata: () => preview,
      apply_metadata: () => ({ applied: ["status"], coverFailed: false }),
    });
  });

  test("hors ligne, rien n'est envoyé avant l'accord de l'utilisateur", async () => {
    useNetwork(false);
    open();
    expect(await screen.findByRole("button", { name: "Autoriser les connexions" })).toBeTruthy();
    expect(calls.some((c) => c.cmd === "search_metadata")).toBe(false);
  });

  test("signale les correspondances sûres et les échecs de fournisseur", async () => {
    useNetwork(true);
    open();
    expect(await screen.findByText(/Très probable/)).toBeTruthy();
    expect(screen.getByText(/À vérifier · 67 %/)).toBeTruthy();
    expect(screen.getByText(/AniList : limite de requêtes atteinte/)).toBeTruthy();
  });

  test("un champ protégé reste décoché et n'est pas envoyé", async () => {
    const user = userEvent.setup();
    useNetwork(true);
    open();
    await user.click((await screen.findAllByRole("button", { name: "Choisir" }))[0]);
    expect(await screen.findByText(/Protégé : modifié à la main/)).toBeTruthy();
    expect(screen.queryByText("Titre"), "un champ identique n'est pas proposé").toBeNull();

    await user.click(screen.getByRole("button", { name: "Appliquer 1 champ" }));
    const apply = calls.find((c) => c.cmd === "apply_metadata");
    expect(apply?.args.fields).toEqual(["status"]);
    expect(await screen.findByText(/Champs mis à jour : Statut/)).toBeTruthy();
  });
});
