/**
 * Type declarations for the pure merge in `mergeSurfacedReplies.mjs`. Runtime
 * lives in `.mjs` so the (TS-loader-less) `node:test` runner imports it
 * directly; this file gives TypeScript callers a typed view.
 */
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";

/** A root timeline entry, or a pointer to a surfaced nested reply. */
export type MergedTimelineRow =
  | { kind: "entry"; entry: MainTimelineEntry }
  | { kind: "surfaced"; message: TimelineMessage };

/**
 * Interleaves `surfaced` pointers into the root-level `entries`, returning a
 * single sequence sorted by `(createdAt, id)` using each row's REAL message id.
 */
export function mergeSurfacedReplies(
  entries: MainTimelineEntry[],
  surfaced: TimelineMessage[],
): MergedTimelineRow[];
