NIP-AS
======

Anonymous Send
--------------

`draft` `optional` `client` `relay`

**Depends on**: NIP-01, NIP-29, `RESEARCH/RING_SIG_SCHEME.md`

## Abstract

This draft defines Buzz anonymous stream messages as `kind:40009` events carrying a
v1 SAG ring-signature artifact in a `ring_sig` tag. A verifier can check that one
member of the embedded ring signed the message without learning which member.

`kind:40009` is Buzz-native channel content. It is stored, searched, and fetched
with other stream-message kinds, but clients MUST NOT render it as an
authenticated anonymous post until the `ring_sig` artifact verifies and the
membership evidence has been evaluated.

## Event Kind

`kind:40009` is allocated as `KIND_STREAM_MESSAGE_ANON`.

It is in the stream-message family:

| Kind | Constant | Meaning |
| --- | --- | --- |
| `9` | `KIND_STREAM_MESSAGE` | Plain stream message |
| `40002` | `KIND_STREAM_MESSAGE_V2` | Rich stream message |
| `40008` | `KIND_STREAM_MESSAGE_DIFF` | Diff/patch stream message |
| `40009` | `KIND_STREAM_MESSAGE_ANON` | Anonymous ring-signed stream message |

The relay MUST treat `kind:40009` as channel-scoped content and require an `h`
tag containing the channel UUID, same as other stream-message content kinds.

## Tags

A `kind:40009` event uses the normal Buzz message tags plus exactly one
ring-signature tag:

```json
["ring_sig", "<lowercase-hex artifact>", "buzz-ring-sag-v1"]
```

Rules:

- The tag name is exactly `ring_sig`.
- The artifact is the binary v1 SAG artifact encoded as lowercase hex.
- The scheme marker is exactly `buzz-ring-sag-v1`.
- A `kind:40009` event with zero `ring_sig` tags, multiple `ring_sig` tags, or
  a `ring_sig` tag whose third element is not `buzz-ring-sag-v1` is malformed
  and MUST NOT be treated as an authenticated anonymous message. Clients MUST
  reject the anonymous-authentication result rather than selecting the first,
  last, or any verifying `ring_sig` tag.
- Signers and verifiers MUST remove every tag whose first element is `ring_sig`
  before computing `msg_hash`.
- Any other tag remains in the NIP-01 preimage in its original order.

The `ring_sig` tag is the detection key for clients, but it is not sufficient
for trust. Clients MUST run verification and render based on `crypto_valid` and
`anonymity_set_state`.

## Message Hash

The ring-signature `msg_hash` for `kind:40009` MUST use the frozen
ring-signature scheme serialization:

```json
[0,"<ring_hash_hex>",created_at,40009,[tags_without_ring_sig],"content"]
```

This is a flat six-element NIP-01 event-id array. It is byte-identical to
NIP-01 serialization except:

- the pubkey slot is `ring_hash_hex`, not the posting event pubkey;
- every `ring_sig` tag is removed from the tag list before serialization.

Implementations MUST follow NIP-01 JSON escaping exactly, including `\u00XX`
escapes for control characters that do not have a shorter named escape. Non-Rust
implementations must not use a generic pretty JSON serializer or a different
escaping policy.

The event `kind` is part of the preimage. A signature for `kind:9` or `kind:40002`
does not verify as `kind:40009`.

## Ring Limits

For v1 interop, clients and relays MUST use `MAX_RING_SIZE = 256`.

Rings still MUST satisfy the frozen scheme rules:

- `N >= 2`;
- every ring key is a canonical x-only secp256k1 public key accepted by
  BIP-340 `lift_x`;
- no duplicate keys;
- ring order is transmitted verbatim and is transcript-bound.

For anonymous-send `kind:40009`, the ring is an application-layer set encoded in
canonical order. Clients MUST sort the selected lowercase x-only public keys
lexicographically before signing. Relays and clients MUST reject `kind:40009`
artifacts whose serialized ring is not in that canonical order. This signer-
position-blind order prevents UI conventions such as "sender first" from leaking
which ring member signed. The low-level SAG transcript still binds the ordered
ring bytes exactly as serialized.

