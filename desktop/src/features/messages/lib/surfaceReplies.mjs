/**
 * Pure projection that pulls agent-authored replies addressed to a human up to
 * the thread root. The channel timeline only renders root-level entries
 * (`buildMainTimelineEntries` keeps `parentId == null || broadcast`), so an
 * agent's reply to a human posted *nested* is invisible where the human reads.
 * This function returns the nested messages that should be surfaced as
 * lightweight root-level pointers; the real message is never moved or copied —
 * the caller (Phase 2) renders a pointer row that links down to it.
 *
 * Lives in `.mjs` (not `.ts`) so the test runner (`node --test`, no TS loader)
 * imports the same source production uses; TypeScript callers get types from
 * the sibling `.d.mts`.
 */

/**
 * A message is root-level — and therefore needs no surfacing — under the exact
 * condition the timeline uses to render it at root: no parent, or a broadcast
 * reply. Inlined (not imported from `threading.ts`) because that module is
 * `.ts` and cannot be loaded by the TS-loader-less test runner; the rule is one
 * line and must stay identical to `buildMainTimelineEntries`'s filter.
 */
function isRootLevel(message) {
  return (
    message.parentId == null ||
    (message.tags ?? []).some((t) => t[0] === "broadcast" && t[1] === "1")
  );
}

/**
 * Returns the subset of `messages` to surface as root-level pointers.
 *
 * A nested message surfaces iff ALL hold:
 *   1. it is not already root-level (root needs no surfacing);
 *   2. its author is an agent — `isHuman(authorPubkey) === false`;
 *   3. it carries at least one human p-tag — a `["p", pubkey]` where
 *      `isHuman(pubkey) === true`.
 *
 * De-dupe is STRICT and THREAD-SCOPED: a candidate is skipped only when a
 * root-level message *in the same thread* has the EXACT SAME body. Author
 * identity is irrelevant — an agent's earlier, unrelated root-level post must
 * not suppress a genuinely new nested reply. Scoping by thread prevents a root
 * "done" in thread A from suppressing a nested "done" in thread B; short common
 * bodies collide constantly across a busy channel. The key is
 * `${threadId}\u0000${body}` where a message's thread id is `rootId ?? id` (a
 * root's own thread id is its `id`, since its `rootId` is null) and `\u0000`
 * (NUL) cannot appear in an event id or normal body, so keys never collide.
 *
 * Empty/whitespace-only bodies carry no content that can "already exist" at
 * root, so they are never seeded and never suppress — every such candidate that
 * passes the trigger conditions surfaces.
 *
 * `isHuman` is authoritative as given. The caller resolves unknown
 * classification to `true` (human), so an unrecognized author is treated human
 * and fails condition (2): the message under-surfaces rather than mis-surfaces.
 */
export function surfaceReplies(messages, isHuman) {
  const threadKey = (message) =>
    `${message.rootId ?? message.id}\u0000${message.body}`;
  const rootBodies = new Set(
    messages
      .filter((message) => isRootLevel(message) && message.body.trim() !== "")
      .map(threadKey),
  );

  return messages.filter((message) => {
    if (isRootLevel(message)) return false;
    if (isHuman(message.pubkey)) return false;
    const tagsHuman = (message.tags ?? []).some(
      (t) => t[0] === "p" && typeof t[1] === "string" && isHuman(t[1]),
    );
    if (!tagsHuman) return false;
    if (message.body.trim() === "") return true;
    return !rootBodies.has(threadKey(message));
  });
}
