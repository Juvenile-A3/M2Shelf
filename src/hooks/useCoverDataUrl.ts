import { useEffect, useMemo, useState } from "react";
import type { MediaNode } from "../types/media";
import { api, desktopAvailable } from "../lib/api";

interface ResolvedCoverEntry {
  url: string | null;
  characters: number;
}

const inFlightCoverRequests = new Map<string, Promise<string | null>>();
const resolvedCoverUrls = new Map<string, ResolvedCoverEntry>();
const coverQueue: Array<() => void> = [];
const MAX_CONCURRENT_COVER_REQUESTS = 4;
const MAX_RESOLVED_COVER_ENTRIES = 128;
const MAX_RESOLVED_COVER_CHARACTERS = 32 * 1024 * 1024;
let activeCoverRequests = 0;
let resolvedCoverCharacters = 0;

function scheduleCoverRequest(run: () => Promise<string | null>): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const start = () => {
      activeCoverRequests += 1;
      void run()
        .then(resolve, reject)
        .finally(() => {
          activeCoverRequests -= 1;
          coverQueue.shift()?.();
        });
    };
    if (activeCoverRequests < MAX_CONCURRENT_COVER_REQUESTS) start();
    else coverQueue.push(start);
  });
}

function removeResolvedCover(key: string) {
  const existing = resolvedCoverUrls.get(key);
  if (!existing) return;
  resolvedCoverCharacters -= existing.characters;
  resolvedCoverUrls.delete(key);
}

function rememberResolvedCover(key: string, url: string | null) {
  removeResolvedCover(key);
  const entry = { url, characters: url?.length ?? 0 };
  resolvedCoverUrls.set(key, entry);
  resolvedCoverCharacters += entry.characters;
  while (resolvedCoverUrls.size > MAX_RESOLVED_COVER_ENTRIES
    || resolvedCoverCharacters > MAX_RESOLVED_COVER_CHARACTERS) {
    const oldestKey = resolvedCoverUrls.keys().next().value;
    if (oldestKey == null) break;
    removeResolvedCover(oldestKey);
  }
}

function requestCover(nodeId: number, key: string): Promise<string | null> {
  const resolved = resolvedCoverUrls.get(key);
  if (resolved) {
    // Refresh insertion order so active covers survive the bounded LRU eviction.
    resolvedCoverUrls.delete(key);
    resolvedCoverUrls.set(key, resolved);
    return Promise.resolve(resolved.url);
  }
  const inFlight = inFlightCoverRequests.get(key);
  if (inFlight) return inFlight;

  // A metadata/cover revision supersedes the previous data URL for this Node immediately.
  const nodePrefix = `${nodeId}:`;
  [...resolvedCoverUrls.keys()]
    .filter((existingKey) => existingKey !== key && existingKey.startsWith(nodePrefix))
    .forEach(removeResolvedCover);

  const request = scheduleCoverRequest(() => api.getCoverDataUrl(nodeId))
    .then((url) => {
      rememberResolvedCover(key, url);
      return url;
    })
    .finally(() => inFlightCoverRequests.delete(key));
  inFlightCoverRequests.set(key, request);
  return request;
}

export function useCoverDataUrl(node: MediaNode | null | undefined, revision: number, enabled = true) {
  const nodeId = node?.id;
  const coverPath = node?.coverCachePath ?? node?.binding?.coverCachePath;
  const nodeUpdatedAt = node?.updatedAt ?? "";
  const bindingUpdatedAt = node?.binding?.updatedAt ?? "";
  const subjectId = node?.binding?.providerSubjectId ?? "";
  const coverError = node?.binding?.coverDownloadError ?? "";
  const clientCoverRevision = node?.clientCoverRevision ?? 0;
  const hasCachedCover = Boolean(coverPath);
  const cacheKey = useMemo(() => node
    ? `${node.id}:${subjectId}:${coverPath ?? ""}:${coverError}:${nodeUpdatedAt}:${bindingUpdatedAt}:${clientCoverRevision}:${revision}`
    : "",
  [bindingUpdatedAt, clientCoverRevision, coverError, coverPath, node, nodeUpdatedAt, revision, subjectId]);
  const [state, setState] = useState<{ key: string; url: string | null; failed: boolean; loading: boolean }>({
    key: "",
    url: null,
    failed: false,
    loading: false,
  });

  useEffect(() => {
    if (!enabled || !desktopAvailable || nodeId == null || !hasCachedCover) {
      setState({ key: cacheKey, url: null, failed: false, loading: false });
      return;
    }
    let active = true;
    setState({ key: cacheKey, url: null, failed: false, loading: true });
    void requestCover(nodeId, cacheKey)
      .then((url) => active && setState({ key: cacheKey, url, failed: !url, loading: false }))
      .catch(() => active && setState({ key: cacheKey, url: null, failed: true, loading: false }));
    return () => { active = false; };
  }, [cacheKey, enabled, hasCachedCover, nodeId]);

  return {
    coverUrl: state.key === cacheKey ? state.url : null,
    coverFailed: state.key === cacheKey && state.failed,
    // A deferred card still has a valid cached cover; treat it as waiting rather than failed so
    // an off-screen poster never flashes the retry state before IntersectionObserver starts it.
    coverLoading: hasCachedCover && (!enabled || state.key !== cacheKey || state.loading),
  };
}
