import {
  BadgeCheck,
  CircleDashed,
  CircleSlash,
  TriangleAlert,
} from "lucide-react";

import type { AnonTreatment } from "@/features/messages/lib/anonState";
import { cn } from "@/shared/lib/cn";

type BadgeSpec = {
  label: string;
  Icon: typeof BadgeCheck;
  /** Tone classes — distinct by shape AND color, never color alone (§1.5). */
  tone: string;
};

const BADGES: Record<Exclude<AnonTreatment, "pending">, BadgeSpec> = {
  member: {
    label: "Verified member",
    Icon: BadgeCheck,
    tone: "bg-anon-verified text-anon-verified-foreground",
  },
  unconfirmed: {
    label: "Unconfirmed",
    Icon: CircleDashed,
    tone: "border border-anon-unconfirmed text-anon-unconfirmed",
  },
  non_member: {
    label: "Not a member post",
    Icon: TriangleAlert,
    tone: "border border-anon-warning bg-anon-warning-bg text-anon-warning",
  },
  invalid: {
    // §5.6: no trust badge — quiet, de-emphasized, no fill.
    label: "Signature failed",
    Icon: CircleSlash,
    tone: "text-anon-error",
  },
};

/**
 * The state badge for a ring-signed message. Each state is distinct by glyph
 * silhouette + label + fill style + color so it survives monochrome and screen
 * readers (GUIDES/ANON_SEND_UX.md §5, §7).
 */
export function AnonStateBadge({
  treatment,
  className,
}: {
  treatment: AnonTreatment;
  className?: string;
}) {
  if (treatment === "pending") {
    return (
      <span
        className={cn(
          "inline-flex items-center gap-1 text-2xs font-medium text-muted-foreground",
          className,
        )}
      >
        <CircleDashed className="h-3 w-3 animate-pulse" aria-hidden />
        Verifying…
      </span>
    );
  }

  const { label, Icon, tone } = BADGES[treatment];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-2xs font-semibold",
        tone,
        className,
      )}
    >
      <Icon className="h-3 w-3" aria-hidden />
      {label}
    </span>
  );
}
