//! Tiny module store for the most recent diff job id (phase-07 has no
//! persistence — phase-08 adds a real history). Lets the wizard show live
//! progress after navigating away and back.

import { useSyncExternalStore } from "react";

let lastJobId: string | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const fn of listeners) fn();
}

export function setLastJob(jobId: string): void {
  lastJobId = jobId;
  emit();
}

export function getLastJob(): string | null {
  return lastJobId;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useLastJob(): string | null {
  return useSyncExternalStore(
    subscribe,
    () => lastJobId,
    () => null,
  );
}
