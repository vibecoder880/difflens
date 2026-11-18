//! Skeleton — loading placeholder block.

import { cn } from "@/lib/cn";

export function Skeleton({ className }: { className?: string }) {
  return (
    <div
      aria-hidden
      className={cn(
        "animate-pulse rounded-md bg-surface-3 [animation-duration:1.6s]",
        className,
      )}
    />
  );
}
