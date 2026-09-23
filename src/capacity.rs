use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{
    CapacityConstraint, CapacityMapping, CapacityObservation, CapacityValue, ScarcityState,
    config::CapacityConfig, db::Database,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityAuthorization {
    pub id: String,
    pub revision: u64,
}

/// Validate observed evidence against one explicit, user-controlled funding
/// epoch. Observations are always retained, but a rejected epoch cannot become
/// authorized merely because the same conflicting observation is seen again.
pub fn authorize_observation(
    db: &Database,
    pool_id: &str,
    observation: &CapacityObservation,
    route_revision: &str,
    authorization_revision: u64,
    funding_source: &str,
) -> Result<CapacityAuthorization> {
    anyhow::ensure!(
        observation.pool_id == pool_id,
        "capacity evidence belongs to another pool"
    );
    anyhow::ensure!(
        observation.valid_until > Utc::now(),
        "capacity evidence expired before authorization"
    );
    let revision = i64::try_from(authorization_revision)
        .context("authorization revision exceeds SQLite range")?;
    // Serialize validation with other authorizers and the final launch fence.
    let transaction = rusqlite::Transaction::new_unchecked(
        db.connection(),
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let conflict = retained_authorization_conflict(
        &transaction,
        pool_id,
        revision,
        observation,
        funding_source,
    )?;
    let id = Ulid::new().to_string();
    let evidence = serde_json::to_string(observation)?;
    let now = Utc::now().to_rfc3339();
    if let Some(reason) = conflict {
        transaction.execute(
            "INSERT INTO capacity_authorizations(id,pool_id,authorization_revision,observation_id,route_revision,evidence_json,status,reason,created_at,valid_until) VALUES(?1,?2,?3,?4,?5,?6,'rejected',?7,?8,?9)",
            rusqlite::params![id, pool_id, revision, observation.id, route_revision, evidence, reason, now, observation.valid_until.to_rfc3339()],
        )?;
        transaction.commit()?;
        bail!("{reason}; funding authorization revision {authorization_revision} is now invalid")
    }
    // Authorization IDs remain bound to this pool and route; only the funding
    // history used to validate them is shared across overlapping mappings.
    let existing = transaction
        .query_row(
            "SELECT id FROM capacity_authorizations WHERE pool_id=?1 AND authorization_revision=?2 AND observation_id=?3 AND route_revision=?4 AND status='authorized' ORDER BY created_at DESC,id DESC LIMIT 1",
            rusqlite::params![pool_id, revision, observation.id, route_revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(id) = existing {
        transaction.commit()?;
        return Ok(CapacityAuthorization {
            id,
            revision: authorization_revision,
        });
    }
    transaction.execute(
        "INSERT INTO capacity_authorizations(id,pool_id,authorization_revision,observation_id,route_revision,evidence_json,status,reason,created_at,valid_until) VALUES(?1,?2,?3,?4,?5,?6,'authorized',NULL,?7,?8)",
        rusqlite::params![id, pool_id, revision, observation.id, route_revision, evidence, now, observation.valid_until.to_rfc3339()],
    )?;
    transaction.commit()?;
    Ok(CapacityAuthorization {
        id,
        revision: authorization_revision,
    })
}

/// Raw conflicts are relevant even if their writer died before validating them.
/// Funding epochs follow the same provider/funding/bucket overlap as admission.
/// A mapping change inherits the epoch's baselines and rejections. Only an
/// explicit new authorization revision starts at its first evidence sample;
/// expiry or Unknown cannot renew an invalidated identity within that epoch.
pub(crate) fn retained_authorization_conflict(
    connection: &rusqlite::Connection,
    pool_id: &str,
    authorization_revision: i64,
    current: &CapacityObservation,
    funding_source: &str,
) -> Result<Option<String>> {
    let pools = serde_json::to_string(&crate::db::overlapping_pools(connection, pool_id)?)?;
    let mut statement = connection.prepare(
        "SELECT status,evidence_json FROM capacity_authorizations WHERE pool_id IN (SELECT value FROM json_each(?1)) AND authorization_revision=?2 ORDER BY created_at,id",
    )?;
    let mut authorized = Vec::new();
    for row in statement.query_map(rusqlite::params![pools, authorization_revision], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (status, evidence) = row?;
        if status == "rejected" {
            return Ok(Some(format!(
                "funding authorization revision {authorization_revision} was invalidated; increment authorization_revision only after affirmatively revalidating the account"
            )));
        }
        authorized.push(serde_json::from_str::<CapacityObservation>(&evidence)?);
    }
    let since = authorized
        .iter()
        .map(|value| (value.sampled_at, value.id.as_str()))
        .min()
        .unwrap_or((current.sampled_at, current.id.as_str()));
    let mut known = authorized.to_vec();
    if known.is_empty() {
        known.push(current.clone());
    }
    for observation in crate::db::shared_capacity_history(connection, pool_id)?
        .into_iter()
        .filter(|value| (value.sampled_at, value.id.as_str()) >= since)
        .chain(std::iter::once(current.clone()))
    {
        if let Some(reason) = authorization_conflict(&known, &observation, funding_source) {
            return Ok(Some(reason));
        }
        known.push(observation);
    }
    Ok(None)
}

/// Resolve each reported constraint independently. Known restrictions survive
/// Unknown/TTL expiry/reset predictions until a newer authoritative reading of
/// that constraint supersedes them. Stale positive readings never imply supply.
pub(crate) fn resolved_scarcity(
    connection: &rusqlite::Connection,
    pool_id: &str,
    now: DateTime<Utc>,
) -> Result<ScarcityState> {
    use std::collections::{BTreeMap, BTreeSet};
    let buckets: String = connection.query_row(
        "SELECT provider_buckets_json FROM resource_pools WHERE id=?1",
        [pool_id],
        |r| r.get(0),
    )?;
    let buckets: Vec<String> = serde_json::from_str(&buckets)?;
    let mut windows = BTreeMap::new();
    let mut rejected = BTreeSet::new();
    let mut legacy_rejected: BTreeMap<String, BTreeSet<Option<String>>> = BTreeMap::new();
    let mut unknown = false;
    for observation in crate::db::shared_capacity_history(connection, pool_id)? {
        let applicable: Vec<_> = observation
            .constraints
            .iter()
            .filter(|constraint| {
                matches!(constraint.scope, CapacityValue::Reported { .. })
                    && constraint
                        .provider_bucket_id
                        .as_ref()
                        .is_some_and(|id| buckets.is_empty() || buckets.contains(id))
            })
            .collect();
        let fresh_sample = observation.valid_until > observation.sampled_at;
        let bucket_ids: BTreeSet<_> = applicable
            .iter()
            .filter_map(|c| c.provider_bucket_id.clone())
            .collect();
        for id in bucket_ids {
            if applicable.iter().any(|c| {
                c.provider_bucket_id.as_ref() == Some(&id) && c.kind == "provider_rejection"
            }) {
                rejected.insert(id);
            } else {
                let reported: Vec<_> = applicable
                    .iter()
                    .filter(|c| {
                        c.provider_bucket_id.as_ref() == Some(&id) && c.kind == "allowance_window"
                    })
                    .collect();
                // Complete, successful availability supersedes a bucket rejection.
                if fresh_sample && !reported.is_empty() && reported.iter().all(|c| {
                    matches!(c.remaining, CapacityValue::Reported {value} if value>0.0)
                        && !matches!(c.reset_at, CapacityValue::Reported {value} if value<=observation.sampled_at)
                }) {
                    rejected.remove(&id);
                    for unresolved in legacy_rejected.values_mut() {
                        unresolved.remove(&Some(id.clone()));
                    }
                    legacy_rejected.retain(|_,unresolved|!unresolved.is_empty());
                }
            }
        }
        let explicit_block = observation.constraints.iter().any(|c| {
            c.kind == "provider_rejection"
                || matches!(c.remaining, CapacityValue::Reported {value} if value<=0.0)
        });
        if observation.scarcity == ScarcityState::Exhausted && !explicit_block {
            // Pre-correction observations did not record bucket rejection separately.
            legacy_rejected.insert(
                observation.pool_id.clone(),
                observation
                    .constraints
                    .iter()
                    .map(|c| c.provider_bucket_id.clone())
                    .collect(),
            );
        } else if fresh_sample
            && observation.mapping == CapacityMapping::Mapped
            && matches!(
                observation.scarcity,
                ScarcityState::Available | ScarcityState::Constrained | ScarcityState::Reserve
            )
        {
            legacy_rejected.remove(&observation.pool_id);
        }
        unknown = observation.mapping == CapacityMapping::Unknown;
        for constraint in applicable {
            if constraint.kind != "allowance_window" {
                continue;
            }
            if let CapacityValue::Reported { value } = constraint.remaining {
                let passed_reset = matches!(constraint.reset_at, CapacityValue::Reported {value} if value<=observation.sampled_at);
                if (fresh_sample && !passed_reset) || value <= 0.0 {
                    windows.insert(
                        (
                            constraint.provider_bucket_id.clone(),
                            constraint.window_id.clone(),
                        ),
                        (value, observation.valid_until),
                    );
                }
            } else {
                unknown = true;
            }
        }
    }
    if !rejected.is_empty()
        || !legacy_rejected.is_empty()
        || windows.values().any(|(value, _)| *value <= 0.0)
    {
        return Ok(ScarcityState::Exhausted);
    }
    if windows.values().any(|(value, _)| *value < 20.0) {
        return Ok(ScarcityState::Reserve);
    }
    if windows.is_empty() || unknown || windows.values().any(|(_, expires)| *expires <= now) {
        return Ok(ScarcityState::Unknown);
    }
    Ok(if windows.values().any(|(value, _)| *value < 50.0) {
        ScarcityState::Constrained
    } else {
        ScarcityState::Available
    })
}

pub(crate) fn authorization_conflict(
    authorized: &[CapacityObservation],
    current: &CapacityObservation,
    funding_source: &str,
) -> Option<String> {
    funding_change(current, funding_source).or_else(|| {
        authorized
            .iter()
            .find_map(|previous| observation_identity_change(Some(previous), current))
    })
}

pub fn launch_block(
    authorized: &CapacityObservation,
    newest: &CapacityObservation,
    funding_source: &str,
    priority: i32,
) -> Option<String> {
    if newest.valid_until <= Utc::now() {
        return Some("newest shared capacity evidence is stale".into());
    }
    if let Some(change) = funding_change(newest, funding_source)
        .or_else(|| observation_identity_change(Some(authorized), newest))
    {
        return Some(change);
    }
    match newest.scarcity {
        ScarcityState::Exhausted => Some("an applicable allowance constraint is exhausted".into()),
        ScarcityState::Reserve if priority < 1 => {
            Some("allowance is in reserve and this request is not explicitly urgent".into())
        }
        _ => None,
    }
}

#[derive(Debug)]
pub struct CapacityProbeResult {
    pub observation: CapacityObservation,
    pub raw: Value,
}

pub fn supports_codex_account_probe(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "codex" | "codex.exe"))
}

pub async fn probe_codex(
    executable: &Path,
    pool_id: &str,
    provider_buckets: &[String],
    service_tier: &str,
    config: &CapacityConfig,
) -> CapacityProbeResult {
    let sampled_at = Utc::now();
    let result = crate::harness::codex::read_account(
        executable,
        Duration::from_secs(config.probe_timeout_secs),
    )
    .await;
    match result {
        Ok(raw) => CapacityProbeResult {
            observation: normalize_codex(
                pool_id,
                provider_buckets,
                service_tier,
                sampled_at,
                config.freshness_secs,
                raw.clone(),
            ),
            raw: json!({"account": raw.account, "rate_limits": raw.limits, "rate_limits_error": raw.limits_error}),
        },
        Err(error) => {
            let reason = format!("probe failure: {error:#}");
            CapacityProbeResult {
                observation: unknown_observation(
                    pool_id,
                    service_tier,
                    sampled_at,
                    config.freshness_secs,
                    &reason,
                ),
                raw: json!({"error": reason}),
            }
        }
    }
}

pub fn unknown_observation(
    pool_id: &str,
    service_tier: &str,
    sampled_at: DateTime<Utc>,
    freshness_secs: u64,
    reason: impl Into<String>,
) -> CapacityObservation {
    let reason = reason.into();
    CapacityObservation {
        id: Ulid::new().to_string(),
        pool_id: pool_id.to_owned(),
        source: "unavailable".into(),
        source_version: "unknown".into(),
        sampled_at,
        valid_until: sampled_at
            + TimeDelta::seconds(i64::try_from(freshness_secs).unwrap_or(i64::MAX)),
        mapping: CapacityMapping::Unknown,
        auth_mode: CapacityValue::unknown(reason.clone()),
        funding_identity: CapacityValue::unknown(reason.clone()),
        plan_type: CapacityValue::unknown(reason.clone()),
        credits_available: CapacityValue::unknown(reason.clone()),
        service_tier: CapacityValue::unknown(format!(
            "{reason}; requested invocation mode is {service_tier}"
        )),
        constraints: vec![CapacityConstraint {
            kind: "allowance_window".into(),
            unit: "provider_percent".into(),
            provider_bucket_id: None,
            window_id: None,
            reported_used_percent: None,
            remaining: CapacityValue::unknown(reason.clone()),
            reset_at: CapacityValue::unknown(reason.clone()),
            window_duration_secs: CapacityValue::unknown(reason.clone()),
            scope: CapacityValue::unknown("unknown pool mapping"),
        }],
        scarcity: ScarcityState::Unknown,
        attributable_attempt_id: None,
        attribution: "pool_level_mixed_activity".into(),
        raw_observation_ref: None,
    }
}

fn normalize_codex(
    pool_id: &str,
    provider_buckets: &[String],
    _service_tier: &str,
    sampled_at: DateTime<Utc>,
    freshness_secs: u64,
    raw: crate::harness::codex::ProbeResponse,
) -> CapacityObservation {
    let auth_mode = raw
        .account
        .pointer("/result/account/type")
        .and_then(Value::as_str)
        .map(|value| CapacityValue::Reported {
            value: value.to_owned(),
        })
        .unwrap_or_else(|| CapacityValue::unknown("account auth mode absent"));
    let plan_type = raw
        .account
        .pointer("/result/account/planType")
        .and_then(Value::as_str)
        .map(|value| CapacityValue::Reported {
            value: value.to_owned(),
        })
        .unwrap_or_else(|| CapacityValue::unknown("account plan absent"));
    let funding_identity = raw
        .account
        .pointer("/result/account/email")
        .or_else(|| raw.account.pointer("/result/account/id"))
        .and_then(Value::as_str)
        .map(|value| CapacityValue::Reported {
            value: format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes()))),
        })
        .unwrap_or_else(|| CapacityValue::unknown("account identity absent"));
    let limits = raw.limits.as_ref().unwrap_or(&Value::Null);
    let buckets = limits
        .pointer("/result/rateLimitsByLimitId")
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| {
            limits
                .pointer("/result/rateLimits")
                .filter(|value| !value.is_null())
                .map(|value| {
                    let id = value
                        .get("limitId")
                        .and_then(Value::as_str)
                        .unwrap_or("legacy")
                        .to_owned();
                    [(id, value.clone())].into_iter().collect()
                })
        })
        .unwrap_or_default();
    let mapping = if !provider_buckets.is_empty()
        && provider_buckets.iter().all(|id| buckets.contains_key(id))
    {
        CapacityMapping::Mapped
    } else {
        CapacityMapping::Unknown
    };
    let selected = if !provider_buckets.is_empty() {
        provider_buckets.to_vec()
    } else {
        buckets.keys().cloned().collect()
    };
    let mut constraints = Vec::new();
    let observed_service_tier = limits
        .pointer("/result/serviceTier")
        .and_then(Value::as_str)
        .or_else(|| {
            buckets
                .values()
                .find_map(|bucket| bucket.get("serviceTier").and_then(Value::as_str))
        })
        .map(|value| CapacityValue::Reported {
            value: value.to_owned(),
        })
        .unwrap_or_else(|| CapacityValue::unknown("service tier absent from provider response"));
    let mut earliest_reset = None;
    let mut any_past_reset = false;
    let mut any_reported = false;
    let mut any_unknown_constraint = false;
    let mut minimum_remaining = 100.0_f64;
    let mut exhausted = false;
    let mut credits_reported = false;
    let mut all_credits_reported = true;
    let mut any_credits = false;
    for bucket_id in selected {
        let bucket = buckets.get(&bucket_id).unwrap_or(&Value::Null);
        let applicable = provider_buckets.contains(&bucket_id);
        if let Some(has_credits) = bucket
            .pointer("/credits/hasCredits")
            .and_then(Value::as_bool)
        {
            credits_reported = true;
            any_credits |= has_credits;
        } else {
            all_credits_reported = false;
        }
        let rejected = bucket
            .get("rateLimitReachedType")
            .is_some_and(|value| !value.is_null());
        exhausted |= applicable && rejected;
        for window_id in ["primary", "secondary"] {
            let window = bucket.get(window_id).unwrap_or(&Value::Null);
            let used = window.get("usedPercent").and_then(Value::as_f64);
            let remaining = used.map(|value| 100.0 - value);
            if let Some(value) = remaining {
                any_reported |= applicable;
                if applicable {
                    minimum_remaining = minimum_remaining.min(value);
                }
                exhausted |= applicable && value <= 0.0;
            }
            let reset = window
                .get("resetsAt")
                .and_then(Value::as_i64)
                .and_then(|value| DateTime::from_timestamp(value, 0));
            if let Some(value) = reset {
                if value > sampled_at {
                    earliest_reset =
                        Some(earliest_reset.map_or(value, |old: DateTime<Utc>| old.min(value)));
                } else {
                    any_past_reset = true;
                }
            }
            let duration = window
                .get("windowDurationMins")
                .and_then(Value::as_u64)
                .and_then(|value| value.checked_mul(60));
            let absent = window.is_null();
            any_unknown_constraint |= remaining.is_none();
            constraints.push(CapacityConstraint {
                kind: "allowance_window".into(),
                unit: "provider_percent".into(),
                provider_bucket_id: Some(bucket_id.clone()),
                window_id: Some(window_id.into()),
                reported_used_percent: used,
                remaining: remaining
                    .map(|value| CapacityValue::Reported { value })
                    .unwrap_or_else(|| {
                        CapacityValue::unknown(if absent {
                            raw.limits_error.as_deref().unwrap_or("window absent")
                        } else {
                            "used percent absent"
                        })
                    }),
                reset_at: reset
                    .map(|value| CapacityValue::Reported { value })
                    .unwrap_or_else(|| {
                        CapacityValue::unknown(if absent {
                            "window absent"
                        } else {
                            "reset absent"
                        })
                    }),
                window_duration_secs: duration
                    .map(|value| CapacityValue::Reported { value })
                    .unwrap_or_else(|| {
                        CapacityValue::unknown(if absent {
                            "window absent"
                        } else {
                            "duration absent"
                        })
                    }),
                scope: if applicable {
                    CapacityValue::Reported {
                        value: pool_id.to_owned(),
                    }
                } else {
                    CapacityValue::unknown("unknown pool mapping")
                },
            });
        }
        if rejected {
            constraints.push(CapacityConstraint {
                kind: "provider_rejection".into(),
                unit: "provider_status".into(),
                provider_bucket_id: Some(bucket_id.clone()),
                window_id: None,
                reported_used_percent: None,
                remaining: CapacityValue::unknown("provider rejected work; not a numeric balance"),
                reset_at: CapacityValue::unknown("refresh required to establish availability"),
                window_duration_secs: CapacityValue::unknown("bucket rejection"),
                scope: if applicable {
                    CapacityValue::Reported {
                        value: pool_id.to_owned(),
                    }
                } else {
                    CapacityValue::unknown("unknown pool mapping")
                },
            });
        }
    }
    if constraints.is_empty() {
        let reason = raw
            .limits_error
            .as_deref()
            .unwrap_or("no rate-limit windows reported");
        constraints.push(CapacityConstraint {
            kind: "allowance_window".into(),
            unit: "provider_percent".into(),
            provider_bucket_id: None,
            window_id: None,
            reported_used_percent: None,
            remaining: CapacityValue::unknown(reason),
            reset_at: CapacityValue::unknown(reason),
            window_duration_secs: CapacityValue::unknown(reason),
            scope: CapacityValue::unknown("unknown pool mapping"),
        });
    }
    let scarcity = if exhausted {
        ScarcityState::Exhausted
    } else if any_reported && minimum_remaining < 20.0 {
        ScarcityState::Reserve
    } else if mapping == CapacityMapping::Unknown
        || any_past_reset
        || !any_reported
        || any_unknown_constraint
    {
        ScarcityState::Unknown
    } else if minimum_remaining < 50.0 {
        ScarcityState::Constrained
    } else {
        ScarcityState::Available
    };
    let nominal_expiry =
        sampled_at + TimeDelta::seconds(i64::try_from(freshness_secs).unwrap_or(i64::MAX));
    let valid_until = earliest_reset
        .filter(|reset| *reset > sampled_at)
        .map_or(nominal_expiry, |reset| nominal_expiry.min(reset));
    CapacityObservation {
        id: Ulid::new().to_string(),
        pool_id: pool_id.to_owned(),
        source: "codex_app_server".into(),
        source_version: raw.source_version,
        sampled_at,
        valid_until,
        mapping,
        auth_mode,
        funding_identity,
        plan_type,
        credits_available: if any_credits || (credits_reported && all_credits_reported) {
            CapacityValue::Reported { value: any_credits }
        } else {
            CapacityValue::unknown("credit state absent")
        },
        service_tier: observed_service_tier,
        constraints,
        scarcity,
        attributable_attempt_id: None,
        attribution: "pool_level_mixed_activity".into(),
        raw_observation_ref: None,
    }
}

