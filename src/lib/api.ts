import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";

export type Theme = "system" | "light" | "dark";
export type LibraryLayout = "grid" | "list";

export type MetadataMode = "automatic" | "assisted" | "manual";
export type ReaderMode = "vertical" | "ltr" | "rtl";
export type ReaderFit = "width" | "height" | "original";
export type ReaderPageSpacing = "none" | "normal" | "large";

export interface Settings {
  theme: Theme;
  networkEnabled: boolean;
  libraryView: LibraryLayout;
  readerMode: ReaderMode;
  readerFit: ReaderFit;
  readerVerticalFit: ReaderFit;
  readerDoublePage: boolean;
  readerBackground: string;
  readerAutoHide: boolean;
  readerPageSpacing: ReaderPageSpacing;
  readerPageWidth: number;
  readerBrightness: number;
  readerContrast: number;
  readerSaturation: number;
  readerGrayscale: boolean;
  readerNightMode: boolean;
  readerAutoScrollSpeed: number;
  readerKeyboardScrollStep: number;
  metadataMode: MetadataMode;
  disabledProviders: string[];
}

export interface StorageInfo {
  dataDir: string;
  cacheDir: string;
  databaseBytes: number;
  coversBytes: number;
  libraryBytes: number;
  cacheBytes: number;
}

export type SeriesSort = "title" | "added" | "updated" | "chapters" | "read";

export interface SeriesFilter {
  query?: string | null;
  kind?: string | null;
  status?: string | null;
  collectionId?: string | null;
  sort?: SeriesSort;
}

export interface SeriesCard {
  id: string;
  title: string;
  kind: string;
  status: string;
  coverAssetId: string | null;
  volumeCount: number;
  chapterCount: number;
  missingCount: number;
  readCount: number;
}

export interface VolumeRow {
  id: string;
  number: number | null;
  title: string | null;
  description: string | null;
  publishedOn: string | null;
}

export interface ChapterRow {
  id: string;
  seriesId: string;
  seriesTitle: string;
  volumeId: string | null;
  number: number | null;
  title: string | null;
  pageCount: number;
  language: string | null;
  publishedOn: string | null;
  sourcePath: string | null;
  sourceStatus: string | null;
  percent: number | null;
  completed: boolean;
}

export interface SeriesPatch {
  title: string;
  kind: string;
  status: string;
  description: string | null;
  publisher: string | null;
  publishedFrom: string | null;
  publishedTo: string | null;
  originalLanguage: string | null;
  altTitles: string[];
  authors: string[];
  artists: string[];
  genres: string[];
  tags: string[];
}

export interface SeriesDetail extends SeriesPatch {
  id: string;
  coverAssetId: string | null;
  collectionIds: string[];
  volumes: VolumeRow[];
  chapters: ChapterRow[];
  resumeChapterId: string | null;
  hasProgress: boolean;
  metadataMode: MetadataMode | null;
  externalLinks: ExternalLink[];
  fieldSources: FieldSource[];
}

export interface ExternalLink {
  sourceId: string;
  sourceName: string;
  externalId: string;
  url: string | null;
}

export interface FieldSource {
  field: string;
  sourceName: string;
  locked: boolean;
}

export interface ProviderSeries {
  externalId: string;
  url: string | null;
  title: string;
  description: string | null;
  authors: string[];
  artists: string[];
  kind: string | null;
  status: string | null;
  publishedFrom: string | null;
}

export interface Match {
  index: number;
  providerId: string;
  providerName: string;
  series: ProviderSeries;
  score: number;
  certain: boolean;
  hasCover: boolean;
}

export interface ProviderFailure {
  providerName: string;
  message: string;
}

export interface SearchOutcome {
  token: string;
  matches: Match[];
  failures: ProviderFailure[];
}

export interface FieldChange {
  field: string;
  current: string;
  proposed: string;
  kind: "fill" | "conflict" | "same";
  currentSource: string | null;
  locked: boolean;
  selected: boolean;
}

export interface MetadataPreview {
  token: string;
  index: number;
  mode: MetadataMode;
  providerName: string;
  changes: FieldChange[];
}

export interface ApplyOutcome {
  applied: string[];
  coverFailed: boolean;
}

export interface ChapterPatch {
  number: number | null;
  title: string | null;
  volumeId: string | null;
  seriesId: string;
}

export interface VolumePatch {
  number: number | null;
  title: string | null;
  description: string | null;
  publishedOn: string | null;
}

export interface Collection {
  id: string;
  systemKey: string | null;
  name: string;
  itemCount: number;
}

export interface PlanChapter {
  id: string;
  displayName: string;
  sourcePath: string;
  volume: number | null;
  number: number | null;
  title: string | null;
  pageCount: number;
  duplicate: boolean;
  hasComicInfo: boolean;
}