The ring-signature artifact length is `64*N + 36` bytes before lowercase-hex
encoding.

## Client Behavior

Clients SHOULD include `kind:40009` in stream-message history, search, unread,
and mention queries wherever they include kinds `9` and `40002`.

When rendering a `kind:40009` event:

1. Parse exactly one `ring_sig` tag with marker `buzz-ring-sag-v1`; zero or
   multiple matching tags are unauthenticated/malformed.
2. Verify the ring signature against the channel id and the exact event body.
3. Evaluate membership evidence separately from cryptographic validity.
4. Render from the two-axis result:
   - `crypto_valid == true` and `MEMBER_VERIFIED`: authenticated anonymous
     channel-member post.
   - `crypto_valid == true` and `UNCHECKED_OR_STALE`: valid ring signature, but
     the anonymity set is unconfirmed or stale.
   - `crypto_valid == true` and `NON_MEMBER`: valid ring signature over a ring
     that includes a non-member.
   - `crypto_valid == false`: unauthenticated content; no anonymous trust badge.

The event `pubkey` is a posting/transport key and MUST NOT be displayed as the
message author. The author line is derived from the verified ring result, e.g.
"Anonymous member - 1 of N" for the fully verified state.

## Signing Integration

The integration that calls `buzz-ring-sig` MUST provide fresh CSPRNG
`aux_rand32` for every signature. Deterministic tests may use fixed aux values,
but production signing must not reuse aux randomness for the real signer nonce.

The final Nostr event is signed in the normal NIP-01 way by the posting key
after the `ring_sig` tag is attached. That posting signature authenticates the
event transport envelope only; anonymous authorship comes from the ring
signature.

## Relay Behavior

Relays that support this draft MUST authorize `kind:40009` through the
anonymous ring-auth path before accepting and storing the event as
channel-scoped message content. A supporting relay MUST accept a `kind:40009`
event only if all of the following hold:

- The event has exactly one non-empty `h` tag containing the channel UUID.
- The event has exactly one `ring_sig` tag with marker `buzz-ring-sag-v1`.
- The relay reconstructs the signed transcript by removing all `ring_sig` tags
  and verifies the artifact against the exact event body with `channel_id` set
  to the single `h` tag.
- The artifact ring is a subset of the channel's current member set.
- The artifact ring is encoded in canonical sorted order as specified in Ring
  Limits.
- The outer event `pubkey` is an ephemeral non-member posting key, not a
  current channel member and not a ring key.
- Any submission auth or identity principal, including NIP-98, NIP-42,
  connection AUTH, or delegated auth tags, is absent or an ephemeral non-member
  principal. It MUST NOT be a current channel member or ring key.
- The normal NIP-01 event signature verifies for the outer posting key, and the
  event has the standard channel write scope.

If any requirement fails, the relay MUST reject the event. Authorization is
"a current channel member signed this event," proven by the embedded
`ring_sig`; it is never derived from the outer posting pubkey. Accepted events
are fanned out and returned for filters that request `kind:40009`.

## Interop With NIP-28 Proxy

`kind:40009` is not translated through the NIP-28 proxy in v1. Mapping it to
ordinary kind:42 would lose the anonymous-authorship state unless the proxy also
translated the ring-signature verification result and UI semantics.

Standard Nostr clients that do not understand this draft may ignore the event or
show it as unsupported Buzz content.

## Security Considerations

Network-level anonymity is out of scope. The relay may still know which account
or connection submitted the event. The ring signature hides cryptographic
authorship within the selected ring; it does not hide transport metadata.

Clients MUST NOT collapse `crypto_valid` and `anonymity_set_state` into one
"verified" boolean. In particular, current-members-only evidence is not
timestamp-backed membership evidence and must not render as `MEMBER_VERIFIED`.