pub fn funding_change(observation: &CapacityObservation, funding_source: &str) -> Option<String> {
    if let CapacityValue::Reported { value } = &observation.auth_mode
        && !(value == "chatgpt"
            || (value == "claude.ai" && observation.source == "claude_auth_status_v1"))
    {
        return Some(format!("authentication changed to {value}"));
    }
    if let CapacityValue::Reported { value: true } = observation.credits_available {
        return Some("paid credits are now available".into());
    }
    if let CapacityValue::Reported { value } = &observation.service_tier
        && !matches!(value.as_str(), "standard" | "default")
    {
        return Some(format!("service tier changed to {value}"));
    }
    if let CapacityValue::Reported { value } = &observation.plan_type {
        let expected = format!("chatgpt-{value}");
        if funding_source.starts_with("chatgpt-") && !funding_source.starts_with(&expected) {
            return Some(format!("funding plan changed to {value}"));
        }
    }
    None
}

pub fn observation_identity_change(
    previous: Option<&CapacityObservation>,
    current: &CapacityObservation,
) -> Option<String> {
    let previous = previous?;
    if let (CapacityValue::Reported { value: old }, CapacityValue::Reported { value: new }) =
        (&previous.funding_identity, &current.funding_identity)
        && old != new
    {
        return Some("authenticated funding identity changed".into());
    }
    // Unknown mapping is not evidence of a change. It stays unknown and can be
    // used only under the separately validated no-overage authorization.
    let identities = |observation: &CapacityObservation| {
        observation
            .constraints
            .iter()
            .filter(|constraint| constraint.kind == "allowance_window")
            .map(|constraint| {
                (
                    constraint.provider_bucket_id.clone(),
                    constraint.window_id.clone(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
    };
    // Window sets describe a particular pool mapping. Funding identity is
    // shared above, but adding another bucket is not a provider window change.
    if previous.pool_id == current.pool_id
        && previous.mapping == CapacityMapping::Mapped
        && current.mapping == CapacityMapping::Mapped
        && identities(previous) != identities(current)
    {
        return Some("provider allowance window identity changed".into());
    }
    None
}

pub fn usable_now(observation: &CapacityObservation, now: DateTime<Utc>) -> bool {
    observation.valid_until > now && observation.scarcity != ScarcityState::Exhausted
}

/// A reset boundary is a signal to observe again, never evidence that capacity
/// was replenished. Estimated upper bounds are intentionally treated the same
/// way as reported reset timestamps for refresh scheduling only.
pub fn needs_refresh(observation: &CapacityObservation, now: DateTime<Utc>) -> bool {
    observation.valid_until <= now
        || observation
            .constraints
            .iter()
            .any(|constraint| match &constraint.reset_at {
                CapacityValue::Reported { value } => {
                    observation.sampled_at < *value && *value <= now
                }
                CapacityValue::Estimated { upper, .. } => {
                    observation.sampled_at < *upper && *upper <= now
                }
                CapacityValue::Unknown { .. } => false,
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(limits: Value) -> crate::harness::codex::ProbeResponse {
        crate::harness::codex::ProbeResponse {
            account: json!({"result":{"account":{"type":"chatgpt","planType":"plus"}}}),
            limits: Some(json!({"result":limits})),
            limits_error: None,
            source_version: "fixture".into(),
        }
    }

    #[test]
    fn keeps_multiple_windows_separate_and_expires_at_reset() {
        let sampled = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let raw = response(
            json!({"rateLimitsByLimitId":{"codex":{"limitId":"codex","primary":{"usedPercent":60.0,"windowDurationMins":300,"resetsAt":1700000120},"secondary":{"usedPercent":10.0,"windowDurationMins":10080,"resetsAt":1700600000},"credits":{"hasCredits":false}}}}),
        );
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.constraints.len(), 2);
        assert_eq!(observation.scarcity, ScarcityState::Constrained);
        assert_eq!(
            observation.valid_until,
            DateTime::from_timestamp(1_700_000_120, 0).unwrap()
        );
    }

    #[test]
    fn null_window_and_unknown_mapping_stay_unknown() {
        let sampled = Utc::now();
        let raw = response(json!({"rateLimitsByLimitId":{"other":{"primary":null}}}));
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.mapping, CapacityMapping::Unknown);
        assert_eq!(observation.scarcity, ScarcityState::Unknown);
        assert!(matches!(
            observation.constraints[0].remaining,
            CapacityValue::Unknown { .. }
        ));
    }

    #[test]
    fn missing_window_in_a_mapped_bucket_is_not_treated_as_unlimited() {
        let sampled = Utc::now();
        let raw =
            response(json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10.0}}}}));
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.mapping, CapacityMapping::Mapped);
        assert_eq!(observation.scarcity, ScarcityState::Unknown);
        assert_eq!(observation.constraints.len(), 2);
    }

    #[test]
    fn already_passed_reset_does_not_fabricate_replenishment_or_probe_loop() {
        let sampled = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let raw = response(
            json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":100.0,"resetsAt":1699999999},"secondary":{"usedPercent":10.0,"resetsAt":1699999999}}}}),
        );
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.scarcity, ScarcityState::Exhausted);
        assert!(!needs_refresh(&observation, sampled));
    }

    #[test]
    fn partial_mapping_and_null_windows_preserve_applicable_restrictions() {
        let now = Utc::now();
        let partial = normalize_codex(
            "pool",
            &["codex".into(), "missing".into()],
            "default",
            now,
            300,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"rateLimitReachedType":"quota","primary":{"usedPercent":10},"secondary":{"usedPercent":10},"credits":{"hasCredits":false}}}}),
            ),
        );
        assert_eq!(partial.mapping, CapacityMapping::Unknown);
        assert_eq!(partial.scarcity, ScarcityState::Exhausted);
        assert!(matches!(
            partial.credits_available,
            CapacityValue::Unknown { .. }
        ));
        assert!(
            partial
                .constraints
                .iter()
                .any(|c| c.kind == "provider_rejection"
                    && matches!(c.scope, CapacityValue::Reported { .. }))
        );
        assert!(
            partial
                .constraints
                .iter()
                .any(|c| c.provider_bucket_id.as_deref() == Some("missing")
                    && matches!(c.remaining, CapacityValue::Unknown { .. }))
        );
        let reserve = normalize_codex(
            "pool",
            &["codex".into()],
            "default",
            now,
            300,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"primary":null,"secondary":{"usedPercent":85}}}}),
            ),
        );
        assert_eq!(reserve.scarcity, ScarcityState::Reserve);
        assert!(launch_block(&reserve, &reserve, "chatgpt-plus", 0).is_some());
        assert!(launch_block(&reserve, &reserve, "chatgpt-plus", 1).is_none());
    }

    #[test]
    fn rejection_survives_unknown_and_reset_until_authoritative_supersession() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state.db");
        let coordinator = crate::admission::AdmissionCoordinator::new(
            &path,
            Duration::from_secs(20),
            Duration::from_secs(1),
        );
        coordinator.register_pool("pool", "openai", "chatgpt-plus", &["codex".into()])?;
        let db = Database::open(&path)?;
        let now = Utc::now();
        let rejected = normalize_codex(
            "pool",
            &["codex".into()],
            "default",
            now,
            1,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"rateLimitReachedType":"quota","primary":{"usedPercent":100,"resetsAt":(now+TimeDelta::seconds(1)).timestamp()},"secondary":{"usedPercent":10}}}}),
            ),
        );
        db.append_capacity_observation(&rejected)?;
        let unknown = unknown_observation(
            "pool",
            "default",
            now + TimeDelta::seconds(2),
            300,
            "failed after reset",
        );
        db.append_capacity_observation(&unknown)?;
        assert_eq!(
            resolved_scarcity(db.connection(), "pool", now + TimeDelta::seconds(3))?,
            ScarcityState::Exhausted
        );
        let refreshed = normalize_codex(
            "pool",
            &["codex".into()],
            "default",
            now + TimeDelta::seconds(4),
            300,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10,"resetsAt":(now+TimeDelta::seconds(100)).timestamp()},"secondary":{"usedPercent":10}}}}),
            ),
        );
        db.append_capacity_observation(&refreshed)?;
        assert_eq!(
            resolved_scarcity(db.connection(), "pool", now + TimeDelta::seconds(5))?,
            ScarcityState::Available
        );
        // Superseded rejection never resurrects when the successful sample expires.
        db.append_capacity_observation(&unknown_observation(
            "pool",
            "default",
            now + TimeDelta::seconds(400),
            300,
            "offline",
        ))?;
        assert_eq!(
            resolved_scarcity(db.connection(), "pool", now + TimeDelta::seconds(401))?,
            ScarcityState::Unknown
        );
        // Historical observations encoded rejection only in the aggregate.
        // A changed display/mapping must not make supersession impossible.
        let mut legacy = refreshed.clone();
        legacy.id = Ulid::new().to_string();
        legacy.sampled_at = now + TimeDelta::seconds(500);
        legacy.valid_until = now + TimeDelta::seconds(800);
        legacy.scarcity = ScarcityState::Exhausted;
        for constraint in &mut legacy.constraints {
            constraint.reset_at = CapacityValue::unknown("legacy provider reset absent");
        }
        db.append_capacity_observation(&legacy)?;
        assert_eq!(
            resolved_scarcity(db.connection(), "pool", now + TimeDelta::seconds(501))?,
            ScarcityState::Exhausted
        );
        coordinator.register_pool(
            "expanded",
            "openai",
            "chatgpt-plus",
            &["codex".into(), "additional".into()],
        )?;
        let mut superseding = legacy;
        superseding.id = Ulid::new().to_string();
        superseding.pool_id = "expanded".into();
        superseding.sampled_at = now + TimeDelta::seconds(502);
        superseding.scarcity = ScarcityState::Available;
        db.append_capacity_observation(&superseding)?;
        assert_ne!(
            resolved_scarcity(db.connection(), "pool", now + TimeDelta::seconds(503))?,
            ScarcityState::Exhausted
        );
        Ok(())
    }

    #[test]
    fn expired_short_window_cannot_erase_exhausted_long_window() {
        let sampled = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let raw = response(json!({"rateLimitsByLimitId":{"codex":{
            "primary":{"usedPercent":20.0,"resetsAt":1699999999},
            "secondary":{"usedPercent":100.0,"resetsAt":1700003600}
        }}}));
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.constraints.len(), 2);
        assert_eq!(observation.scarcity, ScarcityState::Exhausted);
    }

    #[test]
    fn rejected_evidence_does_not_become_the_next_authorized_baseline() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("dispatch.db"))?;
        db.connection().execute(
            "INSERT INTO resource_pools(id,provider,funding_source,provider_buckets_json,max_active,next_fence,created_at,updated_at,canonical_identity) VALUES('pool','openai','chatgpt-plus','[\"codex\"]',1,0,?1,?1,'canonical')",
            [Utc::now().to_rfc3339()],
        )?;
        let mut first = normalize_codex(
            "pool",
            &["codex".into()],
            "default",
            Utc::now() - TimeDelta::seconds(2),
            300,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10.0},"secondary":{"usedPercent":10.0},"credits":{"hasCredits":false}}}}),
            ),
        );
        first.id = "authorization-baseline-a".into();
        first.funding_identity = CapacityValue::Reported {
            value: "account-a".into(),
        };
        db.append_capacity_observation(&first)?;
        let first_authorization =
            authorize_observation(&db, "pool", &first, "route", 1, "chatgpt-plus")?;
        let other_route =
            authorize_observation(&db, "pool", &first, "other-route", 1, "chatgpt-plus")?;
        assert_ne!(first_authorization.id, other_route.id);
        let mut conflicting = first.clone();
        conflicting.id = "authorization-conflict-b".into();
        conflicting.sampled_at = first.sampled_at + TimeDelta::seconds(1);
        conflicting.funding_identity = CapacityValue::Reported {
            value: "account-b".into(),
        };
        db.append_capacity_observation(&conflicting)?;
        assert!(
            authorize_observation(&db, "pool", &conflicting, "route", 1, "chatgpt-plus").is_err()
        );
        assert!(
            authorize_observation(&db, "pool", &conflicting, "route", 1, "chatgpt-plus").is_err()
        );
        assert!(
            authorize_observation(&db, "pool", &conflicting, "route", 2, "chatgpt-plus").is_ok()
        );
        Ok(())
    }

    #[test]
    fn unknown_probe_after_known_evidence_preserves_authorization_without_claiming_facts()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("dispatch.db"))?;
        db.connection().execute(
            "INSERT INTO resource_pools(id,provider,funding_source,provider_buckets_json,max_active,next_fence,created_at,updated_at,canonical_identity) VALUES('pool','openai','chatgpt-plus','[]',1,0,?1,?1,'canonical')",
            [Utc::now().to_rfc3339()],
        )?;
        let mut known = normalize_codex(
            "pool",
            &["codex".into()],
            "default",
            Utc::now(),
            300,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10.0},"secondary":{"usedPercent":10.0},"credits":{"hasCredits":false}}}}),
            ),
        );
        known.funding_identity = CapacityValue::Reported {
            value: "account-a".into(),
        };
        db.append_capacity_observation(&known)?;
        authorize_observation(&db, "pool", &known, "route", 1, "chatgpt-plus")?;
        let unknown = unknown_observation("pool", "default", Utc::now(), 300, "probe failure");
        db.append_capacity_observation(&unknown)?;
        assert!(authorize_observation(&db, "pool", &unknown, "route", 1, "chatgpt-plus").is_ok());
        assert_eq!(unknown.scarcity, ScarcityState::Unknown);
        let mut conflicting = known.clone();
        conflicting.id = Ulid::new().to_string();
        conflicting.funding_identity = CapacityValue::Reported {
            value: "changed-account".into(),
        };
        db.append_capacity_observation(&conflicting)?;
        assert!(
            authorize_observation(&db, "pool", &conflicting, "route", 1, "chatgpt-plus").is_err(),
            "unknown evidence must not erase the last validated known identity"
        );
        Ok(())
    }

    #[test]
    fn exhaustion_and_paid_credit_changes_are_not_hidden() {
        let sampled = Utc::now();
        let raw = response(
            json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":100.0},"credits":{"hasCredits":true}}}}),
        );
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.scarcity, ScarcityState::Exhausted);
        assert_eq!(
            funding_change(&observation, "chatgpt-plus-included").as_deref(),
            Some("paid credits are now available")
        );

        let raw = response(
            json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10.0},"secondary":{"usedPercent":100.0},"credits":{"hasCredits":false}}}}),
        );
        let long_window = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(long_window.scarcity, ScarcityState::Exhausted);
    }

    #[test]
    fn stale_and_estimated_reset_boundaries_request_refresh_without_replenishing() {
        let now = Utc::now();
        let mut observation = unknown_observation(
            "pool",
            "default",
            now - TimeDelta::seconds(10),
            300,
            "fixture",
        );
        observation.scarcity = ScarcityState::Reserve;
        observation.constraints[0].reset_at = CapacityValue::Estimated {
            lower: now - TimeDelta::seconds(5),
            upper: now,
            method: "provider rounded reset".into(),
            samples: 2,
        };
        assert!(needs_refresh(&observation, now));
        assert_eq!(observation.scarcity, ScarcityState::Reserve);
        observation.valid_until = now - TimeDelta::seconds(1);
        assert!(!usable_now(&observation, now));
    }

    #[test]
    fn provider_rejection_exhausts_only_a_mapped_pool() {
        let sampled = Utc::now();
        let raw = response(
            json!({"rateLimitsByLimitId":{"codex":{"rateLimitReachedType":"primary","primary":{"usedPercent":75.0}}}}),
        );
        let observation = normalize_codex("pool", &["codex".into()], "default", sampled, 300, raw);
        assert_eq!(observation.scarcity, ScarcityState::Exhausted);
        assert_eq!(observation.attributable_attempt_id, None);
        assert_eq!(observation.attribution, "pool_level_mixed_activity");
    }

    #[test]
    fn changed_funding_and_window_identity_invalidate_prior_observation() {
        let now = Utc::now();
        let mut previous = unknown_observation("pool", "default", now, 300, "fixture");
        previous.mapping = CapacityMapping::Mapped;
        previous.funding_identity = CapacityValue::Reported {
            value: "one".into(),
        };
        previous.constraints[0].provider_bucket_id = Some("codex".into());
        previous.constraints[0].window_id = Some("primary".into());
        let mut current = previous.clone();
        current.funding_identity = CapacityValue::Reported {
            value: "two".into(),
        };
        assert_eq!(
            observation_identity_change(Some(&previous), &current).as_deref(),
            Some("authenticated funding identity changed")
        );
        current.funding_identity = previous.funding_identity.clone();
        current.constraints[0].window_id = Some("secondary".into());
        assert_eq!(
            observation_identity_change(Some(&previous), &current).as_deref(),
            Some("provider allowance window identity changed")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_timeout_is_an_expiring_unknown_observation() -> Result<()> {
        use std::{fs, os::unix::fs::PermissionsExt};

        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("codex");
        fs::write(&executable, "#!/bin/sh\nsleep 2\n")?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
        let config = CapacityConfig {
            probe_timeout_secs: 1,
            ..CapacityConfig::default()
        };
        let observation = probe_codex(&executable, "pool", &["codex".into()], "default", &config)
            .await
            .observation;
        assert_eq!(observation.scarcity, ScarcityState::Unknown);
        assert!(
            matches!(observation.auth_mode, CapacityValue::Unknown { ref reason } if reason.contains("probe timeout"))
        );
        assert!(observation.valid_until > observation.sampled_at);
        Ok(())
    }

    #[tokio::test]
    async fn failed_probe_after_known_telemetry_stays_unknown_without_erasing_identity()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("dispatch.db"))?;
        db.connection().execute(
            "INSERT INTO resource_pools(id,provider,funding_source,provider_buckets_json,max_active,next_fence,created_at,updated_at,canonical_identity) VALUES('pool','openai','chatgpt-plus','[\"codex\"]',1,0,?1,?1,'canonical')",
            [Utc::now().to_rfc3339()],
        )?;
        let mut known = normalize_codex(
            "pool",
            &["codex".into()],
            "default",
            Utc::now(),
            300,
            response(
                json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10.0},"secondary":{"usedPercent":10.0},"credits":{"hasCredits":false}}}}),
            ),
        );
        known.funding_identity = CapacityValue::Reported {
            value: "account-a".into(),
        };
        db.append_capacity_observation(&known)?;
        authorize_observation(&db, "pool", &known, "route", 1, "chatgpt-plus")?;

        let failed = probe_codex(
            &temp.path().join("missing-codex"),
            "pool",
            &["codex".into()],
            "default",
            &CapacityConfig::default(),
        )
        .await
        .observation;
        assert_eq!(failed.scarcity, ScarcityState::Unknown);
        assert!(matches!(failed.auth_mode, CapacityValue::Unknown { .. }));
        db.append_capacity_observation(&failed)?;
        authorize_observation(&db, "pool", &failed, "route", 1, "chatgpt-plus")?;

        let mut conflicting = known;
        conflicting.id = Ulid::new().to_string();
        conflicting.funding_identity = CapacityValue::Reported {
            value: "account-b".into(),
        };
        db.append_capacity_observation(&conflicting)?;
        assert!(
            authorize_observation(&db, "pool", &conflicting, "route", 1, "chatgpt-plus").is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn account_facts_survive_rate_limit_probe_failure() -> Result<()> {
        use std::{fs, os::unix::fs::PermissionsExt};

        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("codex");
        fs::write(
            &executable,
            "#!/bin/sh\nwhile IFS= read -r line; do\ncase \"$line\" in\n*'\"id\":0'*) printf '%s\\n' '{\"id\":0,\"result\":{\"userAgent\":\"fixture\"}}' ;;\n*'\"id\":1'*) printf '%s\\n' '{\"id\":1,\"result\":{\"account\":{\"type\":\"chatgpt\",\"planType\":\"plus\",\"id\":\"account-a\"}}}' ;;\n*'\"id\":2'*) printf '%s\\n' '{\"id\":2,\"error\":{\"message\":\"quota unavailable\"}}' ;;\nesac\ndone\n",
        )?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
        let result = probe_codex(
            &executable,
            "pool",
            &["codex".into()],
            "default",
            &CapacityConfig::default(),
        )
        .await;
        assert_eq!(
            result.observation.auth_mode,
            CapacityValue::Reported {
                value: "chatgpt".into()
            }
        );
        assert_eq!(
            result.observation.plan_type,
            CapacityValue::Reported {
                value: "plus".into()
            }
        );
        assert_eq!(result.observation.scarcity, ScarcityState::Unknown);
        assert!(matches!(
            result.observation.constraints[0].remaining,
            CapacityValue::Unknown { ref reason } if reason.contains("rate-limit probe failure")
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_is_read_only_and_never_consumes_credits_or_resets() -> Result<()> {
        use std::{fs, os::unix::fs::PermissionsExt};

        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("codex");
        let requests = temp.path().join("requests.jsonl");
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{}'\n  case \"$line\" in\n    *'\"id\":0'*) printf '%s\\n' '{{\"id\":0,\"result\":{{\"userAgent\":\"fixture\"}}}}' ;;\n    *'\"id\":1'*) printf '%s\\n' '{{\"id\":1,\"result\":{{\"account\":{{\"type\":\"chatgpt\",\"planType\":\"plus\"}}}}}}' ;;\n    *'\"id\":2'*) printf '%s\\n' '{{\"id\":2,\"result\":{{\"rateLimitsByLimitId\":{{\"codex\":{{\"primary\":{{\"usedPercent\":10}},\"secondary\":{{\"usedPercent\":10}},\"credits\":{{\"hasCredits\":false}}}}}}}}}}' ;;\n  esac\ndone\n",
                requests.display()
            ),
        )?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
        let result = probe_codex(
            &executable,
            "pool",
            &["codex".into()],
            "default",
            &CapacityConfig::default(),
        )
        .await;
        assert_eq!(result.observation.scarcity, ScarcityState::Available);
        let requests = fs::read_to_string(requests)?;
        assert!(requests.contains("account/read"));
        assert!(requests.contains("account/rateLimits/read"));
        assert!(!requests.contains("reset/consume"));
        assert!(!requests.contains("credits/buy"));
        Ok(())
    }
}
