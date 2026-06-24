#![deny(unsafe_code)]
#![warn(missing_docs)]
//! v1 SAG ring signatures for Buzz anonymous send.
//!
//! This crate implements the reviewed non-linkable AOS/SAG construction over
//! existing Nostr secp256k1 x-only keys. It exposes the frozen artifact format,
//! message transcript hashing, signing, verification, and the separate
//! crypto-valid vs. channel-membership anonymity-set state required by the spec.

pub mod test_vectors;

use nostr::secp256k1::{Parity, PublicKey, Scalar, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// ASCII scheme identifier bound by v1 transcripts.
pub const SCHEME_ID: &str = "buzz-ring-sag-v1";
/// Canonical artifact version byte for v1.
pub const VERSION: u8 = 0x01;
/// Canonical scheme byte for v1 SAG.
pub const SAG_SCHEME_BYTE: u8 = 0x01;
/// Reserved scheme byte for a future linkable variant; v1 verification rejects it.
pub const RESERVED_BLSAG_SCHEME_BYTE: u8 = 0x02;
/// Frozen Nostr tag name for the serialized ring-signature artifact.
pub const RING_SIG_TAG: &str = "ring_sig";
/// Label for the NIP-01 pubkey slot in the v1 `msg_hash` preimage.
///
/// The slot is filled with the 64-character lowercase `ring_hash` hex string,
/// not the transport author's pubkey and not an all-zero placeholder.
pub const MSG_HASH_PUBKEY_SLOT_LABEL: &str = "ring_hash_hex";
/// Minimum v1 anonymity-set size.
pub const MIN_RING_SIZE: usize = 2;
/// Conservative parser cap for v1 artifacts.
pub const MAX_RING_SIZE: usize = 256;
/// Serialized artifact header length: version, scheme byte, and u16 ring size.
pub const ARTIFACT_HEADER_LEN: usize = 4;
/// Serialized artifact trailer length: c0 scalar.
pub const ARTIFACT_C0_LEN: usize = 32;
/// Length in bytes of the artifact per ring member.
pub const ARTIFACT_BYTES_PER_RING_MEMBER: usize = 64;

const CHALLENGE_TAG: &str = "buzz.ringsig/v1/challenge";
const CHANNEL_TAG: &[u8] = b"buzz.ringsig/v1/channel";
const DECOY_TAG: &str = "buzz.ringsig/v1/decoy";
const NONCE_TAG: &str = "buzz.ringsig/v1/nonce";
const RING_TAG: &str = "buzz.ringsig/v1/ring";
const TRANSCRIPT_TAG: &str = "buzz.ringsig/v1/transcript";

const SECP256K1_ORDER: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];

