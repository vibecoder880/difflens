//! Tooltip — CSS-only bubble via the .dl-tip utility (hover + keyboard).

import type { ElementType, ReactNode } from "react";

import { cn } from "@/lib/cn";

export function Tooltip({
  text,
  children,
  className,
  below,
  as = "span",
}: {
  text: string;
  children: ReactNode;
  className?: string;
  /** Flip the bubble below the trigger (for bottom-edge controls). */
  below?: boolean;
  as?: ElementType;
}) {
  const Tag = as;
  return (
    <Tag
      data-tip={text}
      className={cn("dl-tip", below && "dl-tip-below", className)}
      aria-label={typeof children === "string" ? undefined : text}
    >
      {children}
    </Tag>
  );
}
