//! Test-vector schema and bundled v1 matrix vectors.
//!
//! The bundled vectors keep the expected negative-test matrix explicit,
//! concrete, and machine-checkable.

use serde::Deserialize;

use crate::{
    validate_channel_id, validate_ring_hex, verify_artifact_hex, AnonymitySetState,
    MembershipCheck, RingSigError, UnsignedEventBody, SCHEME_ID, VERSION,
};

/// Bundled v1 vector set.
pub const BUNDLED_VECTORS_JSON: &str = include_str!("../test_vectors/v1_sag.json");

/// A collection of v1 SAG test-vector cases.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct VectorSet {
    /// Schema version for this JSON fixture format.
    pub schema_version: u8,
    /// Scheme id these vectors target.
    pub scheme_id: String,
    /// Test cases.
    pub cases: Vec<VectorCase>,
}

/// Expected test-vector outcome.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedVerdict {
    /// A complete vector should verify.
    Accept,
    /// The vector is expected to be rejected by a v1 verifier.
    Reject,
}

/// One ring-signature test-vector case.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct VectorCase {
    /// Unique test name.
    pub name: String,
    /// Human-readable purpose.
    pub description: String,
    /// Expected verifier outcome.
    pub expected: ExpectedVerdict,
    /// Expected rejection reason for negative vectors.
    #[serde(default)]
    pub expected_error: Option<String>,
    /// Channel id bound by the vector.
    pub channel_id: String,
    /// Event timestamp to use in the `msg_hash` preimage.
    pub created_at: u64,
    /// Event kind to use in the `msg_hash` preimage.
    pub kind: u64,
    /// Event tags to use in the `msg_hash` preimage.
    pub tags: Vec<Vec<String>>,
    /// Message content to bind.
    pub content: String,
    /// Ordered x-only pubkey ring.
    pub ring: Vec<String>,
    /// Serialized artifact hex.
    pub artifact_hex: String,
}

/// Parse a vector set from JSON.
pub fn parse_vector_set(json: &str) -> Result<VectorSet, RingSigError> {
    serde_json::from_str(json).map_err(|e| RingSigError::InvalidJson(e.to_string()))
}

/// Load the bundled vector set.
pub fn bundled_vector_set() -> Result<VectorSet, RingSigError> {
    parse_vector_set(BUNDLED_VECTORS_JSON)
}

/// Validate schema-level invariants for a vector set.
pub fn validate_vector_set(set: &VectorSet) -> Result<(), RingSigError> {
    if set.scheme_id != SCHEME_ID {
        return Err(RingSigError::UnsupportedScheme(set.scheme_id.clone()));
    }
    if set.schema_version != VERSION {
        return Err(RingSigError::UnsupportedScheme(format!(
            "schema version {}",
            set.schema_version
        )));
    }

    let mut names = std::collections::HashSet::with_capacity(set.cases.len());
    for case in &set.cases {
        if !names.insert(case.name.as_str()) {
            return Err(RingSigError::InvalidJson(format!(
                "duplicate case name {:?}",
                case.name
            )));
        }
        if matches!(case.expected, ExpectedVerdict::Reject) && case.expected_error.is_none() {
            return Err(RingSigError::InvalidJson(format!(
                "negative case {:?} missing expected_error",
                case.name
            )));
        }
        validate_channel_id(&case.channel_id)?;
        match case.expected_error.as_deref() {
            Some("duplicate-ring-pubkey") => {
                if !matches!(
                    validate_ring_hex(&case.ring),
                    Err(RingSigError::DuplicateRingPubkey(_))
                ) {
                    return Err(RingSigError::InvalidJson(format!(
                        "case {:?} must contain a duplicate ring key",
                        case.name
                    )));
                }
            }
            Some("invalid-pubkey") => {
                if !matches!(
                    validate_ring_hex(&case.ring),
                    Err(RingSigError::InvalidHex {
                        field: "x-only pubkey",
                        ..
                    }) | Err(RingSigError::InvalidPubkey(_))
                ) {
                    return Err(RingSigError::InvalidJson(format!(
                        "case {:?} must contain an invalid pubkey",
                        case.name
                    )));
                }
            }
            _ => validate_ring_hex(&case.ring)?,
        }
        if !crate::is_canonical_hex(&case.artifact_hex, case.artifact_hex.len())
            || case.artifact_hex.len() % 2 != 0
        {
            return Err(RingSigError::InvalidHex {
                field: "artifact",
                value: case.artifact_hex.clone(),
            });
        }
        hex::decode(&case.artifact_hex).map_err(|_| RingSigError::InvalidHex {
            field: "artifact",
            value: case.artifact_hex.clone(),
        })?;
    }
    Ok(())
}

