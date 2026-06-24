import { HatGlasses, Pencil, X } from "lucide-react";

import { cn } from "@/shared/lib/cn";

/**
 * Persistent bar above the composer input while anonymous mode is armed — the
 * anti-footgun so anonymity is never an invisible mode (UX §3.2). Shows the ring
 * size, an edit affordance (reopens the picker) and a clear turn-off.
 */
export function AnonArmedBar({
  ringSize,
  onEdit,
  onDisarm,
  className,
}: {
  ringSize: number;
  onEdit: () => void;
  onDisarm: () => void;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 border border-anon-verified/40 bg-anon-verified/10 px-3 py-1.5 text-xs",
        className,
      )}
    >
      <HatGlasses aria-hidden className="h-4 w-4 text-anon-verified" />
      <span className="font-medium text-anon-verified">
        Anonymous · ring of {ringSize}
      </span>
      <button
        className="ml-1 inline-flex items-center gap-1 text-muted-foreground hover:underline"
        onClick={onEdit}
        type="button"
      >
        <Pencil aria-hidden className="h-3 w-3" />
        edit
      </button>
      <button
        className="ml-auto inline-flex items-center gap-1 text-muted-foreground hover:underline"
        onClick={onDisarm}
        type="button"
      >
        <X aria-hidden className="h-3 w-3" />
        turn off
      </button>
    </div>
  );
}
