use nostr::{Event, EventId, JsonUtil, Keys, PublicKey};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    events,
    managed_agents::{find_managed_agent_mut, load_managed_agents, ManagedAgentRecord},
    models::{
        FeedItemInfo, FeedMeta, FeedResponse, FeedSections, ForumMessageInfo, ForumPostsResponse,
        ForumThreadReplyInfo, ForumThreadResponse, SearchResponse, SendChannelMessageResponse,
        ThreadSummary,
    },
    nostr_convert,
    relay::{query_relay, submit_event, submit_event_with_keys},
};

// ── Reads (pure-nostr) ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_feed(
    since: Option<i64>,
    limit: Option<u32>,
    types: Option<String>,
    state: State<'_, AppState>,
) -> Result<FeedResponse, String> {
    let cap = limit.unwrap_or(50).min(100);

    // Parse types filter — if absent, run all sub-queries.
    // Comma-separated: e.g. "mentions,needs_action".
    let want_mentions = types
        .as_deref()
        .map(|t| t.split(',').any(|s| s.trim() == "mentions"))
        .unwrap_or(true);
    let want_needs_action = types
        .as_deref()
        .map(|t| t.split(',').any(|s| s.trim() == "needs_action"))
        .unwrap_or(true);

    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Mentions: messages that reference me via #p.
    let mut mention_filter = serde_json::json!({
        "kinds": [9, 40002, 40009, 1, 45001, 45003],
        "#p": [my_pubkey],
        "limit": cap,
    });
    if let Some(s) = since {
        mention_filter["since"] = serde_json::json!(s);
    }
    // Needs-action: workflow approval-request events sent to me.
    let mut approval_filter = serde_json::json!({
        "kinds": [46010, 46011, 46012],
        "#p": [my_pubkey],
        "limit": 20,
    });
    if let Some(s) = since {
        approval_filter["since"] = serde_json::json!(s);
    }

    let mention_events = if want_mentions {
        query_relay(&state, &[mention_filter])
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let approval_events = if want_needs_action {
        query_relay(&state, &[approval_filter])
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mentions: Vec<FeedItemInfo> = mention_events
        .iter()
        .map(|ev| feed_item_from_event(ev, "mentions"))
        .collect();
    let needs_action: Vec<FeedItemInfo> = approval_events
        .iter()
        .map(|ev| feed_item_from_event(ev, "needs_action"))
        .collect();

    let total = (mentions.len() + needs_action.len()) as u64;
    Ok(FeedResponse {
        feed: FeedSections {
            mentions,
            needs_action,
            activity: Vec::new(),
            agent_activity: Vec::new(),
        },
        meta: FeedMeta {
            since: since.unwrap_or(0),
            total,
            generated_at: chrono::Utc::now().timestamp(),
        },
    })
}

#[tauri::command]
pub async fn search_messages(
    q: String,
    limit: Option<u32>,
    channel_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let cap = limit.unwrap_or(20).min(100);
    let mut filter = serde_json::Map::new();
    filter.insert(
        "kinds".to_string(),
        serde_json::json!([9, 40002, 40009, 45001, 45003]),
    );
    filter.insert("search".to_string(), serde_json::json!(q.trim()));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(cid) = channel_id {
        filter.insert("#h".to_string(), serde_json::json!([cid]));
    }

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;
    Ok(nostr_convert::search_response_from_events(&events))
}

#[tauri::command]
pub async fn get_forum_posts(
    channel_id: String,
    limit: Option<u32>,
    before: Option<i64>,
    state: State<'_, AppState>,
) -> Result<ForumPostsResponse, String> {
    let cap = limit.unwrap_or(20).min(100);
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".to_string(), serde_json::json!([45001]));
    filter.insert("#h".to_string(), serde_json::json!([channel_id.clone()]));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(t) = before {
        filter.insert("until".to_string(), serde_json::json!(t));
    }

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;
    let messages: Vec<ForumMessageInfo> = events
        .iter()
        .map(|ev| forum_message_from_event(ev, &channel_id))
        .collect();

    let next_cursor = messages.last().map(|m| m.created_at);
    Ok(ForumPostsResponse {
        messages,
        next_cursor,
    })
}

#[tauri::command]
pub async fn get_forum_thread(
    channel_id: String,
    event_id: String,
    limit: Option<u32>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<ForumThreadResponse, String> {
    let _ = (limit, cursor);
    // Two filters: the root event itself, plus any reply (kinds 9/45003)
    // that references it via #e.
    let events = query_relay(
        &state,
        &[
            serde_json::json!({ "ids": [event_id.clone()], "kinds": [9, 40002, 40009, 45001, 45003] }),
            serde_json::json!({
                "kinds": [9, 45003],
                "#e": [event_id.clone()],
                "#h": [channel_id.clone()],
            }),
        ],
    )
    .await?;

    let mut root: Option<ForumMessageInfo> = None;
    let mut replies: Vec<ForumThreadReplyInfo> = Vec::new();
    for ev in &events {
        if ev.id.to_hex() == event_id {
            root = Some(forum_message_from_event(ev, &channel_id));
        } else {
            replies.push(forum_reply_from_event(ev, &channel_id, &event_id));
        }
    }
    let total_replies = replies.len() as u32;

    let root = root.ok_or_else(|| "forum thread root event not found".to_string())?;
    Ok(ForumThreadResponse {
        root,
        replies,
        total_replies,
        next_cursor: None,
    })
}

#[tauri::command]
pub async fn get_event(event_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "ids": [event_id],
            "kinds": [0, 1, 3, 5, 7, 9, 30078, 40002, 40003, 40008, 40009, 40099, 40100, 45001, 45003],
            "limit": 1
        })],
    )
    .await?;

    let ev = events
        .first()
        .ok_or_else(|| "event not found".to_string())?;
    serde_json::to_string(ev).map_err(|e| format!("serialize event: {e}"))
}

