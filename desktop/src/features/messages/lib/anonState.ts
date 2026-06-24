import type { AnonymousSignatureInfo } from "@/shared/api/types";

/**
 * The four non-collapsing render states for a ring-signed (kind:40009) message,
 * plus `pending` for a 40009 event whose signature has not been verified yet.
 * Authoritative semantics: RESEARCH/RING_SIG_SCHEME.md §6 and
 * GUIDES/ANON_SEND_UX.md §5.
 */
export type AnonTreatment =
  | "member" // crypto_valid + MEMBER_VERIFIED — trusted member-anonymous post
  | "unconfirmed" // crypto_valid + UNCHECKED_OR_STALE — valid sig, set unconfirmed
  | "non_member" // crypto_valid + NON_MEMBER — valid sig, ring is contaminated
  | "invalid" // !crypto_valid (or malformed) — reject, not authenticated
  | "pending"; // 40009 event, signature not verified yet

/**
 * Map the verify DTO to exactly one treatment. Keyed on `cryptoValid` FIRST: an
 * invalid signature is rejected regardless of the membership state (§6). The two
 * trust layers are never collapsed. The posting pubkey is never an input here
 * (§7.2) — `AnonymousSignatureInfo` carries no author field by construction.
 */
export function anonTreatment(
  signature: AnonymousSignatureInfo | null | undefined,
): AnonTreatment {
  if (!signature) {
    return "pending";
  }
  if (!signature.cryptoValid || signature.anonymitySetState === "invalid") {
    return "invalid";
  }
  switch (signature.anonymitySetState) {
    case "member_verified":
      return "member";
    case "unchecked_or_stale":
      return "unconfirmed";
    case "non_member":
      return "non_member";
    default:
      return "invalid";
  }
}