/// Errors returned by v1 ring-signature helpers.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RingSigError {
    /// The scheme id did not match [`SCHEME_ID`].
    #[error("unsupported scheme id {0:?}")]
    UnsupportedScheme(String),
    /// The artifact version byte is not accepted by v1.
    #[error("unsupported artifact version 0x{0:02x}")]
    UnsupportedVersion(u8),
    /// The artifact scheme byte is not accepted by v1.
    #[error("unsupported artifact scheme byte 0x{0:02x}")]
    UnsupportedSchemeByte(u8),
    /// A scalar, pubkey, or artifact hex string was not canonical lowercase fixed-width hex.
    #[error("invalid {field} hex: {value:?}")]
    InvalidHex {
        /// Field name.
        field: &'static str,
        /// Rejected value.
        value: String,
    },
    /// Ring size failed v1 bounds.
    #[error("invalid ring size {0}")]
    InvalidRingSize(usize),
    /// A ring pubkey appeared more than once.
    #[error("duplicate ring pubkey {0}")]
    DuplicateRingPubkey(String),
    /// A ring pubkey failed BIP-340 x-only lift.
    #[error("invalid ring pubkey {0}")]
    InvalidPubkey(String),
    /// The channel id failed canonical UUID handling.
    #[error("invalid channel id: {0}")]
    InvalidChannelId(String),
    /// The artifact byte length did not match its declared ring size.
    #[error("invalid artifact length: expected {expected}, got {actual}")]
    InvalidArtifactLength {
        /// Expected length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// A scalar was zero where the spec requires a non-zero scalar.
    #[error("zero {0} scalar")]
    ZeroScalar(&'static str),
    /// A scalar was not canonical for secp256k1.
    #[error("invalid {0} scalar")]
    InvalidScalar(&'static str),
    /// The supplied secret key is invalid.
    #[error("invalid secret key")]
    InvalidSecretKey,
    /// The signer index does not identify the supplied secret key inside the ring.
    #[error("signer secret key does not match ring index {0}")]
    SignerIndexMismatch(usize),
    /// Signing failed after bounded retry attempts.
    #[error("signing failed: {0}")]
    SigningFailed(&'static str),
    /// The ring commitment became the point at infinity.
    #[error("invalid ring commitment")]
    InvalidCommitment,
    /// Verification failed because the challenge chain did not close.
    #[error("invalid ring signature")]
    InvalidSignature,
    /// Test-vector JSON failed to parse.
    #[error("invalid test-vector json: {0}")]
    InvalidJson(String),
}

/// Unsigned Buzz/Nostr event body covered by `msg_hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedEventBody {
    /// Event creation time, unix seconds.
    pub created_at: u64,
    /// Event kind. It is read from the event and included in the frozen preimage.
    pub kind: u64,
    /// Event tags. The frozen `ring_sig` tag is excluded before hashing.
    pub tags: Vec<Vec<String>>,
    /// Event content.
    pub content: String,
}

/// Membership evidence supplied by the caller for the Layer-2 anonymity-set state.
#[derive(Debug, Clone, Copy)]
pub enum MembershipCheck<'a> {
    /// Do not evaluate channel membership.
    Unchecked,
    /// Live/current channel-member x-only pubkeys as lowercase hex.
    ///
    /// This evidence can prove that a current member is missing from the ring,
    /// but it cannot prove that all-present keys were members at `created_at`.
    CurrentMembersOnly(&'a [String]),
    /// Timestamp-backed channel-member x-only pubkeys as lowercase hex.
    ///
    /// Only this evidence can upgrade an all-present ring to
    /// [`AnonymitySetState::MemberVerified`].
    MembersAtCreatedAt(&'a [String]),
}

/// Layer-2 channel-membership result for the ring anonymity set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnonymitySetState {
    /// Every ring key was confirmed as a channel member by the supplied evidence.
    MemberVerified,
    /// Membership was not checked or the evidence is stale/insufficient.
    UncheckedOrStale,
    /// At least one ring key is not a channel member.
    NonMember,
}

/// Successful verification result. Invalid signatures return an error instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationResult {
    /// Layer-1 ring-signature validity. Always true for this success type.
    pub crypto_valid: bool,
    /// Separate Layer-2 membership/anonymity-set state.
    pub anonymity_set_state: AnonymitySetState,
}

/// Parsed v1 SAG signature artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagArtifact {
    /// Ordered ring x-only public keys exactly as serialized in the artifact.
    ring: Vec<[u8; 32]>,
    /// Initial challenge scalar bytes.
    c0: [u8; 32],
    /// Per-ring response scalar bytes.
    responses: Vec<[u8; 32]>,
}

impl SagArtifact {
    /// Return the artifact's expected serialized byte length for a ring size.
    pub fn expected_len(ring_len: usize) -> usize {
        ARTIFACT_HEADER_LEN + ARTIFACT_C0_LEN + ARTIFACT_BYTES_PER_RING_MEMBER * ring_len
    }