// ── Writes ──────────────────────────────────────────────────────────────────

/// Fetch a parent event and extract the thread root from its NIP-10 e-tags.
async fn resolve_thread_ref(
    parent_event_id: &str,
    state: &AppState,
) -> Result<events::ThreadRef, String> {
    let parent_eid =
        EventId::from_hex(parent_event_id).map_err(|e| format!("invalid parent event ID: {e}"))?;

    let evs = query_relay(
        state,
        &[serde_json::json!({
            "ids": [parent_event_id],
            "kinds": [9, 40002, 40009, 45001, 45003],
            "limit": 1
        })],
    )
    .await?;

    let parent = evs
        .first()
        .ok_or_else(|| "parent event not found".to_string())?;

    // Walk tags looking for NIP-10 root/reply markers.
    let (mut root, mut reply) = (None, None);
    for tag in parent.tags.iter() {
        let s = tag.as_slice();
        if s.len() >= 4 && s[0] == "e" {
            match s[3].as_str() {
                "root" => root = Some(s[1].clone()),
                "reply" => reply = Some(s[1].clone()),
                _ => {}
            }
        }
    }
    let root_hex = root.or(reply);

    let root_eid = match root_hex {
        Some(hex) if hex != parent_event_id => {
            EventId::from_hex(&hex).map_err(|e| format!("invalid root event ID: {e}"))?
        }
        _ => parent_eid,
    };

    Ok(events::ThreadRef {
        root_event_id: root_eid,
        parent_event_id: parent_eid,
    })
}

fn single_ring_sig_artifact(ev: &Event) -> Result<&str, String> {
    let ring_sig_tags: Vec<_> = ev
        .tags
        .iter()
        .filter(|tag| {
            tag.as_slice().first().map(String::as_str) == Some(buzz_ring_sig_pkg::RING_SIG_TAG)
        })
        .collect();

    if ring_sig_tags.len() != 1 {
        return Err(format!(
            "kind:40009 requires exactly one {} tag",
            buzz_ring_sig_pkg::RING_SIG_TAG
        ));
    }

    let parts = ring_sig_tags[0].as_slice();
    match (parts.get(1), parts.get(2).map(String::as_str)) {
        (Some(artifact_hex), Some(buzz_ring_sig_pkg::SCHEME_ID)) if !artifact_hex.is_empty() => {
            Ok(artifact_hex)
        }
        _ => Err(format!(
            "kind:40009 {} tag must use marker {}",
            buzz_ring_sig_pkg::RING_SIG_TAG,
            buzz_ring_sig_pkg::SCHEME_ID
        )),
    }
}

fn canonical_channel_id(value: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(value)
        .map(|uuid| uuid.to_string())
        .map_err(|error| format!("invalid channel UUID: {error}"))
}

fn single_channel_h_tag(ev: &Event) -> Result<&str, String> {
    let h_tags: Vec<_> = ev
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
        .collect();

    if h_tags.len() != 1 {
        return Err("kind:40009 requires exactly one channel h tag".to_string());
    }

    h_tags[0]
        .as_slice()
        .get(1)
        .map(String::as_str)
        .filter(|channel_id| !channel_id.is_empty())
        .ok_or_else(|| "kind:40009 malformed channel h tag".to_string())
}

fn bound_anonymous_channel_id(
    ev: &Event,
    supplied_channel_id: Option<&str>,
) -> Result<String, String> {
    let raw_event_channel_id = single_channel_h_tag(ev)?;
    let event_channel_id = canonical_channel_id(&raw_event_channel_id)
        .map_err(|error| format!("kind:40009 malformed channel h tag: {error}"))?;

    if let Some(supplied_channel_id) = supplied_channel_id {
        let supplied_channel_id = canonical_channel_id(supplied_channel_id)
            .map_err(|error| format!("supplied channelId is malformed: {error}"))?;
        if supplied_channel_id != event_channel_id {
            return Err("supplied channelId does not match event h tag".to_string());
        }
    }

    Ok(event_channel_id)
}

fn unsigned_body_from_event(ev: &Event) -> buzz_ring_sig_pkg::UnsignedEventBody {
    buzz_ring_sig_pkg::UnsignedEventBody {
        created_at: ev.created_at.as_secs(),
        kind: ev.kind.as_u16() as u64,
        tags: tags_to_vec(ev),
        content: ev.content.clone(),
    }
}

