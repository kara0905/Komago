import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import type { ImportPlan } from "../lib/api";
import { backend, calls } from "../test/setup";
import ImportView from "./ImportView.svelte";

function plan(): ImportPlan {
  return {
    sessionId: "session",
    relocated: 0,
    issues: [{ path: "/x/evil.cbz", message: "archive refusée : chemin dangereux" }],
    series: [
      {
        key: "ma serie",
        title: "Ma série",
        existingSeriesId: null,
        coverChapterId: "c1",
        chapters: [
          chapter("c1", "Chapter 001", 1, false),
          chapter("c2", "Chapter 002", 2, false),
          chapter("c3", "Chapter 003", 3, true),
        ],
      },
    ],
  };
}

function chapter(id: string, name: string, number: number, duplicate: boolean) {
  return {
    id,
    displayName: name,
    sourcePath: `/x/${name}.cbz`,
    volume: 1,
    number,
    title: null,
    pageCount: 20,
    duplicate,
    hasComicInfo: false,
  };
}

function setup(fromRoots = false) {
  backend({
    analyze_import: () => plan(),
    commit_import: () => ({ seriesCreated: 1, chaptersAdded: 1, relocated: 0, coversFailed: 0, firstSeriesId: "s" }),
    list_collections: () => [],
    scan_roots: () => [],
  });
  render(ImportView, { paths: ["/x"], fromRoots });
}

describe("plan d'import", () => {
  test("montre séries, chapitres et éléments signalés", async () => {
    setup();
    expect(await screen.findByDisplayValue("Ma série")).toBeTruthy();
    expect(screen.getByText("1 élément ignoré ou signalé")).toBeTruthy();
    expect(screen.getByText("Déjà importé")).toBeTruthy();
    const duplicate = screen.getByLabelText("Inclure Chapter 003") as HTMLInputElement;
    expect(duplicate.disabled).toBe(true);
    expect(duplicate.checked).toBe(false);
  });

  test("envoie seulement des corrections, jamais de chemins", async () => {
    const user = userEvent.setup();
    setup(true);
    const title = await screen.findByDisplayValue("Ma série");
    await user.clear(title);
    await user.type(title, "Bon titre");
    await user.click(screen.getByLabelText("Inclure Chapter 002"));
    await user.click(screen.getByRole("button", { name: "Importer 1 chapitre" }));

    const commit = calls.find((c) => c.cmd === "commit_import");
    const edits = commit?.args.edits as {
      series: { title: string }[];
      chapters: { id: string; include: boolean }[];
      ignoreExcluded: boolean;
    };
    expect(edits.series[0].title).toBe("Bon titre");
    expect(edits.chapters.map((c) => [c.id, c.include])).toEqual([
      ["c1", true],
      ["c2", false],
      ["c3", false],
    ]);
    expect(edits.ignoreExcluded).toBe(true);
    expect(JSON.stringify(commit?.args)).not.toContain("/x/");
    expect(await screen.findByText(/1 chapitre ajouté/)).toBeTruthy();
  });

  test("un numéro invalide bloque la validation", async () => {
    const user = userEvent.setup();
    setup();
    await screen.findByDisplayValue("Ma série");
    const numbers = screen.getAllByLabelText("Numéro");
    await user.clear(numbers[0]);
    await user.type(numbers[0], "abc");
    const button = screen.getByRole("button", { name: /Importer 2 chapitres/ }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });
});
