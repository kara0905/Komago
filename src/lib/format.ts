import type { ChapterRow, VolumeRow } from "./api";

export const KINDS: Record<string, string> = {
  unknown: "Non défini",
  manga: "Manga",
  manhwa: "Manhwa",
  manhua: "Manhua",
  webcomic: "Webcomic",
  comic: "Comics",
  other: "Autre",
};

export const STATUSES: Record<string, string> = {
  unknown: "Inconnu",
  ongoing: "En cours de publication",
  completed: "Publication terminée",
  hiatus: "En pause",
  cancelled: "Arrêtée",
};

export function formatNumber(value: number) {
  return value.toLocaleString("fr-FR", { maximumFractionDigits: 3 });
}

/** Accepte la virgule ; `undefined` signale une saisie invalide. */
export function parseNumber(text: string): number | null | undefined {
  const trimmed = text.trim().replace(",", ".");
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value >= 0 ? value : undefined;
}

export function chapterLabel(chapter: Pick<ChapterRow, "number">) {
  return chapter.number === null ? "Sans numéro" : `Chapitre ${formatNumber(chapter.number)}`;
}

export function volumeLabel(volume: Pick<VolumeRow, "number" | "title">) {
  const number = volume.number === null ? "Volume" : `Volume ${formatNumber(volume.number)}`;
  return volume.title ? `${number} · ${volume.title}` : number;
}

export function plural(count: number, one: string, many = `${one}s`) {
  return `${count.toLocaleString("fr-FR")} ${count > 1 ? many : one}`;
}

export function splitList(text: string) {
  return text
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}
