import assert from "node:assert/strict";
import test from "node:test";

// Imports the exact source the renderer wires in Phase 2. No inlined copy → no
// drift between test expectations and production behaviour.
import { surfaceReplies } from "./surfaceReplies.mjs";

const HUMAN = "human-pubkey";
const HUMAN_2 = "human-pubkey-2";
const AGENT = "agent-pubkey";
const AGENT_2 = "agent-pubkey-2";

// Caller contract: unknown pubkeys resolve to human (true). Only the known
// agent pubkeys are non-human; everything else (including undefined) is human.
const isHuman = (pubkey) => pubkey !== AGENT && pubkey !== AGENT_2;

// `rootId` mirrors production: a root-level message has `rootId == null`
// (its own thread id is its `id`); a nested reply carries the thread root's id.
const message = ({
  id,
  pubkey,
  parentId = null,
  rootId = null,
  body = "",
  tags = [],
}) => ({
  id,
  pubkey,
  author: pubkey,
  parentId,
  rootId,
  body,
  createdAt: 0,
  time: "",
  depth: parentId == null ? 0 : 1,
  tags,
});

const p = (pubkey) => ["p", pubkey];
const ids = (msgs) => msgs.map((m) => m.id);

test("agent-authored nested message tagging a human surfaces", () => {
  const nested = message({
    id: "n1",
    pubkey: AGENT,
    parentId: "root1",
    body: "here is your answer",
    tags: [p(HUMAN)],
  });
  const out = surfaceReplies(
    [message({ id: "root1", pubkey: HUMAN }), nested],
    isHuman,
  );
  assert.deepEqual(ids(out), ["n1"]);
});

test("agent-to-agent nested message (no human p-tag) does not surface", () => {
  const nested = message({
    id: "n1",
    pubkey: AGENT,
    parentId: "root1",
    body: "investigating together",
    tags: [p(AGENT_2)],
  });
  const out = surfaceReplies(
    [message({ id: "root1", pubkey: HUMAN }), nested],
    isHuman,
  );
  assert.deepEqual(out, []);
});

test("human-authored nested message tagging a human does not surface", () => {
  const nested = message({
    id: "n1",
    pubkey: HUMAN,
    parentId: "root1",
    body: "what do you think",
    tags: [p(HUMAN_2)],
  });
  const out = surfaceReplies(
    [message({ id: "root1", pubkey: AGENT }), nested],
    isHuman,
  );
  assert.deepEqual(out, []);
});

test("nested agent reply whose exact body already exists at root in the same thread is de-duped", () => {
  const body = "the deploy is green";
  const out = surfaceReplies(
    [
      message({ id: "root1", pubkey: AGENT, body }),
      message({
        id: "n1",
        pubkey: AGENT,
        parentId: "root1",
        rootId: "root1",
        body,
        tags: [p(HUMAN)],
      }),
    ],
    isHuman,
  );
  assert.deepEqual(out, []);
});

test("near-but-not-identical body still surfaces (STRICT, not loose, de-dupe)", () => {
  // Under LOOSE de-dupe (same author already at root) this would be wrongly
  // skipped: the agent has an unrelated root post in this thread. STRICT
  // compares body, so a genuinely new nested reply to the human still surfaces.
  const out = surfaceReplies(
    [
      message({ id: "root1", pubkey: AGENT, body: "the deploy is green" }),
      message({
        id: "n1",
        pubkey: AGENT,
        parentId: "root1",
        rootId: "root1",
        body: "the deploy is green now",
        tags: [p(HUMAN)],
      }),
    ],
    isHuman,
  );
  assert.deepEqual(ids(out), ["n1"]);
});

test("unknown author treated as human under-surfaces (fail-safe)", () => {
  // isHuman returns true for the unknown pubkey → fails the agent-author gate.
  const nested = message({
    id: "n1",
    pubkey: "unknown-pubkey",
    parentId: "root1",
    body: "ambiguous author",
    tags: [p(HUMAN)],
  });
  const out = surfaceReplies(
    [message({ id: "root1", pubkey: HUMAN }), nested],
    isHuman,
  );
  assert.deepEqual(out, []);
});

test("identical body in a different thread surfaces; same-thread root collision de-dupes", () => {
  // Thread A has a root "done"; thread B has a nested agent→human "done".
  // Thread-scoped de-dupe must NOT let thread A's root suppress thread B's
  // reply — but a matching root in the SAME thread still de-dupes.
  const out = surfaceReplies(
    [
      message({ id: "A", pubkey: HUMAN, body: "done" }),
      message({
        id: "nB",
        pubkey: AGENT,
        parentId: "B",
        rootId: "B",
        body: "done",
        tags: [p(HUMAN)],
      }),
      message({ id: "C", pubkey: AGENT, body: "done" }),
      message({
        id: "nC",
        pubkey: AGENT,
        parentId: "C",
        rootId: "C",
        body: "done",
        tags: [p(HUMAN)],
      }),
    ],
    isHuman,
  );
  // nB surfaces (no same-thread root "done"); nC de-dupes (root C is "done").
  assert.deepEqual(ids(out), ["nB"]);
});

test("broadcast-tagged nested message is treated as root and does not surface", () => {
  // Pins the broadcast branch of the inlined isRootLevel: a nested message
  // (parentId != null) carrying ["broadcast","1"] is root-level, so it is
  // excluded from surfacing. Guards against silent drift from
  // buildMainTimelineEntries's canonical filter.
  const out = surfaceReplies(
    [
      message({ id: "root1", pubkey: HUMAN }),
      message({
        id: "n1",
        pubkey: AGENT,
        parentId: "root1",
        rootId: "root1",
        body: "broadcast reply to a human",
        tags: [["broadcast", "1"], p(HUMAN)],
      }),
    ],
    isHuman,
  );
  assert.deepEqual(out, []);
});

test("empty-bodied nested agent reply surfaces despite an empty-bodied root", () => {
  // An empty body carries no content to "already exist" at root, so it neither
  // seeds the de-dupe set nor is suppressed by it.
  const out = surfaceReplies(
    [
      message({ id: "root1", pubkey: HUMAN, body: "" }),
      message({
        id: "n1",
        pubkey: AGENT,
        parentId: "root1",
        rootId: "root1",
        body: "",
        tags: [p(HUMAN)],
      }),
    ],
    isHuman,
  );
  assert.deepEqual(ids(out), ["n1"]);
});