fn event_body(case: &VectorCase) -> UnsignedEventBody {
    UnsignedEventBody {
        created_at: case.created_at,
        kind: case.kind,
        tags: case.tags.clone(),
        content: case.content.clone(),
    }
}

fn assert_expected_rejection(case: &VectorCase, err: RingSigError) {
    match case.expected_error.as_deref() {
        Some("duplicate-ring-pubkey") => {
            assert!(matches!(err, RingSigError::DuplicateRingPubkey(_)))
        }
        Some("invalid-pubkey") => assert!(matches!(err, RingSigError::InvalidPubkey(_))),
        Some("invalid-signature") => assert!(matches!(err, RingSigError::InvalidSignature)),
        Some("invalid-scalar") => assert!(matches!(err, RingSigError::InvalidScalar(_))),
        Some("unsupported-scheme") => {
            assert!(matches!(err, RingSigError::UnsupportedSchemeByte(_)))
        }
        other => panic!(
            "case {:?} has unsupported expected_error {other:?}: {err}",
            case.name
        ),
    }
}

/// Verify all concrete bundled vectors against the implementation.
pub fn verify_vector_set(set: &VectorSet) -> Result<(), RingSigError> {
    validate_vector_set(set)?;

    for case in &set.cases {
        let body = event_body(case);
        let membership = match case.expected {
            ExpectedVerdict::Accept => MembershipCheck::MembersAtCreatedAt(&case.ring),
            ExpectedVerdict::Reject => MembershipCheck::Unchecked,
        };
        let result = verify_artifact_hex(&case.channel_id, &body, &case.artifact_hex, membership);
        match (&case.expected, result) {
            (ExpectedVerdict::Accept, Ok(result)) => {
                if result.anonymity_set_state != AnonymitySetState::MemberVerified {
                    return Err(RingSigError::InvalidJson(format!(
                        "case {:?} did not verify as member-backed",
                        case.name
                    )));
                }
            }
            (ExpectedVerdict::Accept, Err(err)) => return Err(err),
            (ExpectedVerdict::Reject, Ok(_)) => {
                return Err(RingSigError::InvalidJson(format!(
                    "negative case {:?} unexpectedly verified",
                    case.name
                )));
            }
            (ExpectedVerdict::Reject, Err(err)) => assert_expected_rejection(case, err),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_vectors_parse_and_validate() {
        let set = bundled_vector_set().expect("parse bundled vectors");
        validate_vector_set(&set).expect("validate bundled vectors");
    }

    #[test]
    fn bundled_vectors_execute_against_verifier() {
        let set = bundled_vector_set().expect("parse bundled vectors");
        verify_vector_set(&set).expect("verify bundled vectors");
    }

    #[test]
    fn bundled_vectors_cover_locked_negative_matrix() {
        let set = bundled_vector_set().expect("parse bundled vectors");
        let names: std::collections::HashSet<_> =
            set.cases.iter().map(|case| case.name.as_str()).collect();

        for required in [
            "valid",
            "reject-altered-content",
            "reject-altered-channel",
            "reject-reordered-ring",
            "reject-duplicate-key",
            "reject-invalid-pubkey",
            "reject-wrong-scheme",
            "reject-malformed-scalar",
        ] {
            assert!(names.contains(required), "missing vector {required}");
        }
    }
}
