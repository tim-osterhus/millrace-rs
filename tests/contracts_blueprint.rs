use serde_json::{Value, json};

use millrace_ai::contracts::{
    BlueprintCritiqueDocument, BlueprintDraftDocument, BlueprintDraftStatus,
    BlueprintEvaluationDecision, BlueprintEvaluationDocument, BlueprintManifestDocument,
    BlueprintPacketDocument, BlueprintPromotionRecord, BlueprintSourceWorkItemKind,
    RuntimeJsonContract,
};

const NOW: &str = "2026-05-21T00:00:00Z";

fn round_trip<T>(value: Value) -> T
where
    T: RuntimeJsonContract + PartialEq + std::fmt::Debug,
{
    let decoded = T::from_json_value(value).unwrap();
    let serialized = serde_json::to_value(&decoded).unwrap();
    let decoded_again = T::from_json_value(serialized).unwrap();
    assert_eq!(decoded_again, decoded);
    decoded
}

fn manifest_json() -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "blueprint_manifest",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "source_work_item_kind": "spec",
        "source_work_item_id": "spec-root",
        "source_spec_id": "spec-root",
        "draft_ids": ["draft-001"],
        "draft_count": 1,
        "strict_sequence": true,
        "spec_summary": "Build the feature.",
        "decomposition_strategy": "One draft.",
        "global_acceptance_intent": ["all checks pass"],
        "created_at": NOW,
        "created_by": "manager_blueprint"
    })
}

fn draft_json() -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "blueprint_draft",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "source_spec_id": "spec-root",
        "sequence_number": 1,
        "dependency_draft_ids": [],
        "title": "Implement contract models",
        "scope_summary": "Add typed contract models.",
        "target_paths": ["src/contracts/blueprint.rs"],
        "acceptance": ["documents round trip"],
        "context_excerpt": "Relevant spec excerpt.",
        "revision": 0,
        "status": "queued",
        "created_at": NOW,
        "created_by": "manager_blueprint"
    })
}

fn packet_json() -> Value {
    json!({
        "schema_version": "1.0",
        "kind": "blueprint_packet",
        "packet_id": "blueprint-001",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "revision": 1,
        "title": "Contract model packet",
        "implementation_scope": ["Add contracts"],
        "intended_files": ["src/contracts/blueprint.rs"],
        "design_decisions": ["Use serde aliases for legacy field names"],
        "verification_plan": ["cargo test --test contracts_blueprint"],
        "task_acceptance": ["Blueprint packet is valid"],
        "required_checks": ["cargo test --test contracts_blueprint"],
        "risk_notes": ["Schema drift"],
        "created_at": NOW,
        "created_by": "contractor_blueprint"
    })
}

#[test]
fn blueprint_documents_round_trip_and_preserve_alias_compatibility() {
    let manifest = round_trip::<BlueprintManifestDocument>(manifest_json());
    assert_eq!(
        manifest.source_work_item_kind,
        BlueprintSourceWorkItemKind::Spec
    );

    let draft = round_trip::<BlueprintDraftDocument>(draft_json());
    assert_eq!(draft.draft_index, 1);
    assert_eq!(draft.current_revision, 0);
    assert_eq!(draft.status, BlueprintDraftStatus::Queued);

    let packet = round_trip::<BlueprintPacketDocument>(packet_json());
    assert_eq!(packet.blueprint_id, "blueprint-001");
    packet.ensure_matches_draft(&draft).unwrap();

    let critique = round_trip::<BlueprintCritiqueDocument>(json!({
        "schema_version": "1.0",
        "kind": "blueprint_critique",
        "critique_id": "critique-001",
        "evaluation_id": "evaluation-001",
        "blueprint_id": "blueprint-001",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "revision": 1,
        "required_changes": ["Tighten verification."],
        "blocking_reason": "Verification is too broad.",
        "created_at": NOW,
        "created_by": "evaluator_blueprint"
    }));
    critique.ensure_matches_packet(&packet).unwrap();

    let rejected = round_trip::<BlueprintEvaluationDocument>(json!({
        "schema_version": "1.0",
        "kind": "blueprint_evaluation",
        "evaluation_id": "evaluation-001",
        "blueprint_id": "blueprint-001",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "decision": "rejected",
        "rubric_findings": ["Needs revision."],
        "critique_id": "critique-001",
        "created_at": NOW,
        "created_by": "evaluator_blueprint"
    }));
    assert_eq!(rejected.decision, BlueprintEvaluationDecision::Rejected);
    rejected.ensure_matches_packet(&packet).unwrap();

    let approved = round_trip::<BlueprintEvaluationDocument>(json!({
        "schema_version": "1.0",
        "kind": "blueprint_evaluation",
        "evaluation_id": "evaluation-002",
        "blueprint_id": "blueprint-001",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "decision": "approved",
        "rubric_findings": ["Ready."],
        "required_task_fields": ["Task-ID", "Acceptance"],
        "created_at": NOW,
        "created_by": "evaluator_blueprint"
    }));
    assert_eq!(approved.decision, BlueprintEvaluationDecision::Approved);

    let promotion = round_trip::<BlueprintPromotionRecord>(json!({
        "schema_version": "1.0",
        "kind": "blueprint_promotion",
        "promotion_id": "promotion-001",
        "blueprint_id": "blueprint-001",
        "evaluation_id": "evaluation-002",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "generated_task_id": "task-001",
        "generated_task_path": "millrace-agents/tasks/queue/task-001.md",
        "approved_blueprint_path": "millrace-agents/blueprints/packets/approved/blueprint-001.json",
        "evaluation_path": "millrace-agents/blueprints/evaluations/evaluation-002.json",
        "promoted_at": NOW,
        "promoted_by": "runtime"
    }));
    promotion.ensure_matches_evaluation(&approved).unwrap();
}

#[test]
fn blueprint_document_invariants_fail_before_runtime_consumers() {
    let mut bad_manifest = manifest_json();
    bad_manifest["draft_count"] = json!(2);
    let error = BlueprintManifestDocument::from_json_value(bad_manifest).unwrap_err();
    assert!(error.to_string().contains("draft_count"));

    let mut bad_draft = draft_json();
    bad_draft["dependency_draft_ids"] = json!(["draft-001"]);
    let error = BlueprintDraftDocument::from_json_value(bad_draft).unwrap_err();
    assert!(error.to_string().contains("depends_on_draft_ids"));

    let mut bad_eval = json!({
        "schema_version": "1.0",
        "kind": "blueprint_evaluation",
        "evaluation_id": "evaluation-bad",
        "blueprint_id": "blueprint-001",
        "draft_id": "draft-001",
        "manifest_id": "manifest-001",
        "root_spec_id": "spec-root",
        "root_idea_id": "idea-root",
        "decision": "rejected",
        "rubric_findings": ["Needs revision."],
        "created_at": NOW,
        "created_by": "evaluator_blueprint"
    });
    let error = BlueprintEvaluationDocument::from_json_value(bad_eval.take()).unwrap_err();
    assert!(error.to_string().contains("critique_id"));
}