fn anonymity_set_state_name(state: buzz_ring_sig_pkg::AnonymitySetState) -> &'static str {
    match state {
        buzz_ring_sig_pkg::AnonymitySetState::MemberVerified => "member_verified",
        buzz_ring_sig_pkg::AnonymitySetState::UncheckedOrStale => "unchecked_or_stale",
        buzz_ring_sig_pkg::AnonymitySetState::NonMember => "non_member",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnonymousVerificationInfo {
    pub crypto_valid: bool,
    pub anonymity_set_state: String,
    pub ring_members: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn invalid_anonymous_verification(
    ring_members: Vec<String>,
    error: impl Into<String>,
) -> AnonymousVerificationInfo {
    AnonymousVerificationInfo {
        crypto_valid: false,
        anonymity_set_state: "invalid".to_string(),
        ring_members,
        error: Some(error.into()),
    }
}

fn anonymous_ring_is_canonical_sorted(ring: &[[u8; 32]]) -> bool {
    ring.windows(2).all(|window| window[0] < window[1])
}

fn canonical_pubkey_hex(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let canonical = PublicKey::from_hex(trimmed)
        .map(|pubkey| pubkey.to_hex().to_ascii_lowercase())
        .map_err(|error| format!("invalid pubkey: {error}"))?;
    if trimmed != canonical {
        return Err("pubkey must be canonical lowercase 32-byte x-only hex".to_string());
    }
    Ok(canonical)
}

fn anonymous_ring_pubkeys(
    current_member_pubkeys: &[String],
    selected_ring_pubkeys: Option<&[String]>,
    signer_pubkey: &str,
) -> Result<Vec<String>, String> {
    let current_members: std::collections::HashSet<String> = current_member_pubkeys
        .iter()
        .map(|pubkey| canonical_pubkey_hex(pubkey))
        .collect::<Result<_, _>>()?;

    let supplied_ring = selected_ring_pubkeys.is_some();
    let ring_source = match selected_ring_pubkeys {
        Some(selected_ring_pubkeys) => {
            if selected_ring_pubkeys.is_empty() {
                return Err("anonymous ring cannot be empty".to_string());
            }
            selected_ring_pubkeys
        }
        None => current_member_pubkeys,
    };

    let mut seen = std::collections::HashSet::new();
    let mut ring = Vec::with_capacity(ring_source.len());
    for pubkey in ring_source {
        let pubkey = canonical_pubkey_hex(pubkey)?;
        if !seen.insert(pubkey.clone()) {
            return Err("anonymous ring cannot contain duplicate pubkeys".to_string());
        }
        if !current_members.contains(&pubkey) {
            return Err("anonymous ring must contain only current channel members".to_string());
        }
        ring.push(pubkey);
    }

    if ring.len() < buzz_ring_sig_pkg::MIN_RING_SIZE {
        return Err(format!(
            "anonymous ring must contain at least {} distinct current members",
            buzz_ring_sig_pkg::MIN_RING_SIZE
        ));
    }
    let mut sorted_ring = ring.clone();
    sorted_ring.sort();
    if supplied_ring {
        if ring != sorted_ring {
            return Err("anonymous ring must be sorted by canonical pubkey".to_string());
        }
    } else {
        ring = sorted_ring;
    }
    if !ring.iter().any(|pubkey| pubkey == signer_pubkey) {
        return Err("anonymous ring must include the active identity".to_string());
    }
    buzz_ring_sig_pkg::validate_ring_hex(&ring)
        .map_err(|error| format!("invalid anonymous ring: {error}"))?;
    Ok(ring)
}

async fn fetch_current_member_pubkeys(
    channel_id: &str,
    state: &AppState,
) -> Result<Vec<String>, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_NIP29_GROUP_MEMBERS],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    let response = events
        .first()
        .map(nostr_convert::channel_members_from_event)
        .transpose()?
        .ok_or_else(|| "channel members not found".to_string())?;

    Ok(response
        .members
        .iter()
        .map(|member| member.pubkey.to_ascii_lowercase())
        .collect())
}