export interface PlanSeries {
  key: string;
  title: string;
  existingSeriesId: string | null;
  coverChapterId: string | null;
  chapters: PlanChapter[];
}

export interface PlanIssue {
  path: string;
  message: string;
}

export interface ImportPlan {
  sessionId: string;
  series: PlanSeries[];
  issues: PlanIssue[];
  relocated: number;
}

export interface ImportEdits {
  series: { key: string; title: string; existingSeriesId: string | null }[];
  chapters: {
    id: string;
    seriesKey: string;
    volume: number | null;
    number: number | null;
    title: string | null;
    include: boolean;
  }[];
  copyFiles: boolean;
  ignoreExcluded?: boolean;
}

export interface ImportResult {
  seriesCreated: number;
  chaptersAdded: number;
  relocated: number;
  coversFailed: number;
  firstSeriesId: string | null;
}

export interface AnalyzeProgress {
  done: number;
  total: number;
}

export interface ReaderChapter {
  id: string;
  seriesId: string;
  seriesTitle: string;
  direction: string | null;
  number: number | null;
  title: string | null;
  sourceStatus: string | null;
  pages: { width: number | null; height: number | null }[];
  progress: { pageIndex: number; pageOffset: number; percent: number; completed: boolean } | null;
  spreadOffset: boolean;
  previousChapterId: string | null;
  nextChapterId: string | null;
  historyId: string;
}

export interface ProgressUpdate {
  chapterId: string;
  historyId: string | null;
  pageIndex: number;
  pageOffset: number;
  percent: number;
}

export const startupFailure = () => invoke<string | null>("startup_failure");
export const getSettings = () => invoke<Settings>("get_settings");
export const updateSettings = (settings: Settings) =>
  invoke<Settings>("update_settings", { settings });
export const getStorageInfo = () => invoke<StorageInfo>("get_storage_info");
export const clearCache = () => invoke<StorageInfo>("clear_cache");

export const pickImportFiles = () => invoke<string[]>("pick_import_files");
export const pickImportFolder = () => invoke<string[]>("pick_import_folder");
export function analyzeImport(paths: string[], onProgress: (progress: AnalyzeProgress) => void) {
  const channel = new Channel<AnalyzeProgress>();
  channel.onmessage = onProgress;
  return invoke<ImportPlan>("analyze_import", { paths, onProgress: channel });
}
export const commitImport = (sessionId: string, edits: ImportEdits) =>
  invoke<ImportResult>("commit_import", { sessionId, edits });
export const cancelImport = () => invoke<void>("cancel_import");
export const checkSources = () => invoke<number>("check_sources");

export const listSeries = (filter: SeriesFilter) => invoke<SeriesCard[]>("list_series", { filter });
export const searchChapters = (query: string) => invoke<ChapterRow[]>("search_chapters", { query });
export const getSeries = (id: string) => invoke<SeriesDetail>("get_series", { id });
export const updateSeries = (id: string, patch: SeriesPatch) =>
  invoke<SeriesDetail>("update_series", { id, patch });
export const deleteSeries = (id: string) => invoke<void>("delete_series", { id });
export const setCoverFromChapter = (seriesId: string, chapterId: string) =>
  invoke<void>("set_cover_from_chapter", { seriesId, chapterId });
export const updateChapter = (id: string, patch: ChapterPatch) =>
  invoke<void>("update_chapter", { id, patch });
export const deleteChapter = (id: string) => invoke<void>("delete_chapter", { id });
export const createVolume = (seriesId: string, patch: VolumePatch) =>
  invoke<string>("create_volume", { seriesId, patch });
export const updateVolume = (id: string, patch: VolumePatch) =>
  invoke<void>("update_volume", { id, patch });
export const deleteVolume = (id: string) => invoke<void>("delete_volume", { id });

export const listCollections = () => invoke<Collection[]>("list_collections");
export const createCollection = (name: string) => invoke<string>("create_collection", { name });
export const renameCollection = (id: string, name: string) =>
  invoke<void>("rename_collection", { id, name });
export const deleteCollection = (id: string) => invoke<void>("delete_collection", { id });
export const addSeriesToCollection = (collectionId: string, seriesId: string) =>
  invoke<void>("add_series_to_collection", { collectionId, seriesId });
export const removeSeriesFromCollection = (collectionId: string, seriesId: string) =>
  invoke<void>("remove_series_from_collection", { collectionId, seriesId });
export const addChapterToCollection = (collectionId: string, chapterId: string) =>
  invoke<void>("add_chapter_to_collection", { collectionId, chapterId });
export const removeChapterFromCollection = (collectionId: string, chapterId: string) =>
  invoke<void>("remove_chapter_from_collection", { collectionId, chapterId });
export const collectionChapters = (collectionId: string) =>
  invoke<ChapterRow[]>("collection_chapters", { collectionId });

