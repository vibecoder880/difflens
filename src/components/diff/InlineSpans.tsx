//! Renders inline word-level diff spans for one side of a pair row.

import { mergeAdjacentSpans, sidesFromSpans } from "@/lib/diffRows";
import { cn } from "@/lib/cn";
import type { DiffSpan } from "@/lib/types";

const OP_STYLES: Record<string, string> = {
  equal: "",
  insert: "bg-insert-bg text-insert-fg rounded-sm px-0.5 -mx-0.5",
  delete: "bg-delete-bg text-delete-fg rounded-sm px-0.5 -mx-0.5 line-through decoration-delete-fg/60",
};

/** Old column: equal + delete. New column: equal + insert. */
export function SpanText({
  spans,
  side,
  className,
}: {
  spans: DiffSpan[];
  side: "old" | "new";
  className?: string;
}) {
  const parts = sidesFromSpans(spans);
  const pieces = mergeAdjacentSpans(side === "old" ? parts.old : parts.new);
  return (
    <span className={cn("whitespace-pre-wrap break-words", className)}>
      {pieces.map((piece, i) => (
        <span key={i} className={OP_STYLES[piece.op]}>
          {piece.text}
        </span>
      ))}
    </span>
  );
}
