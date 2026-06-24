import * as React from "react";
import { HatGlasses, Search } from "lucide-react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import {
  buildCanonicalRing,
  ringStrength,
  shortKey,
} from "@/features/messages/lib/anonRing";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";

type AnonRingPickerProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channelId: string | null;
  currentPubkey: string;
  /** Pubkeys (incl. signer) of the currently-armed ring, to pre-check on reopen. */
  initialSelection?: string[];
  /** Receives the canonical (sorted, deduped, lowercase) ring including the signer. */
  onConfirm: (ringPubkeys: string[]) => void;
};

/**
 * Ring-member picker. The sender hides among current channel members of their
 * choosing (UX §4). "You" is always in the ring and locked (signer ∈ ring); the
 * ring needs ≥ 1 other member (ring ≥ 2) or it deanonymizes by construction. The
 * confirmed ring is canonicalized so its public order carries no signer signal.
 */
export function AnonRingPicker({
  open,
  onOpenChange,
  channelId,
  currentPubkey,
  initialSelection,
  onConfirm,
}: AnonRingPickerProps) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md overflow-hidden p-0">
        {/* Remounts per open (Radix unmounts closed content) → fresh selection. */}
        {open ? (
          <RingPickerBody
            channelId={channelId}
            currentPubkey={currentPubkey}
            initialSelection={initialSelection}
            onCancel={() => onOpenChange(false)}
            onConfirm={(ring) => {
              onConfirm(ring);
              onOpenChange(false);
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function RingPickerBody({
  channelId,
  currentPubkey,
  initialSelection,
  onCancel,
  onConfirm,
}: {
  channelId: string | null;
  currentPubkey: string;
  initialSelection?: string[];
  onCancel: () => void;
  onConfirm: (ringPubkeys: string[]) => void;
}) {
  const me = normalizePubkey(currentPubkey);
  const membersQuery = useChannelMembersQuery(channelId, channelId !== null);
  const [query, setQuery] = React.useState("");
  const [selectedOthers, setSelectedOthers] = React.useState<Set<string>>(
    () =>
      new Set(
        (initialSelection ?? [])
          .map(normalizePubkey)
          .filter((pubkey) => pubkey !== me),
      ),
  );

  const others = (membersQuery.data ?? []).filter(
    (member) => normalizePubkey(member.pubkey) !== me,
  );
  const needle = query.trim().toLowerCase();
  const filtered = needle
    ? others.filter(
        (member) =>
          (member.displayName ?? "").toLowerCase().includes(needle) ||
          member.pubkey.toLowerCase().includes(needle),
      )
    : others;

  const ringSize = selectedOthers.size + 1; // + You
  const canConfirm = selectedOthers.size >= 1; // ring ≥ 2 (you + ≥1 other)
  const strength = ringStrength(ringSize);

  const toggle = (pubkey: string) => {
    const normalized = normalizePubkey(pubkey);
    setSelectedOthers((previous) => {
      const next = new Set(previous);
      if (next.has(normalized)) {
        next.delete(normalized);
      } else {
        next.add(normalized);
      }
      return next;
    });
  };

  return (
    <div className="flex max-h-[80vh] flex-col">
      <DialogHeader className="shrink-0 border-b border-border/60 px-5 py-4">
        <DialogTitle className="flex items-center gap-2 text-anon-verified">
          <HatGlasses aria-hidden className="h-5 w-5" />
          Build your anonymity ring
        </DialogTitle>
        <DialogDescription>
          Pick members to hide among. Everyone will see this list — only which
          one is you stays hidden.
        </DialogDescription>
      </DialogHeader>

      <div className="relative shrink-0 px-5 pt-3">
        <Search
          aria-hidden
          className="pointer-events-none absolute left-7 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
        />
        <input
          className="h-9 w-full rounded-md border border-input bg-background pl-8 pr-3 text-sm focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search members…"
          value={query}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3">
        <div className="flex items-center gap-2.5 rounded-md px-2 py-1.5">
          <Checkbox aria-label="You (always in the ring)" checked disabled />
          <span className="text-sm font-medium">You</span>
          <span className="ml-auto text-2xs text-muted-foreground">
            always in
          </span>
        </div>
        {membersQuery.isLoading ? (
          <p className="px-2 py-3 text-sm text-muted-foreground">
            Loading members…
          </p>
        ) : filtered.length === 0 ? (
          <p className="px-2 py-3 text-sm text-muted-foreground">
            No other members found.
          </p>
        ) : (
          filtered.map((member) => {
            const normalized = normalizePubkey(member.pubkey);
            return (
              <label
                className="flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 hover:bg-muted/50"
                htmlFor={`ring-${normalized}`}
                key={normalized}
              >
                <Checkbox
                  checked={selectedOthers.has(normalized)}
                  id={`ring-${normalized}`}
                  onCheckedChange={() => toggle(member.pubkey)}
                />
                <span className="truncate text-sm">
                  {member.displayName ?? shortKey(member.pubkey)}
                </span>
              </label>
            );
          })
        )}
      </div>

      <div className="shrink-0 border-t border-border/60 px-5 py-3">
        <div className="mb-2 flex items-center gap-2 text-xs">
          <div aria-hidden className="flex gap-0.5">
            {[0, 1, 2, 3, 4].map((pip) => (
              <span
                className={cn(
                  "h-1.5 w-4 rounded-full",
                  pip < strength.pips ? "bg-anon-verified" : "bg-muted",
                )}
                key={pip}
              />
            ))}
          </div>
          <span className="text-muted-foreground">
            {strength.label} — {ringSize}{" "}
            {ringSize === 1 ? "member" : "members"} (you + {selectedOthers.size}
            )
          </span>
        </div>
        {ringSize === 2 ? (
          <p className="mb-2 text-2xs text-anon-warning">
            Minimal anonymity — only one other member. Anyone who knows it
            wasn't them knows it was you.
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button onClick={onCancel} variant="ghost">
            Cancel
          </Button>
          <Button
            disabled={!canConfirm}
            onClick={() =>
              onConfirm(buildCanonicalRing(currentPubkey, [...selectedOthers]))
            }
          >
            {canConfirm ? "Use this ring" : "Add at least one other"}
          </Button>
        </div>
      </div>
    </div>
  );
}
