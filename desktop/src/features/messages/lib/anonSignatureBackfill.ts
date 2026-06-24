import type { QueryClient } from "@tanstack/react-query";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { verifyAnonymousEvent } from "@/shared/api/tauri";
import type { ChannelMember, RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE_ANON } from "@/shared/constants/kinds";

/**
 * Verify each kind:40009 event's ring signature via the Rust verifier and attach
 * the result as `anonymousSignature`, replacing the events in the channel cache
 * in place (mirrors `backfillAuxForMessages`). The renderer then reads only the
 * verify DTO — never the posting pubkey (RESEARCH/RING_SIG_SCHEME.md §7.2).
 *
 * Membership evidence comes from the cached channel member set (the verifier
 * uses `CurrentMembersOnly` when supplied, `Unchecked` otherwise — never a
 * promotion to `member_verified` without it, §5.1.1).
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

  // Matches the key written by useChannelMembersQuery.
  const memberPubkeys =
    queryClient
      .getQueryData<ChannelMember[]>(["channels", channelId, "members"])
      ?.map((member) => member.pubkey) ?? null;

  try {
    const verified = await Promise.all(
      anonEvents.map((event) =>
        verifyAnonymousEvent(event, channelId, memberPubkeys)
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