    /// Serialize the artifact in the frozen v1 wire format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::expected_len(self.ring.len()));
        out.push(VERSION);
        out.push(SAG_SCHEME_BYTE);
        out.extend_from_slice(&(self.ring.len() as u16).to_be_bytes());
        for key in &self.ring {
            out.extend_from_slice(key);
        }
        out.extend_from_slice(&self.c0);
        for s in &self.responses {
            out.extend_from_slice(s);
        }
        out
    }

    /// Serialize the artifact as lowercase hex.
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }

    /// Ordered ring x-only public keys exactly as serialized in the artifact.
    pub fn ring(&self) -> &[[u8; 32]] {
        &self.ring
    }

    /// Number of ring members declared by the gated artifact.
    pub fn ring_len(&self) -> usize {
        self.ring.len()
    }

    /// Initial challenge scalar bytes.
    pub fn c0(&self) -> &[u8; 32] {
        &self.c0
    }

    /// Per-ring response scalar bytes.
    pub fn responses(&self) -> &[[u8; 32]] {
        &self.responses
    }

    /// Parse a lowercase-hex artifact and run the v1 scheme/length gate.
    pub fn from_hex(hex_artifact: &str) -> Result<Self, RingSigError> {
        if !is_canonical_hex(hex_artifact, hex_artifact.len()) || hex_artifact.len() % 2 != 0 {
            return Err(RingSigError::InvalidHex {
                field: "artifact",
                value: hex_artifact.to_string(),
            });
        }
        let bytes = hex::decode(hex_artifact).map_err(|_| RingSigError::InvalidHex {
            field: "artifact",
            value: hex_artifact.to_string(),
        })?;
        Self::from_bytes(&bytes)
    }

    /// Parse a binary artifact and run the v1 scheme/length gate.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RingSigError> {
        if bytes.len() < ARTIFACT_HEADER_LEN {
            return Err(RingSigError::InvalidArtifactLength {
                expected: ARTIFACT_HEADER_LEN,
                actual: bytes.len(),
            });
        }
        if bytes[0] != VERSION {
            return Err(RingSigError::UnsupportedVersion(bytes[0]));
        }
        if bytes[1] != SAG_SCHEME_BYTE {
            return Err(RingSigError::UnsupportedSchemeByte(bytes[1]));
        }

        let declared_n = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        validate_ring_size(declared_n)?;
        let expected = Self::expected_len(declared_n);
        if bytes.len() != expected {
            return Err(RingSigError::InvalidArtifactLength {
                expected,
                actual: bytes.len(),
            });
        }

        let mut offset = ARTIFACT_HEADER_LEN;
        let mut ring = Vec::with_capacity(declared_n);
        for _ in 0..declared_n {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes[offset..offset + 32]);
            ring.push(key);
            offset += 32;
        }

        let mut c0 = [0u8; 32];
        c0.copy_from_slice(&bytes[offset..offset + 32]);
        offset += 32;

        let mut responses = Vec::with_capacity(declared_n);
        for _ in 0..declared_n {
            let mut s = [0u8; 32];
            s.copy_from_slice(&bytes[offset..offset + 32]);
            responses.push(s);
            offset += 32;
        }

        Ok(Self {
            ring,
            c0,
            responses,
        })
    }
}

/// Return true when `s` is lowercase hex of exactly `len` characters.
pub fn is_canonical_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
}

/// Validate and canonicalize an x-only secp256k1 pubkey hex string.
pub fn parse_xonly_pubkey_hex(s: &str) -> Result<[u8; 32], RingSigError> {
    if !is_canonical_hex(s, 64) {
        return Err(RingSigError::InvalidHex {
            field: "x-only pubkey",
            value: s.to_string(),
        });
    }
    let bytes: [u8; 32] = hex::decode(s)
        .map_err(|_| RingSigError::InvalidHex {
            field: "x-only pubkey",
            value: s.to_string(),
        })?
        .try_into()
        .map_err(|_| RingSigError::InvalidHex {
            field: "x-only pubkey",
            value: s.to_string(),
        })?;
    lift_x_even(&bytes).map(|_| bytes)
}

/// Validate a canonical x-only secp256k1 pubkey encoding.
pub fn validate_xonly_pubkey_hex(s: &str) -> Result<(), RingSigError> {
    parse_xonly_pubkey_hex(s).map(|_| ())
}

/// Validate a canonical secp256k1 scalar hex encoding, allowing zero.
pub fn validate_scalar_hex(s: &str) -> Result<(), RingSigError> {
    if !is_canonical_hex(s, 64) {
        return Err(RingSigError::InvalidHex {
            field: "scalar",
            value: s.to_string(),
        });
    }
    let scalar = bytes32_from_hex(s, "scalar")?;
    scalar_from_canonical_bytes(scalar, true, "scalar").map(|_| ())
}

/// Validate v1 ring bounds, BIP-340 x-only lift, and duplicate membership.
pub fn validate_ring_hex(ring: &[String]) -> Result<(), RingSigError> {
    parse_ring_hex(ring).map(|_| ())
}

/// Validate and canonicalize the channel UUID string.
pub fn validate_channel_id(channel_id: &str) -> Result<(), RingSigError> {
    canonical_channel_id(channel_id).map(|_| ())
}

/// Compute the frozen ordered ring hash.
pub fn ring_hash(ring: &[[u8; 32]]) -> Result<[u8; 32], RingSigError> {
    validate_ring_bytes(ring)?;
    let mut preimage = Vec::with_capacity(4 + 32 * ring.len());
    preimage.extend_from_slice(&(ring.len() as u32).to_be_bytes());
    for key in ring {
        preimage.extend_from_slice(key);
    }
    Ok(tagged_hash(RING_TAG, &preimage))
}

