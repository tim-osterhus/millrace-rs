//! Durable scheduler lane helpers.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    compiler::{CompiledRunPlan, PlaneConcurrencyPolicyDefinition},
    contracts::{
        ActiveRunState, LaneRuntimeState, LaneRuntimeStatus, Plane, RuntimeSnapshot, Timestamp,
        WorkflowPlaneSchedulerPolicyDefinition,
    },
};

use super::supervisor::can_dispatch_plane;

/// Return the compact launch fingerprint persisted on active runtime records.
#[must_use]
pub fn compiled_plan_fingerprint_for_runtime(compiled_plan: &CompiledRunPlan) -> String {
    let payload =
        serde_json::to_value(&compiled_plan.compile_input_fingerprint).unwrap_or(Value::Null);
    let encoded = serde_json::to_vec(&sort_json_value(payload)).unwrap_or_default();
    let digest = Sha256::digest(encoded);
    format!("compile-input-{}", hex_prefix(&digest, 16))
}

/// Return the default lane id for one plane.
#[must_use]
pub fn default_lane_id_for_plane(plane: Plane) -> String {
    format!("{}.main", plane.as_str())
}

/// Return the compiled lane used for a plane.
#[must_use]
pub fn lane_id_for_plane(compiled_plan: Option<&CompiledRunPlan>, plane: Plane) -> String {
    compiled_plan
        .and_then(|plan| plan.lane_policy.as_ref())
        .and_then(|policy| {
            policy
                .lanes
                .iter()
                .find(|lane| lane.plane == plane)
                .map(|lane| lane.lane_id.clone())
        })
        .unwrap_or_else(|| default_lane_id_for_plane(plane))
}

/// Return deterministic supervisor lane order for the active compiled plan.
#[must_use]
pub fn lane_dispatch_order(compiled_plan: Option<&CompiledRunPlan>) -> Vec<String> {
    let Some(policy) = compiled_plan.and_then(|plan| plan.lane_policy.as_ref()) else {
        return [Plane::Planning, Plane::Execution, Plane::Learning]
            .into_iter()
            .map(default_lane_id_for_plane)
            .collect();
    };
    let mut lane_ids = policy
        .lanes
        .iter()
        .map(|lane| lane.lane_id.clone())
        .collect::<Vec<_>>();
    lane_ids.sort();
    let mut ordered = Vec::new();
    for plane in [Plane::Planning, Plane::Execution, Plane::Learning] {
        let default_lane = default_lane_id_for_plane(plane);
        if let Some(index) = lane_ids.iter().position(|lane_id| lane_id == &default_lane) {
            ordered.push(lane_ids.remove(index));
        }
    }
    ordered.extend(lane_ids);
    ordered
}

/// Ensure a snapshot has lane state for every lane declared by the compiled plan.
pub fn ensure_snapshot_lanes(snapshot: &mut RuntimeSnapshot, compiled_plan: &CompiledRunPlan) {
    let fingerprint = compiled_plan_fingerprint_for_runtime(compiled_plan);
    snapshot.compiled_plan_fingerprint = fingerprint.clone();
    let declared_lanes = declared_lane_specs(compiled_plan);
    let active_runs_by_lane = active_runs_by_lane(snapshot.active_runs_by_plane.values());

    for (lane_id, plane) in declared_lanes {
        let active_runs = active_runs_by_lane
            .get(&lane_id)
            .cloned()
            .unwrap_or_default();
        if !active_runs.is_empty() {
            snapshot.lanes_by_id.insert(
                lane_id.clone(),
                lane_state_for_active_runs(&lane_id, plane, &active_runs),
            );
            continue;
        }

        let existing = snapshot.lanes_by_id.get(&lane_id).cloned();
        let mut lane_state = existing.unwrap_or_else(|| LaneRuntimeState {
            lane_id: lane_id.clone(),
            plane,
            status: LaneRuntimeStatus::Idle,
            compiled_plan_id: compiled_plan.compiled_plan_id.clone(),
            compiled_plan_fingerprint: fingerprint.clone(),
            active_run_ids: Vec::new(),
            active_work_refs: Vec::new(),
            pause_requested: false,
            stop_requested: false,
            drain_requested: false,
            mutation_lock_refs: Vec::new(),
            completion_target_refs: Vec::new(),
            failure_counter_refs: Vec::new(),
            last_claim_attempt_at: None,
            last_terminal_outcome: None,
        });
        lane_state.plane = plane;
        lane_state.compiled_plan_id = compiled_plan.compiled_plan_id.clone();
        lane_state.compiled_plan_fingerprint = fingerprint.clone();
        if lane_state.status == LaneRuntimeStatus::Active {
            lane_state.status = LaneRuntimeStatus::Idle;
        }
        lane_state.active_run_ids.clear();
        lane_state.active_work_refs.clear();
        snapshot.lanes_by_id.insert(lane_id, lane_state);
    }
}

