import { expect, type Page, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/reply-surfacing";

// The mock identity (self) — a human, not in mockAgentPubkeys.
const SELF_PUBKEY = "deadbeef".repeat(8);

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(async () => {
      return page.evaluate((channelName) => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }) ?? false
        );
      }, channelName);
    })
    .toBe(true);
}

type EmitInput = {
  channelName: string;
  content: string;
  parentEventId?: string | null;
  pubkey?: string;
  mentionPubkeys?: string[];
};

function emitMockMessage(page: Page, input: EmitInput) {
  return page.evaluate((input) => {
    const emit = (
      window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          parentEventId?: string | null;
          pubkey?: string;
          mentionPubkeys?: string[];
        }) => { id: string; created_at: number };
      }
    ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) {
      throw new Error("Mock message emitter is unavailable.");
    }
    return emit(input);
  }, input);
}

test.describe("reply-surfacing screenshots", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("01 — surfaced reply appears in timeline", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit a root message from a human (bob).
    const root = await emitMockMessage(page, {
      channelName: "general",
      content: "Can someone review the latest deploy?",
      pubkey: TEST_IDENTITIES.bob.pubkey,
    });

    // Emit a nested reply from an agent (alice) mentioning the self (human).
    await emitMockMessage(page, {
      channelName: "general",
      content: "I reviewed the deploy — looks good, shipping now.",
      parentEventId: root.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
    });

    // Assert the surfaced-reply-row appears.
    const surfacedRow = page.getByTestId("surfaced-reply-row");
    await expect(surfacedRow).toBeVisible({ timeout: 5_000 });

    // Screenshot the timeline area.
    const timeline = page.getByTestId("message-timeline");
    await timeline.screenshot({
      path: `${SHOTS}/01-surfaced-reply-in-timeline.png`,
    });
  });

  test("02 — multiple surfaced replies interleave chronologically", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit two root messages from a human.
    const root1 = await emitMockMessage(page, {
      channelName: "general",
      content: "Thread one: design feedback needed",
      pubkey: TEST_IDENTITIES.bob.pubkey,
    });

    const root2 = await emitMockMessage(page, {
      channelName: "general",
      content: "Thread two: backend migration plan",
      pubkey: TEST_IDENTITIES.bob.pubkey,
    });

    // Agent replies to both threads, mentioning the human.
    await emitMockMessage(page, {
      channelName: "general",
      content: "Design feedback: the spacing looks off on mobile.",
      parentEventId: root1.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
    });

    await emitMockMessage(page, {
      channelName: "general",
      content: "Migration plan reviewed — ready to proceed.",
      parentEventId: root2.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
    });

    // Assert multiple surfaced-reply-row elements appear.
    const surfacedRows = page.getByTestId("surfaced-reply-row");
    await expect(surfacedRows).toHaveCount(2, { timeout: 5_000 });

    // Screenshot the timeline.
    const timeline = page.getByTestId("message-timeline");
    await timeline.screenshot({
      path: `${SHOTS}/02-multiple-surfaced-replies.png`,
    });
  });

  test("03 — agent-to-agent reply does NOT surface", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit a root message from a human.
    const root = await emitMockMessage(page, {
      channelName: "general",
      content: "Kick off the pipeline when ready",
      pubkey: TEST_IDENTITIES.bob.pubkey,
    });

    // Agent replies mentioning ANOTHER agent (charlie) — no human p-tag.
    await emitMockMessage(page, {
      channelName: "general",
      content: "Delegating pipeline kick-off to charlie.",
      parentEventId: root.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [TEST_IDENTITIES.charlie.pubkey],
    });

    // Wait a beat to ensure no surfaced row appears.
    await page.waitForTimeout(1_000);

    // Assert NO surfaced-reply-row appears.
    const surfacedRows = page.getByTestId("surfaced-reply-row");
    await expect(surfacedRows).toHaveCount(0);

    // Screenshot the timeline showing no pointer row.
    const timeline = page.getByTestId("message-timeline");
    await timeline.screenshot({
      path: `${SHOTS}/03-agent-to-agent-no-surface.png`,
    });
  });

  test("04 — click navigates to nested message", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    // Emit a root message from a human.
    const root = await emitMockMessage(page, {
      channelName: "general",
      content: "Status update on the release?",
      pubkey: TEST_IDENTITIES.bob.pubkey,
    });

    // Agent replies nested, mentioning self.
    await emitMockMessage(page, {
      channelName: "general",
      content: "Release is on track — all checks green.",
      parentEventId: root.id,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      mentionPubkeys: [SELF_PUBKEY],
    });

    // Wait for the surfaced row.
    const surfacedRow = page.getByTestId("surfaced-reply-row");
    await expect(surfacedRow).toBeVisible({ timeout: 5_000 });

    // Click the surfaced reply row.
    await surfacedRow.click();

    // Assert the thread panel opens.
    const threadPanel = page.getByTestId("message-thread-panel");
    await expect(threadPanel).toBeVisible({ timeout: 5_000 });

    // Screenshot the opened thread panel.
    await threadPanel.screenshot({
      path: `${SHOTS}/04-click-navigates-to-thread.png`,
    });
  });
});
