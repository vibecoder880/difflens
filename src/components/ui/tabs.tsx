//! Tabs — controlled, roving keyboard support (arrow keys), WAI-ARIA roles.

import {
  createContext,
  useCallback,
  useContext,
  useId,
  useRef,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import { cn } from "@/lib/cn";

interface TabsCtx {
  value: string;
  setValue: (v: string) => void;
  baseId: string;
}

const TabsContext = createContext<TabsCtx | null>(null);

function useTabs(): TabsCtx {
  const ctx = useContext(TabsContext);
  if (!ctx) throw new Error("Tabs.* must be used inside <Tabs>");
  return ctx;
}

export function Tabs({
  value,
  onChange,
  children,
  className,
  ariaLabel,
}: {
  value: string;
  onChange: (v: string) => void;
  children: ReactNode;
  className?: string;
  ariaLabel: string;
}) {
  const baseId = useId();
  return (
    <TabsContext.Provider value={{ value, setValue: onChange, baseId }}>
      <div className={cn("flex min-h-0 flex-col", className)}>
        <TabList label={ariaLabel}>{children}</TabList>
      </div>
    </TabsContext.Provider>
  );
}

function TabList({ label, children }: { label: string; children: ReactNode }) {
  const { setValue } = useTabs();
  const listRef = useRef<HTMLDivElement>(null);

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLDivElement>) => {
      const tabs = Array.from(
        listRef.current?.querySelectorAll<HTMLButtonElement>("[role=tab]") ?? [],
      );
      const current = tabs.findIndex((t) => t.getAttribute("aria-selected") === "true");
      let next = -1;
      if (e.key === "ArrowRight") next = (current + 1) % tabs.length;
      else if (e.key === "ArrowLeft") next = (current - 1 + tabs.length) % tabs.length;
      else if (e.key === "Home") next = 0;
      else if (e.key === "End") next = tabs.length - 1;
      if (next >= 0) {
        e.preventDefault();
        const tab = tabs[next];
        const name = tab?.dataset.value;
        if (tab && name) {
          setValue(name);
          tab.focus();
        }
      }
    },
    [setValue],
  );

  return (
    <div
      ref={listRef}
      role="tablist"
      aria-label={label}
      onKeyDown={onKeyDown}
      className="flex shrink-0 items-center gap-1 border-b border-line"
    >
      {children}
    </div>
  );
}

export function Tab({
  value,
  children,
  count,
}: {
  value: string;
  children: ReactNode;
  /** Optional count bubble. */
  count?: number;
}) {
  const { value: active, setValue, baseId } = useTabs();
  const selected = active === value;
  return (
    <button
      type="button"
      role="tab"
      data-value={value}
      id={`${baseId}-tab-${value}`}
      aria-selected={selected}
      aria-controls={`${baseId}-panel-${value}`}
      tabIndex={selected ? 0 : -1}
      onClick={() => setValue(value)}
      className={cn(
        "-mb-px flex items-center gap-1.5 rounded-t-lg border-b-2 px-3 py-2 text-sm font-medium transition-colors",
        selected
          ? "border-accent text-fg"
          : "border-transparent text-muted hover:border-line-strong hover:text-fg",
      )}
    >
      {children}
      {typeof count === "number" ? (
        <span className="tabular rounded-full bg-surface-2 px-1.5 text-xs text-muted">
          {count}
        </span>
      ) : null}
    </button>
  );
}

export function TabPanel({
  value,
  children,
  className,
}: {
  value: string;
  children: ReactNode;
  className?: string;
}) {
  const { value: active, baseId } = useTabs();
  if (active !== value) return null;
  return (
    <div
      role="tabpanel"
      id={`${baseId}-panel-${value}`}
      aria-labelledby={`${baseId}-tab-${value}`}
      tabIndex={0}
      className={cn("min-h-0 flex-1", className)}
    >
      {children}
    </div>
  );
}
