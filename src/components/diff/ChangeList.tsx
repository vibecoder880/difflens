//! Filterable change list: kind/severity chips with counts, click → reveal.

import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ListFilter } from "lucide-react";

import { SeverityBadge } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/empty-state";
import { cn } from "@/lib/cn";
import { KIND_GLYPHS, severityStyle } from "@/lib/format";
import { changeExcerpt } from "@/lib/diffRows";
import {
  EMPTY_FILTERS,
  FILTER_KINDS,
  FILTER_SEVERITIES,
  applyFilters,
  countByKind,
  countBySeverity,
  toggleValue,
} from "@/lib/filters";
import type { DiffFilters } from "@/lib/filters";
import type { Change, ChangeKind, Severity } from "@/lib/types";

function FilterChip({
  active,
  count,
  onClick,
  children,
  dotClass,
}: {
  active: boolean;
  count: number;
  onClick: () => void;
  children: React.ReactNode;
  /** Optional colored dot (severity chips). */
  dotClass?: string;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "tabular inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
        active
          ? "border-accent bg-accent-soft text-accent"
          : "border-line bg-surface text-muted hover:border-line-strong hover:text-fg",
        count === 0 && !active && "opacity-50",
      )}
    >
      {dotClass ? <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", dotClass)} /> : null}
      {children}
      <span className="text-subtle">{count}</span>
    </button>
  );
}

export function ChangeList({
  changes,
  selectedId,
  filters,
  onFiltersChange,
  onSelect,
  className,
}: {
  changes: Change[];
  selectedId: string | null;
  filters: DiffFilters;
  onFiltersChange: (next: DiffFilters) => void;
  /** Called with the change id; parent reveals it in the side-by-side. */
  onSelect: (changeId: string) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  const kindCounts = useMemo(() => countByKind(changes), [changes]);
  const sevCounts = useMemo(() => countBySeverity(changes), [changes]);
  const filtered = useMemo(() => applyFilters(changes, filters), [changes, filters]);
  const filtersActive = filters.kinds.length > 0 || filters.severities.length > 0;

  return (
    <aside
      className={cn("flex min-h-0 flex-col border-l border-line bg-surface", className)}
      aria-label={t("diff.list.title")}
    >
      <div className="shrink-0 border-b border-line px-4 pt-3 pb-2">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-fg">
          <ListFilter aria-hidden className="h-4 w-4 text-subtle" />
          {t("diff.list.title")}
          <span className="tabular text-xs font-normal text-subtle">
            {t("diff.list.count", { count: filtered.length })}
          </span>
        </h2>
        <div className="mt-2 flex flex-wrap gap-1.5" role="group" aria-label={t("diff.list.filters")}>
          {FILTER_KINDS.filter((k) => k !== "equal" || kindCounts.equal > 0).map((kind) => (
            <FilterChip
              key={kind}
              active={filters.kinds.includes(kind)}
              count={kindCounts[kind]}
              onClick={() =>
                onFiltersChange({ ...filters, kinds: toggleValue(filters.kinds, kind as ChangeKind) })
              }
            >
              <span aria-hidden className="font-bold">{KIND_GLYPHS[kind]}</span>
              {t(`kind.${kind}`)}
            </FilterChip>
          ))}
        </div>
        <div className="mt-1.5 flex flex-wrap gap-1.5" role="group" aria-label={t("severity.info")}>
          {FILTER_SEVERITIES.map((sev) => (
            <FilterChip
              key={sev}
              active={filters.severities.includes(sev)}
              count={sevCounts[sev]}
              dotClass={severityStyle(sev as Severity).dot}
              onClick={() =>
                onFiltersChange({
                  ...filters,
                  severities: toggleValue(filters.severities, sev as Severity),
                })
              }
            >
              {t(`severity.${sev}`)}
            </FilterChip>
          ))}
        </div>
        {filtersActive ? (
          <button
            type="button"
            onClick={() => onFiltersChange({ ...EMPTY_FILTERS })}
            className="mt-2 text-xs font-medium text-accent hover:underline"
          >
            {t("diff.list.clear")}
          </button>
        ) : null}
      </div>

      {filtered.length === 0 ? (
        <EmptyState
          icon={ListFilter}
          title={changes.length === 0 ? t("diff.empty") : t("diff.list.none")}
          className="min-h-40 flex-1"
        />
      ) : (
        <ol className="min-h-0 flex-1 overflow-y-auto">
          {filtered.map((c) => {
            const selected = c.id === selectedId;
            return (
              <li key={c.id}>
                <button
                  type="button"
                  data-change-id={c.id}
                  aria-label={`${t(`kind.${c.kind}`)} — ${changeExcerpt(c)}`}
                  onClick={() => onSelect(c.id)}
                  className={cn(
                    "flex w-full flex-col gap-1 border-b border-line px-3 py-2 text-left transition-colors",
                    selected ? "bg-accent-soft" : "hover:bg-surface-2",
                  )}
                >
                  <span className="flex items-center gap-2">
                    <span
                      aria-hidden
                      className={cn(
                        "flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-surface-2 text-xs font-bold",
                        c.kind === "added" && "text-insert-fg",
                        c.kind === "removed" && "text-delete-fg",
                        c.kind === "modified" && "text-severity-medium-text",
                        c.kind === "moved" && "text-accent",
                        c.kind === "equal" && "text-subtle",
                      )}
                    >
                      {KIND_GLYPHS[c.kind]}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-xs text-muted">
                      {changeExcerpt(c)}
                    </span>
                    {c.severity !== null ? <SeverityBadge severity={c.severity} /> : null}
                  </span>
                  <span className="tabular text-[10px] text-subtle">
                    {c.id} · {t(`kind.${c.kind}`)}
                    {c.old_page !== null ? ` · ${t("diff.page", { page: c.old_page })}` : ""}
                    {c.new_page !== null && c.old_page !== c.new_page
                      ? ` → ${t("diff.page", { page: c.new_page })}`
                      : ""}
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      )}
    </aside>
  );
}
