//! Determinate progress bar (0–100) with an accessible label.

import { cn } from "@/lib/cn";

export function Progress({
  value,
  label,
  className,
  indeterminate,
}: {
  /** 0..=100; clamped. */
  value: number;
  /** Accessible name; falls back to the percentage. */
  label?: string;
  className?: string;
  /** Indeterminate shimmer when the backend hasn't reported a percent yet. */
  indeterminate?: boolean;
}) {
  const clamped = Math.max(0, Math.min(100, Math.round(value)));
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : clamped}
      aria-label={label ?? `${clamped}%`}
      className={cn("h-1.5 w-full overflow-hidden rounded-full bg-surface-3", className)}
    >
      <div
        className={cn(
          "h-full rounded-full bg-accent transition-[width] duration-300 ease-out",
          indeterminate && "dl-progress-indeterminate w-1/3",
        )}
        style={indeterminate ? undefined : { width: `${clamped}%` }}
      />
    </div>
  );
}
