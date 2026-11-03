import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import type { Settings } from "../lib/api";
import { settings } from "../lib/settings.svelte";
import { backend, calls } from "../test/setup";
import IdentifyView from "./IdentifyView.svelte";

const card = (id: string, title: string) => ({
  id, title, kind: "unknown", status: "unknown", coverAssetId: null, volumeCount: 0, chapterCount: 3, missingCount: 0, readCount: 0,
});

test("l'identification groupée distingue associées et à vérifier", async () => {
  const user = userEvent.setup();
  settings.current = { networkEnabled: true } as Settings;
  let remaining = [card("a", "Vinland Saga"), card("b", "Monster"), card("c", "Écartée")];
  backend({
    list_unidentified: () => remaining,
    identify_batch: () => {
      remaining = [card("b", "Monster")];
      return [
        { seriesId: "a", title: "Vinland Saga", outcome: "applied", providerName: "MangaDex", matchedTitle: "Vinland Saga", message: null },
        { seriesId: "b", title: "Monster", outcome: "uncertain", providerName: "AniList", matchedTitle: "Monster", message: null },
      ];
    },
    list_collections: () => [],
  });
  render(IdentifyView);

  await user.click(await screen.findByLabelText("Inclure Écartée"));
  await user.click(screen.getByRole("button", { name: "Identifier 2 séries" }));

  expect(calls.find((c) => c.cmd === "identify_batch")?.args.seriesIds).toEqual(["a", "b"]);
  expect(await screen.findByText("Associée")).toBeTruthy();
  expect(screen.getByText("À vérifier")).toBeTruthy();
  expect(await screen.findByText("1 série non identifiée")).toBeTruthy();
});
