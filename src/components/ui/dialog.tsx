//! Dialog built on the native <dialog> element (Esc + focus handling free).
//! Backdrop click closes; content is aria-labelled.

import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/cn";
import { Button } from "./button";

export function Dialog({
  open,
  onClose,
  title,
  children,
  className,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  className?: string;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const { t } = useTranslation();

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (open && !el.open) el.showModal();
    else if (!open && el.open) el.close();
  }, [open]);

  // Native 'cancel' (Esc) fires close → sync React state via onClose.
  return (
    <dialog
      ref={ref}
      className={cn(
        "dl-dialog m-auto w-[min(560px,92vw)] rounded-xl border border-line bg-surface p-0 shadow-2xl",
        className,
      )}
      aria-label={title}
      onClose={onClose}
      onClick={(e) => {
        // Click on the backdrop (the dialog element itself) closes.
        if (e.target === ref.current) onClose();
      }}
    >
      <div className="flex items-center justify-between border-b border-line px-5 py-3">
        <h2 className="text-sm font-semibold text-fg">{title}</h2>
        <Button variant="ghost" size="icon" aria-label={t("common.close")} onClick={onClose}>
          <X aria-hidden className="h-4 w-4" />
        </Button>
      </div>
      <div className="px-5 py-4">{children}</div>
    </dialog>
  );
}