/// Compute the frozen unsigned-event `msg_hash`.
pub fn msg_hash(body: &UnsignedEventBody, ring_hash: &[u8; 32]) -> Result<[u8; 32], RingSigError> {
    let ring_hash_hex = hex::encode(ring_hash);
    let filtered_tags: Vec<Vec<String>> = body
        .tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) != Some(RING_SIG_TAG))
        .cloned()
        .collect();
    let preimage = Nip01Preimage((
        0,
        ring_hash_hex,
        body.created_at,
        body.kind,
        filtered_tags,
        body.content.clone(),
    ));
    let json = serde_json::to_vec(&preimage)
        .map_err(|e| RingSigError::InvalidJson(format!("serialize msg_hash preimage: {e}")))?;
    Ok(Sha256::digest(json).into())
}

/// Compute the frozen transcript hash `mu`.
pub fn transcript_hash(
    channel_id: &str,
    ring: &[[u8; 32]],
    body: &UnsignedEventBody,
) -> Result<[u8; 32], RingSigError> {
    let channel_id = canonical_channel_id(channel_id)?;
    let ring_hash = ring_hash(ring)?;
    let msg_hash = msg_hash(body, &ring_hash)?;

    let sid32 = Sha256::digest(SCHEME_ID.as_bytes());
    let mut cid_preimage = Vec::with_capacity(CHANNEL_TAG.len() + channel_id.len());
    cid_preimage.extend_from_slice(CHANNEL_TAG);
    cid_preimage.extend_from_slice(channel_id.as_bytes());
    let cid32 = Sha256::digest(cid_preimage);

    let mut preimage = Vec::with_capacity(128);
    preimage.extend_from_slice(&sid32);
    preimage.extend_from_slice(&cid32);
    preimage.extend_from_slice(&ring_hash);
    preimage.extend_from_slice(&msg_hash);
    Ok(tagged_hash(TRANSCRIPT_TAG, &preimage))
}

/// Sign an unsigned event body with deterministic-with-aux v1 SAG.
///
/// `aux_rand32` MUST be fresh CSPRNG output for each signature. It is mixed into
/// the real nonce and deterministic decoy-response derivation.
pub fn sign_with_aux(
    channel_id: &str,
    body: &UnsignedEventBody,
    ring_hex: &[String],
    signer_index: usize,
    secret_key32: [u8; 32],
    aux_rand32: [u8; 32],
) -> Result<SagArtifact, RingSigError> {
    let ring = parse_ring_hex(ring_hex)?;
    if signer_index >= ring.len() {
        return Err(RingSigError::InvalidRingSize(signer_index));
    }

    let secp = Secp256k1::new();
    let (normalized_secret, signer_xonly) = normalize_secret_key(&secp, secret_key32)?;
    if signer_xonly != ring[signer_index] {
        return Err(RingSigError::SignerIndexMismatch(signer_index));
    }

    let points = lift_ring(&ring)?;
    let mu = transcript_hash(channel_id, &ring, body)?;
    let mut responses = vec![[0u8; 32]; ring.len()];
    let mut challenges = vec![Scalar::ZERO; ring.len()];

    for attempt in 0u32..16 {
        let alpha = derive_alpha(
            &normalized_secret,
            &signer_xonly,
            signer_index,
            &mu,
            aux_rand32,
            attempt,
        )?;
        let alpha_point = PublicKey::from_secret_key(&secp, &alpha);
        challenges[(signer_index + 1) % ring.len()] =
            challenge_hash(&mu, signer_index, &alpha_point.serialize());

        for i in 0..ring.len() {
            if i != signer_index {
                responses[i] = derive_decoy_response(
                    &normalized_secret,
                    &signer_xonly,
                    signer_index,
                    &mu,
                    aux_rand32,
                    attempt,
                    i,
                )
                .to_be_bytes();
            }
        }

        let mut i = (signer_index + 1) % ring.len();
        while i != signer_index {
            let s_i = scalar_from_canonical_bytes(responses[i], true, "response")?;
            let l_i = commitment_point(&secp, s_i, challenges[i], &points[i])?;
            challenges[(i + 1) % ring.len()] = challenge_hash(&mu, i, &l_i.serialize());
            i = (i + 1) % ring.len();
        }

        match close_response(alpha, challenges[signer_index], normalized_secret) {
            Ok(signer_response) if !is_zero(&signer_response) => {
                responses[signer_index] = signer_response;
                return Ok(SagArtifact {
                    ring,
                    c0: challenges[0].to_be_bytes(),
                    responses,
                });
            }
            _ => continue,
        }
    }

    Err(RingSigError::SigningFailed(
        "unable to derive a non-zero signer response",
    ))
}

