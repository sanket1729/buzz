/**
 * Type declarations for the pure projection in `surfaceReplies.mjs`. Runtime
 * lives in `.mjs` so the (TS-loader-less) `node:test` runner imports it
 * directly; this file gives TypeScript callers a typed view.
 */
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Returns the nested messages to surface as root-level pointers: agent-authored
 * messages that carry a human p-tag and are not already duplicated at root.
 *
 * @param messages - the built timeline messages (mixed root + nested).
 * @param isHuman - classifier for a pubkey; the caller resolves unknown
 *   pubkeys to `true` (human) so unrecognized authors under-surface.
 */
export function surfaceReplies(
  messages: TimelineMessage[],
  isHuman: (pubkey: string | undefined) => boolean,
): TimelineMessage[];
