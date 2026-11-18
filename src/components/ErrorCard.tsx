//! Error card with retry — the standard failure surface for async screens.

import { useTranslation } from "react-i18next";
import { TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardBody } from "@/components/ui/card";

export function ErrorCard({
  detail,
  onRetry,
  className,
}: {
  /** Technical detail line (error message). */
  detail?: string;
  onRetry?: () => void;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <Card className={className}>
      <CardBody className="flex flex-col items-center gap-3 py-10 text-center">
        <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-delete-bg text-delete-fg">
          <TriangleAlert aria-hidden className="h-5 w-5" />
        </div>
        <div className="space-y-1">
          <p className="text-sm font-semibold text-fg">{t("common.error")}</p>
          {detail ? (
            <p className="mx-auto max-w-md font-mono text-xs break-words text-muted">{detail}</p>
          ) : null}
        </div>
        {onRetry ? (
          <Button variant="secondary" size="sm" onClick={onRetry}>
            {t("common.retry")}
          </Button>
        ) : null}
      </CardBody>
    </Card>
  );
}
