//! Virtualized side-by-side diff pane (TanStack Virtual).
//!
//! Rows come from `buildRows` (changes + collapsible equal-context runs).
//! Every row is a two-column grid (old | new) inside one scroll container,
//! so column scrolling is inherently synchronized.

import {
  forwardRef,
  useDeferredValue,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { ArrowLeftRight, ChevronDown, ChevronRight } from "lucide-react";

import { SpanText } from "./InlineSpans";
import { cn } from "@/lib/cn";
import { buildRows, rowIndexForChange } from "@/lib/diffRows";
import { KIND_GLYPHS, formatNumber, severityStyle } from "@/lib/format";
import type { Change } from "@/lib/types";
import type { DiffRow } from "@/lib/diffRows";

export interface SideBySideHandle {
  /** Expand the containing context run (if collapsed) and scroll to it. */
  revealChange: (changeId: string) => void;
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

function PageChip({ page }: { page: number | null }) {
  const { t } = useTranslation();
  if (page === null) return null;
  return (
    <span className="tabular mr-2 inline-flex shrink-0 items-center rounded border border-line bg-surface-2 px-1 text-[10px] text-subtle">
      {t("diff.page", { page })}
    </span>
  );
}

/** Block id chips for Added/Removed (no inline text exists on the wire). */
function BlockIds({ ids }: { ids: string[] }) {
  if (ids.length === 0) return <span className="text-subtle">—</span>;
  return (
    <span className="flex flex-wrap gap-1">
      {ids.map((id) => (
        <span
          key={id}
          className="tabular rounded border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-[11px] text-muted"
        >
          {id}
        </span>
      ))}
    </span>
  );
}

function KindGlyph({ change }: { change: Change }) {
  const { t } = useTranslation();
  const tone =
    change.kind === "added"
      ? "text-insert-fg"
      : change.kind === "removed"
        ? "text-delete-fg"
        : change.kind === "modified"
          ? "text-severity-medium-text"
          : change.kind === "moved"
            ? "text-accent"
            : "text-subtle";
  return (
    <span
      aria-label={t(`kind.${change.kind}`)}
      title={t(`kind.${change.kind}`)}
      className={cn("mr-1.5 inline-block w-3 shrink-0 text-center font-bold", tone)}
    >
      {KIND_GLYPHS[change.kind]}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

function PairRow({
  row,
  selected,
  onSelect,
}: {
  row: Extract<DiffRow, { type: "change" }>;
  selected: boolean;
  onSelect: (changeId: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const c = row.change;
  const isAdded = c.kind === "added";
  const isRemoved = c.kind === "removed";
  const sev = c.severity !== null ? severityStyle(c.severity) : null;

  return (
    <div
      onClick={() => onSelect(c.id)}
      className={cn(
        "grid h-full cursor-pointer grid-cols-2 gap-px border-b border-line",
        selected && "ring-2 ring-accent ring-inset",
        isAdded && "bg-insert-row",
        isRemoved && "bg-delete-row",
      )}
      data-change-row={c.id}
    >
      {/* Old column */}
      <div className="min-w-0 px-3 py-1.5">
        <div className="flex items-start">
          <PageChip page={c.old_page} />
          <span className="min-w-0 flex-1 text-[13px] leading-snug break-words">
            {isAdded ? (
              <span className="text-subtle">—</span>
            ) : isRemoved ? (
              <BlockIds ids={c.old_block_ids} />
            ) : c.spans.length > 0 ? (
              <span>
                <KindGlyph change={c} />
                <SpanText spans={c.spans} side="old" />
              </span>
            ) : (
              <BlockIds ids={c.old_block_ids} />
            )}
          </span>
        </div>
      </div>
      {/* New column */}
      <div
        className={cn(
          "min-w-0 border-l border-line px-3 py-1.5",
          isAdded && "bg-insert-row",
        )}
      >
        <div className="flex items-start">
          <PageChip page={c.new_page} />
          <span className="min-w-0 flex-1 text-[13px] leading-snug break-words">
            {isRemoved ? (
              <span className="text-subtle">—</span>
            ) : isAdded ? (
              <BlockIds ids={c.new_block_ids} />
            ) : c.spans.length > 0 ? (
              <span>
                <KindGlyph change={c} />
                <SpanText spans={c.spans} side="new" />
              </span>
            ) : (
              <BlockIds ids={c.new_block_ids} />
            )}
          </span>
          {c.kind === "moved" ? (
            <span className="ml-2 flex shrink-0 items-center gap-1 text-xs text-accent">
              <ArrowLeftRight aria-hidden className="h-3.5 w-3.5" />
              {c.move_delta
                ? t("diff.moveDelta", {
                    dx: formatNumber(Math.round(c.move_delta[0]), i18n.language),
                    dy: formatNumber(Math.round(c.move_delta[1]), i18n.language),
                  })
                : t("kind.moved")}
            </span>
          ) : null}
          {sev && c.kind !== "equal" ? (
            <span aria-hidden className={cn("mt-1 ml-2 h-2 w-2 shrink-0 rounded-full", sev.dot)} />
          ) : null}
        </div>
      </div>
    </div>
  );
}

function EqualRow({ change }: { change: Change }) {
  return (
    <div className="grid h-full grid-cols-2 gap-px border-b border-line bg-surface">
      <div className="flex min-w-0 items-center gap-2 overflow-hidden px-3 py-1">
        <PageChip page={change.old_page} />
        <span className="truncate">
          <BlockIds ids={change.old_block_ids} />
        </span>
      </div>
      <div className="flex min-w-0 items-center gap-2 overflow-hidden border-l border-line px-3 py-1">
        <PageChip page={change.new_page} />
        <span className="truncate">
          <BlockIds ids={change.new_block_ids} />
        </span>
      </div>
    </div>
  );
}

function ContextRow({
  row,
  onToggle,
}: {
  row: Extract<DiffRow, { type: "context" }>;
  onToggle: (key: string) => void;
}) {
  const { t } = useTranslation();
  const count = row.changes.length;
  const label = t("diff.context.collapsed", { count });
  if (!row.expanded) {
    return (
      <div className="flex h-full items-center justify-center border-b border-line bg-surface-2">
        <button
          type="button"
          onClick={() => onToggle(row.key)}
          aria-expanded={false}
          aria-label={label}
          className="flex items-center gap-1 rounded-md px-3 py-1 text-xs font-medium text-subtle transition-colors hover:bg-surface-3 hover:text-fg"
        >
          <ChevronRight aria-hidden className="h-3.5 w-3.5" />
          {label}
        </button>
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col border-b border-line">
      <div className="flex items-center justify-center bg-surface-2 py-1">
        <button
          type="button"
          onClick={() => onToggle(row.key)}
          aria-expanded={true}
          aria-label={t("diff.context.collapse")}
          className="flex items-center gap-1 rounded-md px-3 py-0.5 text-xs font-medium text-subtle transition-colors hover:bg-surface-3 hover:text-fg"
        >
          <ChevronDown aria-hidden className="h-3.5 w-3.5" />
          {t("diff.context.collapse")}
        </button>
      </div>
      {row.changes.map((c) => (
        <EqualRow key={c.id} change={c} />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

export const SideBySideView = forwardRef<
  SideBySideHandle,
  {
    changes: Change[];
    selectedId: string | null;
    onSelect: (changeId: string) => void;
  }
>(function SideBySideView({ changes, selectedId, onSelect }, ref) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const parentRef = useRef<HTMLDivElement>(null);
  // Large results: rebuild rows off the interaction path.
  const expandedDeferred = useDeferredValue(expanded);
  const rows = useMemo(() => buildRows(changes, expandedDeferred), [changes, expandedDeferred]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 72,
    overscan: 10,
    getItemKey: (index) => rows[index]?.key ?? String(index),
  });

  useImperativeHandle(ref, () => ({
    revealChange(changeId: string) {
      const idx = rowIndexForChange(rows, changeId);
      if (idx < 0) return;
      const row = rows[idx];
      if (row?.type === "context" && !row.expanded) {
        setExpanded((prev) => {
          const next = new Set(prev);
          next.add(row.key);
          return next;
        });
        // Wait one frame for the expanded rows to mount, then center it.
        window.setTimeout(() => virtualizer.scrollToIndex(idx, { align: "center" }), 80);
      } else {
        virtualizer.scrollToIndex(idx, { align: "center" });
      }
    },
  }));

  // Re-measure on selection so the ring doesn't shift measurements.
  useEffect(() => {
    virtualizer.measure();
  }, [selectedId, virtualizer]);

  const toggle = (key: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  return (
    <div ref={parentRef} className="h-full overflow-auto" role="region" aria-label={t("diff.title")}>
      {/* Sticky column headers */}
      <div className="sticky top-0 z-10 grid grid-cols-2 gap-px border-b border-line-strong bg-surface-2 text-xs font-semibold text-muted">
        <div className="px-3 py-1.5">{t("diff.old")}</div>
        <div className="border-l border-line px-3 py-1.5">{t("diff.new")}</div>
      </div>
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((vi) => {
          const row = rows[vi.index];
          if (!row) return null;
          return (
            <div
              key={vi.key}
              data-index={vi.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${vi.start}px)`,
                minHeight: 40,
              }}
            >
              {row.type === "context" ? (
                <ContextRow row={row} onToggle={toggle} />
              ) : (
                <PairRow
                  row={row}
                  selected={row.change.id === selectedId}
                  onSelect={onSelect}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
});