/// Verify a lowercase-hex v1 SAG artifact.
pub fn verify_artifact_hex(
    channel_id: &str,
    body: &UnsignedEventBody,
    artifact_hex: &str,
    membership: MembershipCheck<'_>,
) -> Result<VerificationResult, RingSigError> {
    let artifact = SagArtifact::from_hex(artifact_hex)?;
    verify_artifact(channel_id, body, &artifact, membership)
}

/// Verify a parsed v1 SAG artifact.
pub fn verify_artifact(
    channel_id: &str,
    body: &UnsignedEventBody,
    artifact: &SagArtifact,
    membership: MembershipCheck<'_>,
) -> Result<VerificationResult, RingSigError> {
    if artifact.responses.len() != artifact.ring.len() {
        return Err(RingSigError::InvalidArtifactLength {
            expected: artifact.ring.len(),
            actual: artifact.responses.len(),
        });
    }
    validate_ring_bytes(&artifact.ring)?;
    let points = lift_ring(&artifact.ring)?;
    let c0 = scalar_from_canonical_bytes(artifact.c0, false, "c0")?;
    let mut c = c0;
    let mu = transcript_hash(channel_id, &artifact.ring, body)?;
    let secp = Secp256k1::new();

    for (i, point) in points.iter().enumerate() {
        let s_i = scalar_from_canonical_bytes(artifact.responses[i], true, "response")?;
        let l_i = commitment_point(&secp, s_i, c, point)?;
        c = challenge_hash(&mu, i, &l_i.serialize());
    }

    if c != c0 {
        return Err(RingSigError::InvalidSignature);
    }

    Ok(VerificationResult {
        crypto_valid: true,
        anonymity_set_state: evaluate_membership(&artifact.ring, membership)?,
    })
}

/// Parse a ring of lowercase-hex x-only keys into byte arrays.
pub fn parse_ring_hex(ring: &[String]) -> Result<Vec<[u8; 32]>, RingSigError> {
    validate_ring_size(ring.len())?;
    let mut parsed = Vec::with_capacity(ring.len());
    for pubkey in ring {
        parsed.push(parse_xonly_pubkey_hex(pubkey)?);
    }
    validate_ring_bytes(&parsed)?;
    Ok(parsed)
}

fn validate_ring_size(len: usize) -> Result<(), RingSigError> {
    if (MIN_RING_SIZE..=MAX_RING_SIZE).contains(&len) {
        Ok(())
    } else {
        Err(RingSigError::InvalidRingSize(len))
    }
}

fn validate_ring_bytes(ring: &[[u8; 32]]) -> Result<(), RingSigError> {
    validate_ring_size(ring.len())?;
    let mut seen = HashSet::with_capacity(ring.len());
    for pubkey in ring {
        lift_x_even(pubkey)?;
        if !seen.insert(*pubkey) {
            return Err(RingSigError::DuplicateRingPubkey(hex::encode(pubkey)));
        }
    }
    Ok(())
}

fn lift_ring(ring: &[[u8; 32]]) -> Result<Vec<PublicKey>, RingSigError> {
    ring.iter().map(lift_x_even).collect()
}

fn lift_x_even(xonly_bytes: &[u8; 32]) -> Result<PublicKey, RingSigError> {
    let xonly = XOnlyPublicKey::from_slice(xonly_bytes)
        .map_err(|_| RingSigError::InvalidPubkey(hex::encode(xonly_bytes)))?;
    Ok(PublicKey::from_x_only_public_key(xonly, Parity::Even))
}

fn bytes32_from_hex(hex_value: &str, field: &'static str) -> Result<[u8; 32], RingSigError> {
    if !is_canonical_hex(hex_value, 64) {
        return Err(RingSigError::InvalidHex {
            field,
            value: hex_value.to_string(),
        });
    }
    hex::decode(hex_value)
        .map_err(|_| RingSigError::InvalidHex {
            field,
            value: hex_value.to_string(),
        })?
        .try_into()
        .map_err(|_| RingSigError::InvalidHex {
            field,
            value: hex_value.to_string(),
        })
}

fn scalar_from_canonical_bytes(
    bytes: [u8; 32],
    allow_zero: bool,
    field: &'static str,
) -> Result<Scalar, RingSigError> {
    let scalar = Scalar::from_be_bytes(bytes).map_err(|_| RingSigError::InvalidScalar(field))?;
    if !allow_zero && is_zero(&bytes) {
        return Err(RingSigError::ZeroScalar(field));
    }
    Ok(scalar)
}