#[allow(clippy::too_many_arguments)]
async fn build_signed_anonymous_channel_message(
    channel_uuid: uuid::Uuid,
    content: &str,
    thread_ref: Option<&events::ThreadRef>,
    mention_refs: &[&str],
    media: &[Vec<String>],
    emoji: &[Vec<String>],
    mention_refs_only: &[Vec<String>],
    selected_ring_pubkeys: Option<&[String]>,
    state: &AppState,
) -> Result<(nostr::EventBuilder, Keys, u64), String> {
    let channel_id = channel_uuid.to_string();
    let signer_keys = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let current_member_pubkeys = fetch_current_member_pubkeys(&channel_id, state).await?;
    let signer_pubkey = signer_keys.public_key().to_hex().to_ascii_lowercase();
    let ring_pubkeys = anonymous_ring_pubkeys(
        &current_member_pubkeys,
        selected_ring_pubkeys,
        &signer_pubkey,
    )?;
    let signer_index = ring_pubkeys
        .iter()
        .position(|pubkey| pubkey == &signer_pubkey)
        .ok_or_else(|| {
            "anonymous send requires the active identity to be a channel member".to_string()
        })?;

    let created_at = chrono::Utc::now().timestamp() as u64;
    let body = events::build_anonymous_message_unsigned_body(
        channel_uuid,
        content,
        created_at,
        thread_ref,
        mention_refs,
        media,
        emoji,
        mention_refs_only,
    )?;
    let secret_key32: [u8; 32] = signer_keys
        .secret_key()
        .as_secret_bytes()
        .try_into()
        .map_err(|_| "active identity secret key must be 32 bytes".to_string())?;
    let aux_rand32: [u8; 32] = rand::random();
    let artifact = buzz_ring_sig_pkg::sign_with_aux(
        &channel_id,
        &body,
        &ring_pubkeys,
        signer_index,
        secret_key32,
        aux_rand32,
    )
    .map_err(|error| format!("failed to sign anonymous message: {error}"))?;

    let builder = events::build_anonymous_message(
        channel_uuid,
        content,
        created_at,
        thread_ref,
        mention_refs,
        media,
        emoji,
        mention_refs_only,
        &artifact.to_hex(),
    )?;

    let posting_keys = Keys::generate();

    Ok((builder, posting_keys, created_at))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_channel_message(
    channel_id: String,
    content: String,
    parent_event_id: Option<String>,
    media_tags: Option<Vec<Vec<String>>>,
    emoji_tags: Option<Vec<Vec<String>>>,
    mention_tags: Option<Vec<Vec<String>>>,
    mention_pubkeys: Option<Vec<String>>,
    kind: Option<u32>,
    anonymous_ring_pubkeys: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let mentions = mention_pubkeys.unwrap_or_default();
    let mention_refs: Vec<&str> = mentions.iter().map(|s| s.as_str()).collect();
    let media = media_tags.unwrap_or_default();
    let emoji = emoji_tags.unwrap_or_default();
    let mention_refs_only = mention_tags.unwrap_or_default();
    let kind_num = kind.unwrap_or(buzz_core_pkg::kind::KIND_STREAM_MESSAGE);

    let mut resolved_root: Option<String> = None;
    let mut explicit_keys: Option<Keys> = None;
    let mut response_created_at = chrono::Utc::now().timestamp();

    let builder = match kind_num {
        buzz_core_pkg::kind::KIND_FORUM_POST => events::build_forum_post(
            channel_uuid,
            content.trim(),
            &mention_refs,
            &media,
            &mention_refs_only,
        )?,
        buzz_core_pkg::kind::KIND_FORUM_COMMENT => {
            let parent_id = parent_event_id
                .as_deref()
                .ok_or("forum comment requires parent_event_id")?;
            let thread_ref = resolve_thread_ref(parent_id, &state).await?;
            resolved_root = Some(thread_ref.root_event_id.to_hex());
            events::build_forum_comment(
                channel_uuid,
                content.trim(),
                &thread_ref,
                &mention_refs,
                &media,
                &mention_refs_only,
            )?
        }
        buzz_core_pkg::kind::KIND_STREAM_MESSAGE_ANON => {
            let thread_ref = match parent_event_id.as_deref() {
                Some(pid) => {
                    let tr = resolve_thread_ref(pid, &state).await?;
                    resolved_root = Some(tr.root_event_id.to_hex());
                    Some(tr)
                }
                None => None,
            };
            let (builder, keys, created_at) = build_signed_anonymous_channel_message(
                channel_uuid,
                content.trim(),
                thread_ref.as_ref(),
                &mention_refs,
                &media,
                &emoji,
                &mention_refs_only,
                anonymous_ring_pubkeys.as_deref(),
                &state,
            )
            .await?;
            explicit_keys = Some(keys);
            response_created_at = created_at as i64;
            builder
        }
        _ => {
            let thread_ref = match parent_event_id.as_deref() {
                Some(pid) => {
                    let tr = resolve_thread_ref(pid, &state).await?;
                    resolved_root = Some(tr.root_event_id.to_hex());
                    Some(tr)
                }
                None => None,
            };
            events::build_message(
                channel_uuid,
                content.trim(),
                thread_ref.as_ref(),
                &mention_refs,
                &media,
                &emoji,
                &mention_refs_only,
            )?
        }
    };

    let result = if let Some(keys) = explicit_keys.as_ref() {
        submit_event_with_keys(builder, &state, keys, None).await?
    } else {
        submit_event(builder, &state).await?
    };

    let depth = match (&parent_event_id, &resolved_root) {
        (None, _) => 0,
        (Some(pid), Some(root)) if pid == root => 1,
        (Some(_), Some(_)) => 2,
        (Some(_), None) => 1,
    };

    Ok(SendChannelMessageResponse {
        event_id: result.event_id,
        root_event_id: resolved_root,
        parent_event_id,
        depth,
        created_at: response_created_at,
    })
}

#[tauri::command]
pub fn verify_anonymous_event(
    event_json: String,
    channel_id: Option<String>,
    member_pubkeys: Option<Vec<String>>,
) -> Result<AnonymousVerificationInfo, String> {
    let event =
        Event::from_json(event_json).map_err(|error| format!("invalid event json: {error}"))?;
    if event.kind.as_u16() as u32 != buzz_core_pkg::kind::KIND_STREAM_MESSAGE_ANON {
        return Ok(invalid_anonymous_verification(
            Vec::new(),
            "event is not kind:40009",
        ));
    }

    let artifact_hex = match single_ring_sig_artifact(&event) {
        Ok(artifact_hex) => artifact_hex,
        Err(error) => return Ok(invalid_anonymous_verification(Vec::new(), error)),
    };
    let channel_id = match bound_anonymous_channel_id(&event, channel_id.as_deref()) {
        Ok(channel_id) => channel_id,
        Err(error) => return Ok(invalid_anonymous_verification(Vec::new(), error)),
    };
    let artifact = match buzz_ring_sig_pkg::SagArtifact::from_hex(artifact_hex) {
        Ok(artifact) => artifact,
        Err(error) => {
            return Ok(invalid_anonymous_verification(
                Vec::new(),
                format!("invalid ring signature artifact: {error}"),
            ));
        }
    };
    let ring_members: Vec<String> = artifact.ring().iter().map(hex::encode).collect();
    if !anonymous_ring_is_canonical_sorted(artifact.ring()) {
        return Ok(invalid_anonymous_verification(
            ring_members,
            "kind:40009 anonymous signature ring must be canonical sorted".to_string(),
        ));
    }
    let outer_pubkey = event.pubkey.to_hex().to_ascii_lowercase();
    if ring_members.iter().any(|member| member == &outer_pubkey) {
        return Ok(invalid_anonymous_verification(
            ring_members,
            "kind:40009 outer pubkey must be ephemeral, not a ring member".to_string(),
        ));
    }

    let body = unsigned_body_from_event(&event);
    let normalized_members: Vec<String> = member_pubkeys
        .unwrap_or_default()
        .into_iter()
        .map(|pubkey| pubkey.to_ascii_lowercase())
        .collect();
    if normalized_members
        .iter()
        .any(|member| member == &outer_pubkey)
    {
        return Ok(invalid_anonymous_verification(
            ring_members,
            "kind:40009 outer pubkey must be ephemeral, not a current channel member".to_string(),
        ));
    }
    let membership = if normalized_members.is_empty() {
        buzz_ring_sig_pkg::MembershipCheck::Unchecked
    } else {
        buzz_ring_sig_pkg::MembershipCheck::CurrentMembersOnly(&normalized_members)
    };

    match buzz_ring_sig_pkg::verify_artifact(&channel_id, &body, &artifact, membership) {
        Ok(result) => Ok(AnonymousVerificationInfo {
            crypto_valid: result.crypto_valid,
            anonymity_set_state: anonymity_set_state_name(result.anonymity_set_state).to_string(),
            ring_members,
            error: None,
        }),
        Err(error) => Ok(invalid_anonymous_verification(
            ring_members,
            format!("anonymous signature verification failed: {error}"),
        )),
    }
}

fn event_has_client_marker(event: &Event, marker: &str) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "client" && parts[1] == marker
    })
}