export const openChapter = (id: string) => invoke<ReaderChapter>("open_chapter", { id });
export const saveProgress = (update: ProgressUpdate) => invoke<void>("save_progress", { update });
export const setChapterRead = (id: string, read: boolean) =>
  invoke<void>("set_chapter_read", { id, read });
export const setSeriesDirection = (seriesId: string, direction: ReaderMode) =>
  invoke<void>("set_series_direction", { seriesId, direction });

export const listMetadataProviders = () =>
  invoke<{ id: string; name: string }[]>("list_metadata_providers");
export const searchMetadata = (seriesId: string, title: string | null) =>
  invoke<SearchOutcome>("search_metadata", { seriesId, title });
export const previewMetadata = (token: string, index: number) =>
  invoke<MetadataPreview>("preview_metadata", { token, index });
export const refreshMetadata = (seriesId: string, providerId: string) =>
  invoke<MetadataPreview>("refresh_metadata", { seriesId, providerId });
export const applyMetadata = (token: string, index: number, fields: string[] | null) =>
  invoke<ApplyOutcome>("apply_metadata", { token, index, fields });
export const rejectMetadata = (token: string, index: number) =>
  invoke<void>("reject_metadata", { token, index });
export const unlinkMetadata = (seriesId: string, sourceId: string) =>
  invoke<void>("unlink_metadata", { seriesId, sourceId });
export const setSeriesMetadataMode = (id: string, mode: MetadataMode | null) =>
  invoke<void>("set_series_metadata_mode", { id, mode });
export const candidateCoverUrl = (token: string, index: number) =>
  convertFileSrc(`candidate/${token}/${index}`, "komago");

export const openSeriesLink = (seriesId: string, sourceId: string) =>
  invoke<void>("open_series_link", { seriesId, sourceId });
export const revealDataFolder = () => invoke<void>("reveal_data_folder");
export const appInfo = () => invoke<{ version: string; databaseVersion: number }>("app_info");

export const pageUrl = (chapterId: string, index: number) =>
  convertFileSrc(`page/${chapterId}/${index}`, "komago");
export const coverUrl = (assetId: string) => convertFileSrc(`cover/${assetId}`, "komago");
export const previewUrl = (sessionId: string, chapterId: string) =>
  convertFileSrc(`preview/${sessionId}/${chapterId}`, "komago");

// Dossiers suivis ---------------------------------------------------------------

export interface LibraryRoot {
  id: string;
  path: string;
  status: "available" | "missing";
  sourceCount: number;
  lastScannedAt: string | null;
}

export interface RootScan {
  rootId: string;
  path: string;
  available: boolean;
  newPaths: string[];
}

export const listRoots = () => invoke<{ roots: LibraryRoot[]; ignored: number }>("list_roots");
export const addRoot = (path: string) => invoke<string>("add_root", { path });
export const removeRoot = (id: string) => invoke<void>("remove_root", { id });
export const scanRoots = () => invoke<RootScan[]>("scan_roots");
export const clearIgnoredSources = () => invoke<number>("clear_ignored_sources");

// Lecture -----------------------------------------------------------------------

export interface ContinueItem {
  chapter: ChapterRow;
  coverAssetId: string | null;
  lastReadAt: string;
}

export const continueReading = () => invoke<ContinueItem[]>("continue_reading");
export const setSpreadOffset = (seriesId: string, offset: boolean) =>
  invoke<void>("set_spread_offset", { seriesId, offset });

// Identification groupée --------------------------------------------------------

export interface BatchEntry {
  seriesId: string;
  title: string;
  outcome: "applied" | "uncertain" | "none" | "failed";
  providerName: string | null;
  matchedTitle: string | null;
  message: string | null;
}

export const listUnidentified = () => invoke<SeriesCard[]>("list_unidentified");
export function identifyBatch(seriesIds: string[], onProgress: (p: { done: number; total: number }) => void) {
  const channel = new Channel<{ done: number; total: number }>();
  channel.onmessage = onProgress;
  return invoke<BatchEntry[]>("identify_batch", { seriesIds, onProgress: channel });
}
export const cancelIdentifyBatch = () => invoke<void>("cancel_identify_batch");
export const fillChapterTitles = (seriesId: string, providerId: string) =>
  invoke<number>("fill_chapter_titles", { seriesId, providerId });

// Sauvegarde --------------------------------------------------------------------

export interface StagedRestore {
  manifest: { appVersion: string; databaseVersion: number; createdAt: string; includesLibrary: boolean };
  seriesCount: number;
  chapterCount: number;
}

export const exportBackup = (includeLibrary: boolean) =>
  invoke<string | null>("export_backup", { includeLibrary });
export const stageRestore = () => invoke<StagedRestore | null>("stage_restore");
export const discardRestore = () => invoke<void>("discard_restore");
export const restartApp = () => invoke<void>("restart_app");
