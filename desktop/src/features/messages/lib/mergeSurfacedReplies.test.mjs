import assert from "node:assert/strict";
import test from "node:test";

// Imports the exact source the renderer wires. No inlined copy -> no drift
// between test expectations and production behaviour.
import { mergeSurfacedReplies } from "./mergeSurfacedReplies.mjs";

// Minimal shapes: the merge only reads `message.id` and `message.createdAt`.
const entry = (id, createdAt) => ({
  message: { id, createdAt },
  summary: null,
});
const surfaced = (id, createdAt) => ({ id, createdAt });

// Reduce rows to a comparable trace of (kind, id) in result order.
const trace = (rows) =>
  rows.map((row) =>
    row.kind === "entry"
      ? `entry:${row.entry.message.id}`
      : `surfaced:${row.message.id}`,
  );

test("surfaced reply lands between root entries by its own createdAt", () => {
  const rows = mergeSurfacedReplies(
    [entry("a", 10), entry("c", 30)],
    [surfaced("b", 20)],
  );
  assert.deepEqual(trace(rows), ["entry:a", "surfaced:b", "entry:c"]);
});

test("equal createdAt tiebreaks on real id so insertion is deterministic", () => {
  // Pointer "m" and root "z" share createdAt 20; "m" < "z" by id.
  const rows = mergeSurfacedReplies(
    [entry("a", 10), entry("z", 20)],
    [surfaced("m", 20)],
  );
  assert.deepEqual(trace(rows), ["entry:a", "surfaced:m", "entry:z"]);
});

test("tiebreak uses the real message id not a synthetic render key", () => {
  // If the merge keyed on a "surfaced:" prefix it would sort AFTER "entry"
  // rows at the same createdAt (lexically "s" > "e"); keying on the real id
  // "b" places it before root "c".
  const rows = mergeSurfacedReplies([entry("c", 5)], [surfaced("b", 5)]);
  assert.deepEqual(trace(rows), ["surfaced:b", "entry:c"]);
});

test("empty surfaced list returns the root entries unchanged in order", () => {
  const rows = mergeSurfacedReplies(
    [entry("a", 10), entry("b", 20), entry("c", 30)],
    [],
  );
  assert.deepEqual(trace(rows), ["entry:a", "entry:b", "entry:c"]);
});

test("empty entries returns only the surfaced pointers in createdAt order", () => {
  const rows = mergeSurfacedReplies(
    [],
    [surfaced("late", 30), surfaced("early", 10)],
  );
  assert.deepEqual(trace(rows), ["surfaced:early", "surfaced:late"]);
});

test("multiple surfaced replies interleave across multiple roots", () => {
  const rows = mergeSurfacedReplies(
    [entry("r1", 10), entry("r2", 40)],
    [surfaced("s1", 20), surfaced("s2", 30), surfaced("s3", 50)],
  );
  assert.deepEqual(trace(rows), [
    "entry:r1",
    "surfaced:s1",
    "surfaced:s2",
    "entry:r2",
    "surfaced:s3",
  ]);
});
