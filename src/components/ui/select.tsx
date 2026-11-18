//! Native <select> styled to match the design system (accessible for free).

import { forwardRef, type SelectHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/lib/cn";

export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement>>(
  function Select({ className, children, ...rest }, ref) {
    return (
      <div className={cn("relative inline-flex", className)}>
        <select
          ref={ref}
          className="h-9 w-full appearance-none rounded-lg border border-line bg-surface-2 pr-8 pl-3 text-sm text-fg transition-colors hover:border-line-strong disabled:opacity-50"
          {...rest}
        >
          {children}
        </select>
        <ChevronDown
          aria-hidden
          className="pointer-events-none absolute top-1/2 right-2.5 h-4 w-4 -translate-y-1/2 text-muted"
        />
      </div>
    );
  },
);
