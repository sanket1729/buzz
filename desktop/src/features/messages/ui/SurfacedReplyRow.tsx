import type { TimelineMessage } from "@/features/messages/types";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/**
 * A read-time pointer that surfaces an agent's nested reply to a human up to
 * the root timeline, at the reply's own `createdAt`. The real message is never
 * moved or copied; clicking navigates DOWN to it in its thread (open the thread
 * at its `rootId`, scroll to and highlight the nested message).
 *
 * Reuses the visual idiom of `MessageThreadSummaryRow` (rounded pill, avatar,
 * muted label) but is a distinct component: a thread summary means
 * reply-count/participants and opens a panel rooted at itself, whereas this
 * points at one specific reply and navigates down to it.
 */
export function SurfacedReplyRow({
  message,
  onNavigate,
}: {
  message: TimelineMessage;
  onNavigate: (message: TimelineMessage) => void;
}) {
  return (
    <div className="relative pb-1 pt-0.5">
      <button
        aria-label={`Go to ${message.author}'s reply`}
        className="group relative isolate inline-flex h-8 w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-full text-left text-xs font-medium text-muted-foreground transition-[color,opacity] before:pointer-events-none before:absolute before:-bottom-0.5 before:-left-0.5 before:-right-2 before:-top-0.5 before:-z-10 before:rounded-full before:content-[''] before:transition-[background-color,box-shadow] hover:text-foreground hover:opacity-90 hover:before:bg-background/95 hover:before:ring-1 hover:before:ring-border/70 focus-visible:outline-hidden focus-visible:before:bg-background/95 focus-visible:before:ring-1 focus-visible:before:ring-ring"
        data-surfaced-reply-id={message.id}
        data-testid="surfaced-reply-row"
        onClick={() => onNavigate(message)}
        type="button"
      >
        <UserAvatar
          avatarUrl={message.avatarUrl ?? null}
          className="ml-0.5 h-7 w-7"
          displayName={message.author}
          size="sm"
        />
        <div className="min-w-0">
          <span className="font-medium transition-colors group-hover:text-foreground">
            {message.author}
          </span>
          <span className="mx-1 font-normal text-muted-foreground/50">·</span>
          <span className="font-normal text-muted-foreground/70">replied</span>
        </div>
      </button>
    </div>
  );
}