async fn find_managed_agent_channel_message_by_marker(
    state: &AppState,
    agent_pubkey: Option<&str>,
    channel_id: &str,
    marker: &str,
) -> Result<Option<Event>, String> {
    let author = agent_pubkey
        .map(str::trim)
        .filter(|pubkey| !pubkey.is_empty())
        .map(str::to_ascii_lowercase);

    let mut until: Option<u64> = None;

    for _ in 0..10 {
        let mut filter = serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_STREAM_MESSAGE],
            "#h": [channel_id],
            "limit": 500,
        });
        if let Some(author) = author.as_deref() {
            filter["authors"] = serde_json::json!([author]);
        }
        if let Some(until) = until {
            filter["until"] = serde_json::json!(until);
        }

        let events = query_relay(state, &[filter]).await?;
        if let Some(existing) = events
            .iter()
            .find(|event| event_has_client_marker(event, marker))
        {
            return Ok(Some(existing.clone()));
        }

        if events.len() < 500 {
            break;
        }
        until = events
            .iter()
            .map(|event| event.created_at.as_secs())
            .min()
            .map(|timestamp| timestamp.saturating_sub(1));
        if until.is_none() {
            break;
        }
    }

    Ok(None)
}

fn marker_author_for_scope<'a>(
    marker_scope: Option<&str>,
    agent_pubkey: &'a str,
) -> Option<&'a str> {
    match marker_scope {
        Some("channel") => None,
        _ => Some(agent_pubkey),
    }
}

