//! Module store of `llm://download` job payloads, keyed by job id
//! (`server-<tag>`, `model-<id>`). Cards on the Downloads page subscribe
//! per job id; the store survives route changes so progress is never lost.

import { useSyncExternalStore } from "react";

import type { LlmDownloadPayload } from "./types";

const jobs = new Map<string, LlmDownloadPayload>();
const listeners = new Set<() => void>();

function emit(): void {
  for (const fn of listeners) fn();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Record/replace a payload (events are full snapshots, not deltas). */
export function upsertDownload(payload: LlmDownloadPayload): void {
  jobs.set(payload.job_id, payload);
  emit();
}

export function getDownload(jobId: string | null): LlmDownloadPayload | null {
  if (jobId === null) return null;
  return jobs.get(jobId) ?? null;
}

export function activeDownloadCount(): number {
  let count = 0;
  for (const job of jobs.values()) {
    if (job.state === "running") count += 1;
  }
  return count;
}

/** React binding for one job's latest payload. */
export function useDownloadJob(jobId: string | null): LlmDownloadPayload | null {
  return useSyncExternalStore(subscribe, () => getDownload(jobId), () => null);
}
