import { useEffect, useMemo, useState } from "react";
import type { MediaNode } from "../types/media";
import { api, desktopAvailable } from "../lib/api";

const coverRequests = new Map<string, Promise<string | null>>();
const coverQueue: Array<() => void> = [];
const MAX_CONCURRENT_COVER_REQUESTS = 4;
let activeCoverRequests = 0;

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

function requestCover(nodeId: number, key: string): Promise<string | null> {
  const cached = coverRequests.get(key);
  if (cached) return cached;
  if (coverRequests.size > 600) coverRequests.clear();
  const request = scheduleCoverRequest(() => api.getCoverDataUrl(nodeId)).catch((error) => {
    coverRequests.delete(key);
    throw error;
  });
  coverRequests.set(key, request);
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