fn stored_managed_agent_auth_tag(auth_tag: Option<&str>) -> Option<String> {
    auth_tag
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn legacy_managed_agent_auth_tag(
    owner_keys: &Keys,
    agent_pubkey: &PublicKey,
) -> Result<Option<String>, String> {
    if owner_keys.public_key() == *agent_pubkey {
        return Ok(None);
    }

    buzz_sdk_pkg::nip_oa::compute_auth_tag(owner_keys, agent_pubkey, "")
        .map(Some)
        .map_err(|error| format!("failed to compute managed agent auth tag: {error}"))
}

fn managed_agent_submission_auth_tag(
    record: &ManagedAgentRecord,
    state: &AppState,
    agent_pubkey: &PublicKey,
) -> Result<Option<String>, String> {
    if let Some(auth_tag) = stored_managed_agent_auth_tag(record.auth_tag.as_deref()) {
        return Ok(Some(auth_tag));
    }

    let owner_keys = state.keys.lock().map_err(|error| error.to_string())?;
    legacy_managed_agent_auth_tag(&owner_keys, agent_pubkey)
}

#[tauri::command]
pub async fn send_managed_agent_channel_message(
    agent_pubkey: String,
    channel_id: String,
    content: String,
    marker: Option<String>,
    marker_scope: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("message content is required".into());
    }
    let marker = marker
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let requested_pubkey = agent_pubkey.trim().to_ascii_lowercase();

    let record = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        find_managed_agent_mut(&mut records, &requested_pubkey)?.clone()
    };

    let keys = Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("failed to parse managed agent key: {error}"))?;
    let key_pubkey = keys.public_key().to_hex();
    if key_pubkey != record.pubkey.to_ascii_lowercase() {
        return Err(format!(
            "managed agent key does not match stored pubkey {}",
            record.pubkey
        ));
    }
    let submission_auth_tag =
        managed_agent_submission_auth_tag(&record, &state, &keys.public_key())?;

    if let Some(marker) = marker.as_deref() {
        if let Some(existing) = find_managed_agent_channel_message_by_marker(
            &state,
            marker_author_for_scope(marker_scope.as_deref(), &record.pubkey),
            &channel_id,
            marker,
        )
        .await?
        {
            return Ok(SendChannelMessageResponse {
                event_id: existing.id.to_hex(),
                parent_event_id: None,
                root_event_id: None,
                depth: 0,
                created_at: existing.created_at.as_secs() as i64,
            });
        }
    }

    let client_tags = marker
        .as_deref()
        .map(|marker| vec![vec!["client".to_string(), marker.to_string()]])
        .unwrap_or_default();
    let builder = events::build_message_with_client_tags(
        channel_uuid,
        trimmed,
        None,
        &[],
        &[],
        &[],
        &[],
        &client_tags,
    )?;
    let result =
        submit_event_with_keys(builder, &state, &keys, submission_auth_tag.as_deref()).await?;

    Ok(SendChannelMessageResponse {
        event_id: result.event_id,
        parent_event_id: None,
        root_event_id: None,
        depth: 0,
        created_at: chrono::Utc::now().timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_anon_event_with_tags(tags: Vec<Vec<&str>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|tag| nostr::Tag::parse(tag).expect("tag should parse"))
            .collect::<Vec<_>>();
        nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core_pkg::kind::KIND_STREAM_MESSAGE_ANON as u16),
            "anon",
        )
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .expect("event should sign")
    }

    fn test_secret(byte: u8) -> [u8; 32] {
        let mut secret = [0u8; 32];
        secret[31] = byte;
        secret
    }

    fn test_keys(byte: u8) -> Keys {
        Keys::new(nostr::SecretKey::from_slice(&test_secret(byte)).unwrap())
    }

    fn signed_valid_anonymous_event(
        channel_id: uuid::Uuid,
        signer_keys: &Keys,
        outer_keys: &Keys,
        ring_pubkeys: &[String],
    ) -> Event {
        let content = "anonymous hello";
        let created_at = 1_782_250_000u64;
        let channel = channel_id.to_string();
        let body = buzz_ring_sig_pkg::UnsignedEventBody {
            created_at,
            kind: buzz_core_pkg::kind::KIND_STREAM_MESSAGE_ANON as u64,
            tags: vec![vec!["h".to_string(), channel.clone()]],
            content: content.to_string(),
        };
        let signer_pubkey = signer_keys.public_key().to_hex().to_ascii_lowercase();
        let signer_index = ring_pubkeys
            .iter()
            .position(|pubkey| pubkey == &signer_pubkey)
            .expect("signer should be in ring");
        let secret_key32: [u8; 32] = signer_keys
            .secret_key()
            .as_secret_bytes()
            .try_into()
            .expect("test key is 32 bytes");
        let artifact = buzz_ring_sig_pkg::sign_with_aux(
            &channel,
            &body,
            ring_pubkeys,
            signer_index,
            secret_key32,
            [7u8; 32],
        )
        .expect("anonymous signature should sign");
        let h_tag = nostr::Tag::parse(["h", channel.as_str()]).unwrap();
        let ring_sig_tag = nostr::Tag::parse([
            buzz_ring_sig_pkg::RING_SIG_TAG,
            artifact.to_hex().as_str(),
            buzz_ring_sig_pkg::SCHEME_ID,
        ])
        .unwrap();

        nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core_pkg::kind::KIND_STREAM_MESSAGE_ANON as u16),
            content,
        )
        .tags([h_tag, ring_sig_tag])
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(outer_keys)
        .unwrap()
    }

    #[test]
    fn single_ring_sig_artifact_requires_exactly_one_marked_tag() {
        let event = signed_anon_event_with_tags(vec![vec![
            buzz_ring_sig_pkg::RING_SIG_TAG,
            "artifact",
            buzz_ring_sig_pkg::SCHEME_ID,
        ]]);
        assert_eq!(single_ring_sig_artifact(&event), Ok("artifact"));

        let duplicate = signed_anon_event_with_tags(vec![
            vec![
                buzz_ring_sig_pkg::RING_SIG_TAG,
                "artifact",
                buzz_ring_sig_pkg::SCHEME_ID,
            ],
            vec![
                buzz_ring_sig_pkg::RING_SIG_TAG,
                "other",
                buzz_ring_sig_pkg::SCHEME_ID,
            ],
        ]);
        assert!(single_ring_sig_artifact(&duplicate).is_err());

        let wrong_marker = signed_anon_event_with_tags(vec![vec![
            buzz_ring_sig_pkg::RING_SIG_TAG,
            "artifact",
            "other-scheme",
        ]]);
        assert!(single_ring_sig_artifact(&wrong_marker).is_err());
    }

    #[test]
    fn bound_anonymous_channel_id_rejects_supplied_channel_mismatch() {
        let event_channel_id = uuid::Uuid::new_v4().to_string();
        let supplied_channel_id = uuid::Uuid::new_v4().to_string();
        let event = signed_anon_event_with_tags(vec![
            vec!["h", &event_channel_id],
            vec![
                buzz_ring_sig_pkg::RING_SIG_TAG,
                "artifact",
                buzz_ring_sig_pkg::SCHEME_ID,
            ],
        ]);

        assert!(bound_anonymous_channel_id(&event, Some(&supplied_channel_id)).is_err());
    }

    #[test]
    fn bound_anonymous_channel_id_rejects_missing_h_even_with_supplied_channel() {
        let supplied_channel_id = uuid::Uuid::new_v4().to_string();
        let event = signed_anon_event_with_tags(vec![vec![
            buzz_ring_sig_pkg::RING_SIG_TAG,
            "artifact",
            buzz_ring_sig_pkg::SCHEME_ID,
        ]]);

        assert!(bound_anonymous_channel_id(&event, Some(&supplied_channel_id)).is_err());
    }

    #[test]
    fn anonymous_ring_pubkeys_accepts_canonical_selected_order() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let third_member_keys = test_keys(3);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            third_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let mut selected = vec![
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        selected.sort();

        let ring = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect("selected ring should validate");

        assert_eq!(ring, selected);
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_noncanonical_selected_order() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let mut selected = vec![
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        selected.sort();
        selected.reverse();

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("noncanonical ring order should be rejected");

        assert!(err.contains("sorted"));
    }

    #[test]
    fn anonymous_ring_pubkeys_round_trips_selected_ring_to_artifact() {
        let channel_id = uuid::Uuid::parse_str("2114f4bc-5130-4816-83da-acbfcc65243b").unwrap();
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let third_member_keys = test_keys(3);
        let outer_keys = test_keys(4);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            third_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let mut selected = vec![
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        selected.sort();

        let ring = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect("selected ring should validate");
        let event = signed_valid_anonymous_event(channel_id, &signer_keys, &outer_keys, &ring);
        let artifact =
            buzz_ring_sig_pkg::SagArtifact::from_hex(single_ring_sig_artifact(&event).unwrap())
                .expect("artifact should parse");
        let artifact_ring: Vec<String> = artifact.ring().iter().map(hex::encode).collect();

        assert_eq!(artifact_ring, selected);
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_duplicate_selected_members() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let selected = vec![
            signer_keys.public_key().to_hex(),
            signer_keys.public_key().to_hex(),
        ];

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("duplicate ring members should be rejected");

        assert!(err.contains("duplicate"));
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_non_current_member() {
        let signer_keys = test_keys(1);
        let non_member_keys = test_keys(2);
        let other_member_keys = test_keys(3);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let selected = vec![
            signer_keys.public_key().to_hex(),
            non_member_keys.public_key().to_hex(),
        ];

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("non-member ring key should be rejected");

        assert!(err.contains("current channel members"));
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_noncanonical_selected_member() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let selected = vec![
            signer_keys.public_key().to_hex().to_ascii_uppercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("noncanonical ring key should be rejected");

        assert!(err.contains("canonical lowercase"));
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_invalid_selected_member() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let selected = vec![
            signer_keys.public_key().to_hex(),
            "not-a-pubkey".to_string(),
        ];

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("invalid ring key should be rejected");

        assert!(err.contains("invalid pubkey"));
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_one_member_ring() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let selected = vec![signer_keys.public_key().to_hex()];

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("one-member ring should be rejected");

        assert!(err.contains("at least 2"));
    }

    #[test]
    fn anonymous_ring_pubkeys_rejects_missing_signer() {
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let third_member_keys = test_keys(3);
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            third_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let selected = vec![
            other_member_keys.public_key().to_hex(),
            third_member_keys.public_key().to_hex(),
        ];

        let err = anonymous_ring_pubkeys(
            &current_members,
            Some(&selected),
            &signer_keys.public_key().to_hex().to_ascii_lowercase(),
        )
        .expect_err("ring without active identity should be rejected");

        assert!(err.contains("active identity"));
    }

    #[test]
    fn verify_anonymous_event_rejects_outer_pubkey_in_ring() {
        let channel_id = uuid::Uuid::parse_str("2114f4bc-5130-4816-83da-acbfcc65243b").unwrap();
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let ring_pubkeys = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let event =
            signed_valid_anonymous_event(channel_id, &signer_keys, &signer_keys, &ring_pubkeys);

        let info = verify_anonymous_event(event.as_json(), Some(channel_id.to_string()), None)
            .expect("verification command should return a DTO");

        assert!(!info.crypto_valid);
        assert_eq!(info.anonymity_set_state, "invalid");
        assert_eq!(info.ring_members, ring_pubkeys);
        assert!(
            info.error
                .as_deref()
                .is_some_and(|error| error.contains("outer pubkey must be ephemeral")),
            "unexpected error: {:?}",
            info.error
        );
    }

    #[test]
    fn verify_anonymous_event_rejects_unsorted_ring() {
        let channel_id = uuid::Uuid::parse_str("2114f4bc-5130-4816-83da-acbfcc65243b").unwrap();
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let outer_keys = test_keys(3);
        let mut ring_pubkeys = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        ring_pubkeys.sort();
        ring_pubkeys.reverse();
        let event =
            signed_valid_anonymous_event(channel_id, &signer_keys, &outer_keys, &ring_pubkeys);

        let info = verify_anonymous_event(event.as_json(), Some(channel_id.to_string()), None)
            .expect("verification command should return a DTO");

        assert!(!info.crypto_valid);
        assert_eq!(info.anonymity_set_state, "invalid");
        assert_eq!(info.ring_members, ring_pubkeys);
        assert!(
            info.error
                .as_deref()
                .is_some_and(|error| error.contains("canonical sorted")),
            "unexpected error: {:?}",
            info.error
        );
    }

    #[test]
    fn verify_anonymous_event_rejects_outer_pubkey_in_supplied_current_members() {
        let channel_id = uuid::Uuid::parse_str("2114f4bc-5130-4816-83da-acbfcc65243b").unwrap();
        let signer_keys = test_keys(1);
        let other_member_keys = test_keys(2);
        let outer_keys = test_keys(3);
        let ring_pubkeys = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let current_members = vec![
            signer_keys.public_key().to_hex().to_ascii_lowercase(),
            other_member_keys.public_key().to_hex().to_ascii_lowercase(),
            outer_keys.public_key().to_hex().to_ascii_lowercase(),
        ];
        let event =
            signed_valid_anonymous_event(channel_id, &signer_keys, &outer_keys, &ring_pubkeys);

        let info = verify_anonymous_event(
            event.as_json(),
            Some(channel_id.to_string()),
            Some(current_members),
        )
        .expect("verification command should return a DTO");

        assert!(!info.crypto_valid);
        assert_eq!(info.anonymity_set_state, "invalid");
        assert_eq!(info.ring_members, ring_pubkeys);
        assert!(
            info.error
                .as_deref()
                .is_some_and(|error| error.contains("current channel member")),
            "unexpected error: {:?}",
            info.error
        );
    }

    #[test]
    fn marker_author_scope_defaults_to_agent() {
        assert_eq!(
            marker_author_for_scope(None, "agent-pubkey"),
            Some("agent-pubkey")
        );
        assert_eq!(
            marker_author_for_scope(Some("agent"), "agent-pubkey"),
            Some("agent-pubkey")
        );
        assert_eq!(
            marker_author_for_scope(Some("unknown"), "agent-pubkey"),
            Some("agent-pubkey")
        );
    }

    #[test]
    fn marker_author_scope_can_dedupe_across_channel() {
        assert_eq!(
            marker_author_for_scope(Some("channel"), "agent-pubkey"),
            None
        );
    }

    #[test]
    fn stored_managed_agent_auth_tag_trims_blank_values() {
        assert_eq!(
            stored_managed_agent_auth_tag(Some("  [\"auth\",\"owner\",\"\",\"sig\"]  ")),
            Some("[\"auth\",\"owner\",\"\",\"sig\"]".to_string())
        );
        assert_eq!(stored_managed_agent_auth_tag(Some("   ")), None);
        assert_eq!(stored_managed_agent_auth_tag(None), None);
    }

    #[test]
    fn legacy_managed_agent_auth_tag_verifies_for_agent_pubkey() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();

        let tag = legacy_managed_agent_auth_tag(&owner_keys, &agent_keys.public_key())
            .expect("legacy auth tag should compute")
            .expect("legacy auth tag should be present");

        let owner = buzz_sdk_pkg::nip_oa::verify_auth_tag(&tag, &agent_keys.public_key())
            .expect("legacy auth tag should verify");
        assert_eq!(owner, owner_keys.public_key());
    }

    #[test]
    fn legacy_managed_agent_auth_tag_skips_self_attestation() {
        let owner_keys = Keys::generate();

        let tag = legacy_managed_agent_auth_tag(&owner_keys, &owner_keys.public_key())
            .expect("self-attestation should be skipped");

        assert_eq!(tag, None);
    }
}

#[tauri::command]
pub async fn add_reaction(
    event_id: String,
    emoji: String,
    emoji_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let builder = match emoji_url {
        // Custom-emoji reaction (NIP-30): kind:7 with `:shortcode:` content and
        // an `["emoji", shortcode, url]` tag. Delegates to the SDK builder so
        // shortcode normalization + validation match the relay exactly.
        Some(url) => buzz_sdk_pkg::build_custom_emoji_reaction(target_eid, emoji.trim(), &url)
            .map_err(|e| format!("invalid custom emoji reaction: {e}"))?,
        None => events::build_reaction(target_eid, emoji.trim())?,
    };
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn remove_reaction(
    event_id: String,
    emoji: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Find our own kind:7 reaction event referencing the target.
    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };
    let target = event_id.trim();
    let trimmed_emoji = emoji.trim();

    let reactions = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [7],
            "#e": [target],
            "authors": [my_pubkey],
        })],
    )
    .await?;

    let reaction_event = reactions
        .iter()
        .find(|ev| ev.content.trim() == trimmed_emoji)
        .ok_or("could not find your reaction event for this emoji")?;

    let builder = events::build_remove_reaction(reaction_event.id)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn edit_message(
    channel_id: String,
    event_id: String,
    content: String,
    media_tags: Vec<Vec<String>>,
    emoji_tags: Option<Vec<Vec<String>>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let trimmed = content.trim();
    // Empty text is allowed when the edit still carries imeta attachments
    // (a media-only edit). Reject only when both are empty.
    if trimmed.is_empty() && media_tags.is_empty() {
        return Err("edit must have content or attachments".into());
    }
    let emoji = emoji_tags.unwrap_or_default();
    let builder =
        events::build_message_edit(channel_uuid, target_eid, trimmed, &media_tags, &emoji)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn delete_message(
    channel_id: String,
    event_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let builder = events::build_delete_compat(channel_uuid, target_eid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

// ── Local helpers ───────────────────────────────────────────────────────────

fn channel_id_from_tags(ev: &nostr::Event) -> Option<String> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "h" {
            Some(s[1].clone())
        } else {
            None
        }
    })
}

