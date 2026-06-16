/**
 * Pure merge that interleaves surfaced reply pointers into the root-level
 * timeline. `buildMainTimelineEntries` yields the root entries the channel
 * renders, in ascending `createdAt` order; `surfaceReplies` yields the nested
 * agent->human replies to surface as pointers at their OWN `createdAt` (the
 * blessed "surface it where the eye reads" position).
 *
 * The two streams are combined into one discriminated sequence sorted by
 * `(createdAt, id)` so the existing day-divider grouping loop can run over it
 * unchanged. `id` is the REAL message id (a root entry's `message.id`, a
 * pointer's surfaced-message `id`), giving a stable tiebreak when a pointer and
 * a root share a `createdAt` — deterministic insertion, no re-render flicker.
 * This mirrors the `(createdAt, id.localeCompare)` tiebreak already used for
 * review comments in `TimelineMessageList`.
 *
 * Lives in `.mjs` (not `.ts`) so the TS-loader-less test runner imports the
 * same source production uses; the sibling `.d.mts` types the discriminant for
 * TypeScript callers.
 */

/**
 * @param {{ message: { id: string, createdAt: number } }[]} entries
 * @param {{ id: string, createdAt: number }[]} surfaced
 */
export function mergeSurfacedReplies(entries, surfaced) {
  const merged = [
    ...entries.map((entry) => ({ kind: "entry", entry })),
    ...surfaced.map((message) => ({ kind: "surfaced", message })),
  ];

  const sortId = (row) =>
    row.kind === "entry" ? row.entry.message.id : row.message.id;
  const sortCreatedAt = (row) =>
    row.kind === "entry" ? row.entry.message.createdAt : row.message.createdAt;

  return merged.sort((left, right) => {
    const byCreatedAt = sortCreatedAt(left) - sortCreatedAt(right);
    if (byCreatedAt !== 0) return byCreatedAt;
    return sortId(left).localeCompare(sortId(right));
  });
}
