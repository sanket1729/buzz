import * as React from "react";

import {
  anonTreatment,
  type AnonTreatment,
} from "@/features/messages/lib/anonState";
import { AnonStateBadge } from "@/features/messages/ui/AnonStateBadge";
import type { AnonymousSignatureInfo } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/** Public ring members are shown by design (§1.3); keys shortened for display. */
function shortKey(pubkey: string): string {
  return pubkey.length > 14
    ? `${pubkey.slice(0, 8)}…${pubkey.slice(-4)}`
    : pubkey;
}

const HEADLINE: Record<AnonTreatment, (n: number) => string> = {
  valid: (n) => `Anonymous · 1 of ${n}`,
  invalid: () => "Signature failed — treat as unsigned",
  pending: (n) => (n > 0 ? `Anonymous · 1 of ${n}` : "Anonymous"),
};

/**
 * The author line for a received ring-signed message. The author is the ring
 * ("one of N"), NEVER the posting pubkey (§7.2) — this component is given only
 * the verify DTO, which carries no author field. Renders inline inside
 * `MessageHeaderRow`; the expanded ring list wraps to its own line (`w-full`).
 */
export function AnonMessageHeader({
  signature,
}: {
  signature: AnonymousSignatureInfo | null | undefined;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const treatment = anonTreatment(signature);
  const members = signature?.ringMembers ?? [];
  const ringSize = members.length;
  const canShowRing = treatment !== "invalid" && ringSize > 0;

  return (
    <>
      <span
        className={cn(
          "truncate text-sm font-semibold leading-4 tracking-tight",
          treatment === "invalid" ? "text-anon-error" : "text-foreground",
        )}
      >
        {HEADLINE[treatment](ringSize)}
      </span>
      <AnonStateBadge treatment={treatment} />
      {canShowRing ? (
        <button
          type="button"
          aria-expanded={expanded}
          className="text-2xs text-muted-foreground hover:underline"
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? "Hide ring" : `Show ring (${ringSize})`}
        </button>
      ) : null}
      {canShowRing && expanded ? (
        <div className="w-full rounded-lg border border-border bg-muted/40 px-2.5 py-1.5 text-xs text-muted-foreground">
          One of these {ringSize} signed — which one is hidden by design:
          <div className="mt-0.5 break-all font-mono text-foreground">
            {members.map(shortKey).join(" · ")}
          </div>
        </div>
      ) : null}
    </>
  );
}