fn tags_to_vec(ev: &nostr::Event) -> Vec<Vec<String>> {
    ev.tags.iter().map(|t| t.as_slice().to_vec()).collect()
}

fn feed_item_from_event(ev: &nostr::Event, category: &str) -> FeedItemInfo {
    let channel_id = channel_id_from_tags(ev);
    FeedItemInfo {
        id: ev.id.to_hex(),
        kind: ev.kind.as_u16() as u32,
        pubkey: ev.pubkey.to_hex(),
        content: ev.content.clone(),
        created_at: ev.created_at.as_secs(),
        channel_id,
        channel_name: String::new(),
        channel_type: None,
        tags: tags_to_vec(ev),
        category: category.to_string(),
    }
}

fn forum_message_from_event(ev: &nostr::Event, channel_id: &str) -> ForumMessageInfo {
    ForumMessageInfo {
        event_id: ev.id.to_hex(),
        pubkey: ev.pubkey.to_hex(),
        content: ev.content.clone(),
        kind: ev.kind.as_u16() as u32,
        created_at: ev.created_at.as_secs() as i64,
        channel_id: channel_id.to_string(),
        tags: tags_to_vec(ev),
        thread_summary: Some(ThreadSummary {
            reply_count: 0,
            descendant_count: 0,
            last_reply_at: None,
            participants: Vec::new(),
        }),
        reactions: serde_json::Value::Null,
    }
}

