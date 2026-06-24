import { normalizePubkey } from "@/shared/lib/pubkey";

/** Public ring members are shown by design (§1.3); keys shortened for display. */
export function shortKey(pubkey: string): string {
  return pubkey.length > 14
    ? `${pubkey.slice(0, 8)}…${pubkey.slice(-4)}`
    : pubkey;
}

/**
 * Assemble the anonymity ring from the signer + chosen others, returned as a
 * CANONICAL set: deduped, lowercased, sorted lexicographically.
 *
 * The canonical sort is load-bearing for anonymity, not cosmetic. Ring order is
 * PUBLIC in the artifact; SAG hides which index signed, but a signer-relative
 * convention (e.g. "me first") would re-encode the signer's position by
 * convention and deanonymize across messages. Sorting makes position carry no
 * signal. The producer (`anonymous_ring_pubkeys`) rejects non-canonically
 * sorted input, so this MUST match its ordering (RING_SIG_SCHEME §7.3).
 */
export function buildCanonicalRing(
  signerPubkey: string,
  otherPubkeys: string[],
): string[] {
  const ring = new Set<string>([normalizePubkey(signerPubkey)]);
  for (const pubkey of otherPubkeys) {
    ring.add(normalizePubkey(pubkey));
  }
  return [...ring].sort();
}

/** Qualitative anonymity strength from ring size (UX §4.1). */
export function ringStrength(ringSize: number): {
  label: string;
  pips: number;
} {
  if (ringSize >= 10) {
    return { label: "Strong", pips: 5 };
  }
  if (ringSize >= 5) {
    return { label: "Good", pips: 4 };
  }
  if (ringSize >= 3) {
    return { label: "Low", pips: 2 };
  }
  return { label: "Minimal", pips: 1 };
}
