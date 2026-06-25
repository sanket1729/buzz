import type { QueryClient } from "@tanstack/react-query";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { verifyAnonymousEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE_ANON } from "@/shared/constants/kinds";

/**
 * Verify each kind:40009 event's ring signature via the Rust verifier and attach
 * the result as `anonymousSignature`, replacing the events in the channel cache
 * in place (mirrors `backfillAuxForMessages`). The renderer then reads only the
 * verify DTO — never the posting pubkey (RESEARCH/RING_SIG_SCHEME.md §7.2).
 *
 * v1 checks the ring signature only — no channel-membership evidence is passed.
 * Membership is enforced by the relay at post time (ring ⊆ current members,
 * §7.2/§7.3), so a 40009 reaching a client through a conforming Buzz relay has
 * already passed admission; the client just confirms the signature and shows
 * valid/invalid.
 *
 * Best-effort: per-event and overall failures are logged and never reject, so a
 * flaky verify can't blank freshly-loaded messages. Already-verified events are
 * skipped so this is cheap to call on every history fetch.
 */
export async function backfillAnonymousSignatures(
  queryClient: QueryClient,
  channelId: string,
  historyEvents: RelayEvent[],
): Promise<void> {
  const anonEvents = historyEvents.filter(
    (event) =>
      event.kind === KIND_STREAM_MESSAGE_ANON && !event.anonymousSignature,
  );
  if (anonEvents.length === 0) {
    return;
  }

  try {
    const verified = await Promise.all(
      anonEvents.map((event) =>
        // No member set — v1 receive rendering trusts configured Buzz relay
        // admission for membership and checks the ring signature only (§7.2/§7.3).
        verifyAnonymousEvent(event, channelId, null)
          .then((anonymousSignature) => ({ ...event, anonymousSignature }))
          .catch((error) => {
            console.error("Failed to verify anonymous event", event.id, error);
            return null;
          }),
      ),
    );

    const enrichedById = new Map<string, RelayEvent>();
    for (const event of verified) {
      if (event) {
        enrichedById.set(event.id, event);
      }
    }
    if (enrichedById.size === 0) {
      return;
    }

    queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(channelId),
      (current = []) =>
        current.map((event) => enrichedById.get(event.id) ?? event),
    );
  } catch (error) {
    console.error(
      "Failed to backfill anonymous signatures for channel",
      channelId,
      error,
    );
  }
}
