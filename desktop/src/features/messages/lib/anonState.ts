import type { AnonymousSignatureInfo } from "@/shared/api/types";

/**
 * The render states for a ring-signed (kind:40009) message. v1 keys purely on
 * the ring-signature crypto check — `valid` (the sig verified, author is one of
 * the ring) or `invalid` (it did not). `pending` covers a 40009 event whose
 * signature has not been verified yet.
 *
 * Channel membership of the ring is NOT a client-side concern: the relay already
 * enforces ring ⊆ current members at post time (RESEARCH/RING_SIG_SCHEME.md
 * §7.2/§7.3), so any 40009 that reaches a client already carries a member ring.
 * The client therefore just checks the signature and does not re-derive a
 * membership trust state.
 */
export type AnonTreatment =
  | "valid" // crypto_valid — ring signature verified, author is one of the ring
  | "invalid" // !crypto_valid (or malformed) — reject, not authenticated
  | "pending"; // 40009 event, signature not verified yet

/**
 * Map the verify DTO to exactly one treatment, keyed solely on `cryptoValid`.
 * The membership state on the DTO is intentionally ignored (membership is
 * relay-enforced, not re-checked here). The posting pubkey is never an input
 * (§7.2) — `AnonymousSignatureInfo` carries no author field by construction.
 */
export function anonTreatment(
  signature: AnonymousSignatureInfo | null | undefined,
): AnonTreatment {
  if (!signature) {
    return "pending";
  }
  return signature.cryptoValid ? "valid" : "invalid";
}