fn secret_from_nonzero_bytes(bytes: [u8; 32]) -> Result<SecretKey, RingSigError> {
    SecretKey::from_slice(&bytes).map_err(|_| RingSigError::InvalidSecretKey)
}

fn normalize_secret_key(
    secp: &Secp256k1<nostr::secp256k1::All>,
    secret_key32: [u8; 32],
) -> Result<(SecretKey, [u8; 32]), RingSigError> {
    let secret = secret_from_nonzero_bytes(secret_key32)?;
    let public = PublicKey::from_secret_key(secp, &secret);
    let (xonly, parity) = public.x_only_public_key();
    let normalized = if parity == Parity::Odd {
        secret.negate()
    } else {
        secret
    };
    Ok((normalized, xonly.serialize()))
}

fn canonical_channel_id(channel_id: &str) -> Result<String, RingSigError> {
    let uuid = uuid::Uuid::parse_str(channel_id)
        .map_err(|_| RingSigError::InvalidChannelId(channel_id.to_string()))?;
    let canonical = uuid.to_string();
    if channel_id.len() == canonical.len() && channel_id.to_ascii_lowercase() == canonical {
        Ok(canonical)
    } else {
        Err(RingSigError::InvalidChannelId(channel_id.to_string()))
    }
}

fn tagged_hash(tag: &str, message: &[u8]) -> [u8; 32] {
    let tag_hash = Sha256::digest(tag.as_bytes());
    let mut hasher = Sha256::new();
    hasher.update(tag_hash);
    hasher.update(tag_hash);
    hasher.update(message);
    hasher.finalize().into()
}

fn hash_to_scalar_clamped(tag: &str, message: &[u8]) -> Scalar {
    let mut reduced = reduce_mod_q(tagged_hash(tag, message));
    if is_zero(&reduced) {
        reduced[31] = 1;
    }
    Scalar::from_be_bytes(reduced).expect("reduced scalar is in range")
}

fn reduce_mod_q(mut value: [u8; 32]) -> [u8; 32] {
    if value >= SECP256K1_ORDER {
        let mut borrow = 0u16;
        for i in (0..32).rev() {
            let sub = SECP256K1_ORDER[i] as u16 + borrow;
            if value[i] as u16 >= sub {
                value[i] = (value[i] as u16 - sub) as u8;
                borrow = 0;
            } else {
                value[i] = (value[i] as u16 + 256 - sub) as u8;
                borrow = 1;
            }
        }
    }
    value
}

fn challenge_hash(mu: &[u8; 32], index: usize, compressed_l: &[u8; 33]) -> Scalar {
    let mut preimage = Vec::with_capacity(32 + 4 + 33);
    preimage.extend_from_slice(mu);
    preimage.extend_from_slice(&(index as u32).to_be_bytes());
    preimage.extend_from_slice(compressed_l);
    hash_to_scalar_clamped(CHALLENGE_TAG, &preimage)
}

fn derive_alpha(
    normalized_secret: &SecretKey,
    signer_xonly: &[u8; 32],
    signer_index: usize,
    mu: &[u8; 32],
    aux_rand32: [u8; 32],
    attempt: u32,
) -> Result<SecretKey, RingSigError> {
    let mut preimage = Vec::with_capacity(32 + 32 + 4 + 32 + 32 + 4);
    preimage.extend_from_slice(&normalized_secret.secret_bytes());
    preimage.extend_from_slice(signer_xonly);
    preimage.extend_from_slice(&(signer_index as u32).to_be_bytes());
    preimage.extend_from_slice(mu);
    preimage.extend_from_slice(&aux_rand32);
    if attempt != 0 {
        preimage.extend_from_slice(&attempt.to_be_bytes());
    }
    let scalar = hash_to_scalar_clamped(NONCE_TAG, &preimage);
    secret_from_nonzero_bytes(scalar.to_be_bytes())
}

fn derive_decoy_response(
    normalized_secret: &SecretKey,
    signer_xonly: &[u8; 32],
    signer_index: usize,
    mu: &[u8; 32],
    aux_rand32: [u8; 32],
    attempt: u32,
    index: usize,
) -> Scalar {
    let mut preimage = Vec::with_capacity(32 + 32 + 4 + 32 + 32 + 4 + 4);
    preimage.extend_from_slice(&normalized_secret.secret_bytes());
    preimage.extend_from_slice(signer_xonly);
    preimage.extend_from_slice(&(signer_index as u32).to_be_bytes());
    preimage.extend_from_slice(mu);
    preimage.extend_from_slice(&aux_rand32);
    preimage.extend_from_slice(&attempt.to_be_bytes());
    preimage.extend_from_slice(&(index as u32).to_be_bytes());
    let reduced = reduce_mod_q(tagged_hash(DECOY_TAG, &preimage));
    Scalar::from_be_bytes(reduced).expect("reduced scalar is in range")
}

