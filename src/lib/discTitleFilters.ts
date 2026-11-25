import type { DiscTitle } from "./types";

/** Standard TV episode runtime window (20m – 65m). */
export const EPISODE_MIN_SECONDS = 20 * 60;
export const EPISODE_MAX_SECONDS = 65 * 60;

/** Minimum title duration (5m) to flag as a duplicate-duration playlist candidate. */
export const DUPLICATE_MIN_DURATION_SECONDS = 5 * 60;

/** Tolerance (±2s) for clustering duplicate-duration playlists on obfuscated Blu-rays. */
export const DUPLICATE_DURATION_TOLERANCE_SECONDS = 2;

export interface DuplicateDurationGroup {
  groupId: number;
  titleIds: number[];
  canonicalDurationSeconds: number;
  count: number;
  isFeatureLength: boolean;
}

export interface DiscTitleAnalysis {
  mainFeatureTitleId: number | null;
  episodeTitleIds: number[];
  duplicateGroups: DuplicateDurationGroup[];
  duplicateByTitleId: Record<number, DuplicateDurationGroup>;
  duplicateTitleCount: number;
  hasObfuscatedPattern: boolean;
}

function isPreferredMainPlaylist(sourceFileName: string | null): boolean {
  if (!sourceFileName) return false;
  return /^00800\.mpls$/i.test(sourceFileName.trim());
}

function compareTitlePriority(a: DiscTitle, b: DiscTitle): number {
  const durDiff = (b.durationSeconds ?? 0) - (a.durationSeconds ?? 0);
  if (Math.abs(durDiff) > DUPLICATE_DURATION_TOLERANCE_SECONDS) {
    return durDiff;
  }
  // Within duplicate duration tolerance, prefer canonical 00800.mpls if present,
  // then largest size, then highest chapter count, then longest exact duration, then lowest ID.
  const aPref = isPreferredMainPlaylist(a.sourceFileName) ? 1 : 0;
  const bPref = isPreferredMainPlaylist(b.sourceFileName) ? 1 : 0;
  if (bPref !== aPref) return bPref - aPref;

  const sizeDiff = (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0);
  if (sizeDiff !== 0) return sizeDiff;

  const chapDiff = (b.chapterCount ?? 0) - (a.chapterCount ?? 0);
  if (chapDiff !== 0) return chapDiff;

  if (durDiff !== 0) return durDiff;
  return a.titleId - b.titleId;
}

/**
 * Analyzes MakeMKV scanned disc titles to identify:
 * - Main Feature (longest title, breaking ties intelligently)
 * - TV Episode candidates (20–65 minutes)
 * - Duplicate-duration clusters (common on ScreenPass / playlist-obfuscated Blu-rays)
 */
export function analyzeDiscTitles(titles: DiscTitle[]): DiscTitleAnalysis {
  if (titles.length === 0) {
    return {
      mainFeatureTitleId: null,
      episodeTitleIds: [],
      duplicateGroups: [],
      duplicateByTitleId: {},
      duplicateTitleCount: 0,
      hasObfuscatedPattern: false,
    };
  }

  // 1. Main feature (longest title with smart tie-breaking)
  const sortedForMain = [...titles].sort(compareTitlePriority);
  const mainFeatureTitleId = sortedForMain[0]?.titleId ?? null;

  // 2. TV Episodes (20–65 minutes)
  const episodeTitleIds = titles
    .filter((t) => {
      const dur = t.durationSeconds;
      return (
        dur !== null &&
        dur >= EPISODE_MIN_SECONDS &&
        dur <= EPISODE_MAX_SECONDS
      );
    })
    .map((t) => t.titleId);

  // 3. Duplicate-duration detection (cluster durations >= 5m within ±2s)
  const eligible = titles
    .filter(
      (t): t is DiscTitle & { durationSeconds: number } =>
        t.durationSeconds !== null &&
        t.durationSeconds >= DUPLICATE_MIN_DURATION_SECONDS,
    )
    .sort((a, b) => a.durationSeconds - b.durationSeconds);

  const duplicateGroups: DuplicateDurationGroup[] = [];
  const duplicateByTitleId: Record<number, DuplicateDurationGroup> = {};
  let nextGroupId = 1;

  let cluster: (DiscTitle & { durationSeconds: number })[] = [];
  const flushCluster = () => {
    if (cluster.length >= 2) {
      const canonical = cluster[0].durationSeconds;
      const group: DuplicateDurationGroup = {
        groupId: nextGroupId++,
        titleIds: cluster.map((t) => t.titleId),
        canonicalDurationSeconds: canonical,
        count: cluster.length,
        isFeatureLength: canonical >= 50 * 60,
      };
      duplicateGroups.push(group);
      for (const item of cluster) {
        duplicateByTitleId[item.titleId] = group;
      }
    }
    cluster = [];
  };

  for (const item of eligible) {
    if (cluster.length === 0) {
      cluster.push(item);
      continue;
    }
    const anchor = cluster[0].durationSeconds;
    if (
      Math.abs(item.durationSeconds - anchor) <=
      DUPLICATE_DURATION_TOLERANCE_SECONDS
    ) {
      cluster.push(item);
    } else {
      flushCluster();
      cluster.push(item);
    }
  }
  flushCluster();

  const duplicateTitleCount = duplicateGroups.reduce(
    (sum, g) => sum + g.count,
    0,
  );
  const hasObfuscatedPattern =
    duplicateGroups.some((g) => g.isFeatureLength && g.count >= 3) ||
    duplicateTitleCount >= 5;

  return {
    mainFeatureTitleId,
    episodeTitleIds,
    duplicateGroups,
    duplicateByTitleId,
    duplicateTitleCount,
    hasObfuscatedPattern,
  };
}

/**
 * Deduplicates a selection of title IDs so at most one title (highest priority) is kept
 * from each duplicate-duration group, preserving all non-duplicate checked titles.
 */
export function dedupeTitleSelection(
  titles: DiscTitle[],
  selectedIds: number[],
  analysis: DiscTitleAnalysis,
): number[] {
  const byId = new Map(titles.map((t) => [t.titleId, t]));
  const selectedSet = new Set(selectedIds);
  const kept = new Set<number>();

  // Keep all non-duplicate selected titles as-is
  for (const id of selectedIds) {
    if (!analysis.duplicateByTitleId[id]) {
      kept.add(id);
    }
  }

  // For each duplicate group that has >= 1 selected title, keep only the highest-priority one
  for (const group of analysis.duplicateGroups) {
    const chosenInGroup = group.titleIds
      .filter((id) => selectedSet.has(id))
      .map((id) => byId.get(id))
      .filter((t): t is DiscTitle => t !== undefined)
      .sort(compareTitlePriority);
    if (chosenInGroup.length > 0) {
      kept.add(chosenInGroup[0].titleId);
    }
  }

  return titles.map((t) => t.titleId).filter((id) => kept.has(id));
}
