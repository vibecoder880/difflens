//! Button — variants: primary (accent), secondary, ghost, danger, outline.

import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { Loader2 } from "lucide-react";

import { cn } from "@/lib/cn";

const buttonVariants = cva(
  // Base: 8px rhythm, inline-flex for icon+label rows, visible focus ring.
  "inline-flex select-none items-center justify-center gap-2 rounded-lg font-medium transition-colors duration-150 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary: "bg-accent text-accent-fg hover:bg-accent-hover",
        secondary:
          "bg-surface-2 text-fg border border-line hover:bg-surface-3 hover:border-line-strong",
        ghost: "text-muted hover:bg-surface-2 hover:text-fg",
        outline: "border border-line-strong text-fg hover:bg-surface-2",
        danger:
          "bg-red-600 text-white hover:bg-red-700 dark:bg-red-500 dark:hover:bg-red-400 dark:text-white",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-9 px-4 text-sm",
        lg: "h-10 px-5 text-sm",
        icon: "h-8 w-8 p-0",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  /** Shows a spinner and blocks interaction. */
  busy?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { className, variant, size, busy, disabled, children, type, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      // Buttons default to type="submit"; this app has no forms that submit.
      type={type ?? "button"}
      disabled={disabled || busy}
      className={cn(buttonVariants({ variant, size }), className)}
      {...rest}
    >
      {busy ? <Loader2 aria-hidden className="h-4 w-4 animate-spin" /> : null}
      {children}
    </button>
  );
});