fn commitment_point(
    secp: &Secp256k1<nostr::secp256k1::All>,
    s: Scalar,
    c: Scalar,
    point: &PublicKey,
) -> Result<PublicKey, RingSigError> {
    let c_point = point
        .mul_tweak(secp, &c)
        .map_err(|_| RingSigError::InvalidCommitment)?;
    if s == Scalar::ZERO {
        return Ok(c_point);
    }

    let s_secret =
        SecretKey::from_slice(&s.to_be_bytes()).map_err(|_| RingSigError::InvalidScalar("s"))?;
    let s_point = PublicKey::from_secret_key(secp, &s_secret);
    s_point
        .combine(&c_point)
        .map_err(|_| RingSigError::InvalidCommitment)
}

fn close_response(
    alpha: SecretKey,
    c_pi: Scalar,
    normalized_secret: SecretKey,
) -> Result<[u8; 32], RingSigError> {
    let cx = normalized_secret
        .mul_tweak(&c_pi)
        .map_err(|_| RingSigError::InvalidScalar("challenge"))?;
    let neg_cx = cx.negate();
    alpha
        .add_tweak(&Scalar::from(neg_cx))
        .map(|s| s.secret_bytes())
        .map_err(|_| RingSigError::ZeroScalar("signer response"))
}

fn evaluate_membership(
    ring: &[[u8; 32]],
    membership: MembershipCheck<'_>,
) -> Result<AnonymitySetState, RingSigError> {
    match membership {
        MembershipCheck::Unchecked => Ok(AnonymitySetState::UncheckedOrStale),
        MembershipCheck::CurrentMembersOnly(members) => {
            let mut member_set = HashSet::with_capacity(members.len());
            for member in members {
                member_set.insert(parse_xonly_pubkey_hex(member)?);
            }
            if ring.iter().all(|key| member_set.contains(key)) {
                Ok(AnonymitySetState::UncheckedOrStale)
            } else {
                Ok(AnonymitySetState::NonMember)
            }
        }
        MembershipCheck::MembersAtCreatedAt(members) => {
            let mut member_set = HashSet::with_capacity(members.len());
            for member in members {
                member_set.insert(parse_xonly_pubkey_hex(member)?);
            }
            if ring.iter().all(|key| member_set.contains(key)) {
                Ok(AnonymitySetState::MemberVerified)
            } else {
                Ok(AnonymitySetState::NonMember)
            }
        }
    }
}

fn is_zero(bytes: &[u8; 32]) -> bool {
    bytes.iter().all(|b| *b == 0)
}