fn forum_reply_from_event(
    ev: &nostr::Event,
    channel_id: &str,
    root_event_id: &str,
) -> ForumThreadReplyInfo {
    // Walk e-tags for NIP-10 parent/root markers.
    let (mut parent_id, mut explicit_root) = (None, None);
    for t in ev.tags.iter() {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "e" {
            match s.get(3).map(|x| x.as_str()) {
                Some("root") => explicit_root = Some(s[1].clone()),
                Some("reply") => parent_id = Some(s[1].clone()),
                _ => {
                    if parent_id.is_none() {
                        parent_id = Some(s[1].clone());
                    }
                }
            }
        }
    }
    let parent = parent_id
        .clone()
        .unwrap_or_else(|| root_event_id.to_string());
    let root = explicit_root.unwrap_or_else(|| root_event_id.to_string());
    let depth = if parent == root { 1 } else { 2 };

    ForumThreadReplyInfo {
        event_id: ev.id.to_hex(),
        pubkey: ev.pubkey.to_hex(),
        content: ev.content.clone(),
        kind: ev.kind.as_u16() as u32,
        created_at: ev.created_at.as_secs() as i64,
        channel_id: channel_id.to_string(),
        tags: tags_to_vec(ev),
        parent_event_id: Some(parent),
        root_event_id: Some(root),
        depth,
        broadcast: false,
        reactions: serde_json::Value::Null,
    }
}
