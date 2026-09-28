use crate::{
    BundleId, ConversationReviewRow, HostId, PlanId, ProjectFileSyncEligibility, RemoteContinuity,
    ReplicaId, RestoreAction, RestorePlan, StorageId,
};
use serde::{Deserialize, Serialize};

/// Pull comparison exchanged with the frontend. Conversation rows carry
/// selection IDs; executable conversation actions stay in the backend.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullReview {
    /// The most bytes one Pull Apply installs. The review meter refuses a
    /// selection past it; Apply recomputes the selection from the pinned
    /// manifest and refuses the same way before its first write.
    pub apply_limit_bytes: u64,
    pub schema_version: u32,
    pub plan_id: PlanId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub host_id: HostId,
    pub replica_id: ReplicaId,
    pub expected_host_identity_revision: u64,
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
    pub binding_revision: u64,
    pub created_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub actions: Vec<RestoreAction>,
    #[serde(default)]
    pub conversation_review: Vec<ConversationReviewRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_content_eligibility: Option<ProjectFileSyncEligibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_continuity: Option<RemoteContinuity>,
}

impl PullReview {
    /// The review the frontend sees, carrying the Apply limit the caller
    /// enforces. The domain does not own that constant.
    pub fn from_plan(plan: RestorePlan, apply_limit_bytes: u64) -> Result<Self, String> {
        plan.validate_review()?;
        Ok(Self {
            apply_limit_bytes,
            schema_version: plan.schema_version,
            plan_id: plan.plan_id,
            storage_id: plan.storage_id,
            bundle_id: plan.bundle_id,
            host_id: plan.host_id,
            replica_id: plan.replica_id,
            expected_host_identity_revision: plan.expected_host_identity_revision,
            revision: plan.revision,
            commit_id: plan.commit_id,
            manifest_sha256: plan.manifest_sha256,
            binding_revision: plan.binding_revision,
            created_at: plan.created_at,
            expires_at: plan.expires_at,
            actions: plan.actions,
            conversation_review: plan.conversation_review,
            project_content_eligibility: plan.project_content_eligibility,
            remote_continuity: plan.remote_continuity,
        })
    }
}

impl From<PullReview> for RestorePlan {
    fn from(review: PullReview) -> Self {
        Self {
            schema_version: review.schema_version,
            plan_id: review.plan_id,
            storage_id: review.storage_id,
            bundle_id: review.bundle_id,
            host_id: review.host_id,
            replica_id: review.replica_id,
            expected_host_identity_revision: review.expected_host_identity_revision,
            revision: review.revision,
            commit_id: review.commit_id,
            manifest_sha256: review.manifest_sha256,
            binding_revision: review.binding_revision,
            created_at: review.created_at,
            expires_at: review.expires_at,
            actions: review.actions,
            conversation_review: review.conversation_review,
            project_content_eligibility: review.project_content_eligibility,
            remote_continuity: review.remote_continuity,
            conversation_actions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn review_plan() -> RestorePlan {
        serde_json::from_value(json!({
            "schema_version": 1,
            "plan_id": "plan-review",
            "storage_id": "storage-test",
            "bundle_id": "00112233445566778899aabbccddeeff",
            "host_id": "ffeeddccbbaa99887766554433221100",
            "replica_id": "replica-test",
            "expected_host_identity_revision": 3,
            "revision": 4,
            "commit_id": "commit-test",
            "manifest_sha256": "a".repeat(64),
            "binding_revision": 2,
            "created_at": 10,
            "expires_at": 20,
            "actions": [],
            "conversation_actions": [],
            "conversation_review": [{
                "stored_without_credential_redaction": false,
                "provider": "codex",
                "thread_id": null,
                "cloud_thread_id": "cloudthread-test",
                "resource_id": "codex:conversation:cloudthread-test",
                "action": "create_local",
                "action_id": "selection-test",
                "display_title": "Stored conversation"
            }]
        }))
        .unwrap()
    }

    const LIMIT: u64 = 512 * 1024 * 1024;

    #[test]
    fn review_round_trip_preserves_approval_pins_and_rows_without_executable_actions() {
        let plan = review_plan();
        let review = PullReview::from_plan(plan.clone(), LIMIT).unwrap();
        let json = serde_json::to_value(&review).unwrap();
        assert!(json.get("conversation_actions").is_none());
        assert!(json["conversation_review"][0].get("pins").is_none());
        // The limit rides the review out and is dropped on the way back in:
        // Apply enforces the constant, never a number the frontend sent.
        assert_eq!(json["apply_limit_bytes"], LIMIT);
        let decoded: PullReview = serde_json::from_value(json).unwrap();
        let restored = RestorePlan::from(decoded);
        restored.validate_review().unwrap();
        assert_eq!(restored, plan);
    }

    #[test]
    fn review_rejects_the_obsolete_executable_action_field() {
        let review = PullReview::from_plan(review_plan(), LIMIT).unwrap();
        let mut json = serde_json::to_value(review).unwrap();
        json["conversation_actions"] = json!([]);
        let error = serde_json::from_value::<PullReview>(json).unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown field `conversation_actions`"));
    }

    #[test]
    fn review_conversion_preserves_validation_of_lifetime_and_unique_selection_ids() {
        let mut plan = review_plan();
        plan.expires_at = plan.created_at - 1;
        assert!(PullReview::from_plan(plan, LIMIT).is_err());

        let mut plan = review_plan();
        plan.conversation_review
            .push(plan.conversation_review[0].clone());
        assert!(PullReview::from_plan(plan, LIMIT)
            .unwrap_err()
            .contains("duplicate restore selection"));
    }
}