#[derive(Serialize)]
struct Nip01Preimage((u8, String, u64, u64, Vec<Vec<String>>, String));

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(bytes: u8) -> [u8; 32] {
        let mut sk = [0u8; 32];
        sk[31] = bytes;
        sk
    }

    fn xonly_hex_for_secret(bytes: u8) -> String {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&secret(bytes)).expect("valid test key");
        let pk = PublicKey::from_secret_key(&secp, &sk);
        let (xonly, _) = pk.x_only_public_key();
        hex::encode(xonly.serialize())
    }

    fn body() -> UnsignedEventBody {
        UnsignedEventBody {
            created_at: 1_782_250_000,
            kind: 9,
            tags: vec![
                vec![
                    "h".to_string(),
                    "2114f4bc-5130-4816-83da-acbfcc65243b".to_string(),
                ],
                vec![RING_SIG_TAG.to_string(), "ignored".to_string()],
            ],
            content: "anonymous hello".to_string(),
        }
    }

    fn ring() -> Vec<String> {
        vec![
            xonly_hex_for_secret(1),
            xonly_hex_for_secret(2),
            xonly_hex_for_secret(3),
        ]
    }

    #[test]
    fn hex_validation_is_lowercase_and_fixed_width() {
        let ok = xonly_hex_for_secret(1);
        validate_xonly_pubkey_hex(&ok).expect("valid");
        validate_scalar_hex(&"a".repeat(64)).expect("valid scalar shape and range");

        assert!(validate_xonly_pubkey_hex(&ok.to_uppercase()).is_err());
        assert!(validate_scalar_hex(&"a".repeat(63)).is_err());
    }

    #[test]
    fn ring_validation_rejects_duplicates_and_small_rings() {
        let a = xonly_hex_for_secret(1);
        let b = xonly_hex_for_secret(2);
        validate_ring_hex(&[a.clone(), b]).expect("valid ring");

        assert!(matches!(
            validate_ring_hex(std::slice::from_ref(&a)),
            Err(RingSigError::InvalidRingSize(1))
        ));
        assert!(matches!(
            validate_ring_hex(&[a.clone(), a]),
            Err(RingSigError::DuplicateRingPubkey(_))
        ));
    }

    #[test]
    fn sign_and_verify_valid_artifact() {
        let ring = ring();
        let artifact = sign_with_aux(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &ring,
            1,
            secret(2),
            [7u8; 32],
        )
        .expect("sign");

        let result = verify_artifact(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &artifact,
            MembershipCheck::MembersAtCreatedAt(&ring),
        )
        .expect("verify");
        assert_eq!(
            result,
            VerificationResult {
                crypto_valid: true,
                anonymity_set_state: AnonymitySetState::MemberVerified,
            }
        );
    }

    #[test]
    fn verifier_rejects_bound_context_changes() {
        let ring = ring();
        let artifact = sign_with_aux(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &ring,
            1,
            secret(2),
            [7u8; 32],
        )
        .expect("sign");

        let mut altered_body = body();
        altered_body.content = "tampered hello".to_string();
        assert!(matches!(
            verify_artifact(
                "2114f4bc-5130-4816-83da-acbfcc65243b",
                &altered_body,
                &artifact,
                MembershipCheck::Unchecked,
            ),
            Err(RingSigError::InvalidSignature)
        ));

        assert!(matches!(
            verify_artifact(
                "00000000-0000-4000-8000-000000000001",
                &body(),
                &artifact,
                MembershipCheck::Unchecked,
            ),
            Err(RingSigError::InvalidSignature)
        ));
    }

    #[test]
    fn parser_rejects_wrong_scheme_and_malformed_scalars() {
        let ring = ring();
        let artifact = sign_with_aux(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &ring,
            1,
            secret(2),
            [7u8; 32],
        )
        .expect("sign");

        let mut bytes = artifact.to_bytes();
        bytes[1] = RESERVED_BLSAG_SCHEME_BYTE;
        assert!(matches!(
            SagArtifact::from_bytes(&bytes),
            Err(RingSigError::UnsupportedSchemeByte(
                RESERVED_BLSAG_SCHEME_BYTE
            ))
        ));

        let mut bytes = artifact.to_bytes();
        let c0_offset = ARTIFACT_HEADER_LEN + 32 * artifact.ring_len();
        bytes[c0_offset..c0_offset + 32].copy_from_slice(&SECP256K1_ORDER);
        let parsed = SagArtifact::from_bytes(&bytes).expect("structure ok");
        assert!(matches!(
            verify_artifact(
                "2114f4bc-5130-4816-83da-acbfcc65243b",
                &body(),
                &parsed,
                MembershipCheck::Unchecked,
            ),
            Err(RingSigError::InvalidScalar("c0"))
        ));
    }

    #[test]
    fn membership_state_is_not_collapsed() {
        let ring = ring();
        let artifact = sign_with_aux(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &ring,
            1,
            secret(2),
            [7u8; 32],
        )
        .expect("sign");

        let unchecked = verify_artifact(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &artifact,
            MembershipCheck::Unchecked,
        )
        .expect("verify");
        assert_eq!(
            unchecked.anonymity_set_state,
            AnonymitySetState::UncheckedOrStale
        );

        let missing_member = vec![ring[0].clone(), ring[1].clone()];
        let non_member = verify_artifact(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &artifact,
            MembershipCheck::CurrentMembersOnly(&missing_member),
        )
        .expect("verify");
        assert_eq!(non_member.anonymity_set_state, AnonymitySetState::NonMember);

        let all_current_only = verify_artifact(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &artifact,
            MembershipCheck::CurrentMembersOnly(&ring),
        )
        .expect("verify");
        assert_eq!(
            all_current_only.anonymity_set_state,
            AnonymitySetState::UncheckedOrStale
        );

        let all_at_created_at = verify_artifact(
            "2114f4bc-5130-4816-83da-acbfcc65243b",
            &body(),
            &artifact,
            MembershipCheck::MembersAtCreatedAt(&ring),
        )
        .expect("verify");
        assert_eq!(
            all_at_created_at.anonymity_set_state,
            AnonymitySetState::MemberVerified
        );
    }
}