/// Mark the lane owning one active run as active.
pub fn snapshot_with_lane_active_run(snapshot: &mut RuntimeSnapshot, active_run: &ActiveRunState) {
    let lane_id = if active_run.lane_id.trim().is_empty() {
        default_lane_id_for_plane(active_run.plane)
    } else {
        active_run.lane_id.clone()
    };
    snapshot.lanes_by_id.insert(
        lane_id.clone(),
        lane_state_for_active_runs(&lane_id, active_run.plane, std::slice::from_ref(active_run)),
    );
}

/// Remove one active run from its lane state, leaving the lane idle when empty.
pub fn snapshot_without_lane_active_run(
    snapshot: &mut RuntimeSnapshot,
    lane_id: &str,
    run_id: &str,
    now: &Timestamp,
    terminal_outcome: Option<&str>,
) {
    let Some(existing) = snapshot.lanes_by_id.get_mut(lane_id) else {
        return;
    };
    existing
        .active_run_ids
        .retain(|active_id| active_id != run_id);
    if existing.active_run_ids.is_empty() {
        existing.status = LaneRuntimeStatus::Idle;
        existing.active_work_refs.clear();
    }
    existing.last_claim_attempt_at = Some(now.clone());
    existing.last_terminal_outcome = terminal_outcome.map(ToOwned::to_owned);
}

/// Return whether a candidate lane may start beside active lanes.
#[must_use]
pub fn can_dispatch_lane<I>(
    scheduler_policy: Option<&WorkflowPlaneSchedulerPolicyDefinition>,
    concurrency_policy: Option<&PlaneConcurrencyPolicyDefinition>,
    active_lane_ids: I,
    candidate_lane_id: &str,
) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let Some(policy) = scheduler_policy else {
        let active_count = active_lane_ids
            .into_iter()
            .filter(|lane_id| !lane_id.as_ref().trim().is_empty())
            .count();
        return active_count == 0;
    };
    let Some(candidate_lane) = policy
        .lanes
        .iter()
        .find(|lane| lane.lane_id == candidate_lane_id)
    else {
        return false;
    };

    let mut active_ids = Vec::new();
    for lane_id in active_lane_ids {
        let lane_id = lane_id.as_ref().trim();
        if lane_id.is_empty() {
            continue;
        }
        active_ids.push(lane_id.to_owned());
    }
    let same_lane_count = active_ids
        .iter()
        .filter(|lane_id| lane_id.as_str() == candidate_lane_id)
        .count() as u64;
    if same_lane_count >= candidate_lane.max_active_runs {
        return false;
    }

    let mut active_planes = Vec::new();
    for active_lane_id in active_ids {
        let Some(active_lane) = policy
            .lanes
            .iter()
            .find(|lane| lane.lane_id == active_lane_id)
        else {
            return false;
        };
        if active_lane.lane_id == candidate_lane_id {
            continue;
        }
        active_planes.push(active_lane.plane);
        if !lane_policy_allows(policy, &candidate_lane.lane_id, &active_lane.lane_id)
            || !lane_policy_allows(policy, &active_lane.lane_id, &candidate_lane.lane_id)
        {
            return false;
        }
    }
    can_dispatch_plane(concurrency_policy, active_planes, candidate_lane.plane)
}

