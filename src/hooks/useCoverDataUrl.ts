import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import type { MediaNode } from "../types/media";
import { api, desktopAvailable } from "../lib/api";

interface ResolvedCoverEntry {
  url: string | null;
  characters: number;
}

const inFlightCoverRequests = new Map<string, Promise<string | null>>();
const latestCoverKeyByNode = new Map<number, string>();
const resolvedCoverUrls = new Map<string, ResolvedCoverEntry>();
const resolvedCoverEvictionListeners = new Map<string, Set<() => void>>();
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

function notifyResolvedCoverEvicted(key: string) {
  const listeners = resolvedCoverEvictionListeners.get(key);
  if (!listeners) return;
  // Eviction is a one-shot event for this exact revision key. Remove the listener set first so
  // released cards cannot keep stale setState closures alive after the source entry is gone.
  resolvedCoverEvictionListeners.delete(key);
  listeners.forEach((listener) => listener());
}

function subscribeResolvedCoverEviction(key: string, listener: () => void) {
  const listeners = resolvedCoverEvictionListeners.get(key) ?? new Set<() => void>();
  listeners.add(listener);
  resolvedCoverEvictionListeners.set(key, listeners);
  return () => {
    listeners.delete(listener);
    if (!listeners.size && resolvedCoverEvictionListeners.get(key) === listeners) {
      resolvedCoverEvictionListeners.delete(key);
    }
  };
}

function removeResolvedCover(key: string, notify = true) {
  const existing = resolvedCoverUrls.get(key);
  if (!existing) return;
  resolvedCoverCharacters -= existing.characters;
  resolvedCoverUrls.delete(key);
  if (notify) notifyResolvedCoverEvicted(key);
}

function rememberResolvedCover(key: string, url: string | null) {
  removeResolvedCover(key, false);
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
  const nodePrefix = String(nodeId) + ":";
  [...resolvedCoverUrls.keys()]
    .filter((existingKey) => existingKey !== key && existingKey.startsWith(nodePrefix))
    .forEach((existingKey) => removeResolvedCover(existingKey));

  // Route changes must not cancel work that already entered this bounded queue. Completing it
  // warms the cross-page LRU, matching the smooth navigation behavior of 0.5.7.
  latestCoverKeyByNode.set(nodeId, key);
  const request = scheduleCoverRequest(() => api.getCoverDataUrl(nodeId))
    .then((url) => {
      // A superseded request may finish after a bind, retry, cache clear, or scan refresh. Its
      // caller can settle normally, but it must not push an obsolete revision back into the LRU.
      if (latestCoverKeyByNode.get(nodeId) === key) rememberResolvedCover(key, url);
      return url;
    })
    .finally(() => {
      inFlightCoverRequests.delete(key);
      const nodePrefix = String(nodeId) + ":";
      if (![...inFlightCoverRequests.keys()].some((existingKey) => existingKey.startsWith(nodePrefix))) {
        latestCoverKeyByNode.delete(nodeId);
      }
    });
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

  // Off-screen cards may keep an exact cached preview mounted so route restoration never flashes
  // a placeholder. Once that exact source entry reaches the 128/32 MiB LRU boundary, notify only
  // its mounted consumers so released cards can drop their final data-URL reference as well.
  const [evictedCacheKey, setEvictedCacheKey] = useState("");
  useLayoutEffect(() => {
    if (enabled || !hasCachedCover || !cacheKey) return;
    if (!resolvedCoverUrls.has(cacheKey)) {
      setEvictedCacheKey(cacheKey);
      return;
    }
    return subscribeResolvedCoverEviction(cacheKey, () => {
      setEvictedCacheKey(cacheKey);
    });
  }, [cacheKey, enabled, hasCachedCover]);

  useEffect(() => {
    if (!enabled || !desktopAvailable || nodeId == null || !hasCachedCover) {
      setState({ key: cacheKey, url: null, failed: false, loading: false });
      return;
    }
    let active = true;
    const resolved = resolvedCoverUrls.get(cacheKey);
    if (resolved) {
      resolvedCoverUrls.delete(cacheKey);
      resolvedCoverUrls.set(cacheKey, resolved);
      setState({ key: cacheKey, url: resolved.url, failed: !resolved.url, loading: false });
      return;
    }
    setState({ key: cacheKey, url: null, failed: false, loading: true });
    void requestCover(nodeId, cacheKey)
      .then((url) => {
        if (active) setState({ key: cacheKey, url, failed: !url, loading: false });
      })
      .catch(() => active && setState({ key: cacheKey, url: null, failed: true, loading: false }));
    return () => { active = false; };
  }, [cacheKey, enabled, hasCachedCover, nodeId]);

  // An exact revision-key hit is safe to show synchronously even while deferred. It never starts
  // new IPC work; it only prevents a route remount from flashing the placeholder for one frame.
  const resolved = desktopAvailable && nodeId != null && hasCachedCover && evictedCacheKey !== cacheKey
    ? resolvedCoverUrls.get(cacheKey)
    : undefined;
  const current = resolved
    ? { key: cacheKey, url: resolved.url, failed: !resolved.url, loading: false }
    : state.key === cacheKey
      ? state
      : null;
  return {
    coverCacheKey: cacheKey,
    coverUrl: current?.url ?? null,
    coverFailed: current?.failed ?? false,
    // A deferred card still has a valid cached cover; treat it as waiting rather than failed so
    // an off-screen poster never flashes the retry state before IntersectionObserver starts it.
    coverLoading: hasCachedCover && !resolved && (!enabled || state.key !== cacheKey || state.loading),
  };
}