fn declared_lane_specs(compiled_plan: &CompiledRunPlan) -> Vec<(String, Plane)> {
    if let Some(policy) = &compiled_plan.lane_policy {
        let mut lanes = policy
            .lanes
            .iter()
            .map(|lane| (lane.lane_id.clone(), lane.plane))
            .collect::<Vec<_>>();
        lanes.sort_by(|left, right| left.0.cmp(&right.0));
        return lanes;
    }
    compiled_plan
        .loop_ids_by_plane
        .keys()
        .copied()
        .map(|plane| (default_lane_id_for_plane(plane), plane))
        .collect()
}

fn active_runs_by_lane<'a, I>(active_runs: I) -> HashMap<String, Vec<ActiveRunState>>
where
    I: IntoIterator<Item = &'a ActiveRunState>,
{
    let mut grouped: HashMap<String, Vec<ActiveRunState>> = HashMap::new();
    for active_run in active_runs {
        let lane_id = if active_run.lane_id.trim().is_empty() {
            default_lane_id_for_plane(active_run.plane)
        } else {
            active_run.lane_id.clone()
        };
        grouped.entry(lane_id).or_default().push(active_run.clone());
    }
    grouped
}

fn lane_state_for_active_runs(
    lane_id: &str,
    plane: Plane,
    active_runs: &[ActiveRunState],
) -> LaneRuntimeState {
    let primary = active_runs
        .first()
        .expect("lane_state_for_active_runs requires active runs");
    LaneRuntimeState {
        lane_id: lane_id.to_owned(),
        plane,
        status: LaneRuntimeStatus::Active,
        compiled_plan_id: primary.compiled_plan_id.clone(),
        compiled_plan_fingerprint: primary.compiled_plan_fingerprint.clone(),
        active_run_ids: active_runs
            .iter()
            .map(|active_run| active_run.run_id.clone())
            .collect(),
        active_work_refs: active_runs.iter().map(active_work_ref).collect(),
        pause_requested: false,
        stop_requested: false,
        drain_requested: false,
        mutation_lock_refs: Vec::new(),
        completion_target_refs: Vec::new(),
        failure_counter_refs: Vec::new(),
        last_claim_attempt_at: None,
        last_terminal_outcome: None,
    }
}

fn active_work_ref(active_run: &ActiveRunState) -> String {
    if let (Some(family_id), Some(work_item_id)) = (
        active_run.work_item_family_id.as_deref(),
        active_run.work_item_id.as_deref(),
    ) {
        return format!("{family_id}:{work_item_id}");
    }
    if let Some(root_spec_id) = active_run.closure_target_root_spec_id.as_deref() {
        return format!("closure_target:{root_spec_id}");
    }
    format!("run:{}", active_run.run_id)
}

fn lane_policy_allows(
    policy: &WorkflowPlaneSchedulerPolicyDefinition,
    first_lane_id: &str,
    second_lane_id: &str,
) -> bool {
    policy.lane_conflict_policies.iter().any(|conflict| {
        conflict
            .lane_ids
            .iter()
            .any(|lane_id| lane_id == first_lane_id)
            && conflict
                .concurrent_with_lane_ids
                .iter()
                .any(|lane_id| lane_id == second_lane_id)
            && (conflict.conflict_scopes.is_empty()
                || conflict
                    .lane_ids
                    .iter()
                    .all(|lane_id| conflict.lock_acquisition_order.contains(lane_id)))
    })
}

fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(sort_json_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, sort_json_value(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        value => value,
    }
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .take(chars)
        .map(|nibble| char::from_digit(nibble as u32, 16).expect("hex nibble"))
        .collect()
}
