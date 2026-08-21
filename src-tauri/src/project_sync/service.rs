//! Non-Tauri application boundary for project setup and reviewed sync.
//!
//! The desktop command layer and external integration tests use this service
//! so setup, Push approval, publication, Pull planning, and Pull apply all
//! cross the same production orchestration path.

use super::commands::{self, ProjectContentPushReview};
use mallard_domain::{
    ActionId, BundleId, BundleRecipe, BundleSnapshot, LocalProjectId, PlanId, ProjectBinding,
    ProjectSetupDraft, ResourceId, SetupDraftId, StorageId,
};
use mallard_infrastructure::bundle_engine::FetchProgress;
use mallard_infrastructure::persistence::LocalStateRepository;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

pub use super::commands::{
    CreateSetupDraftResult, ProjectDetail, ProjectOperationResult, PushDestinationPullReason,
    PushDestinationStatus, PushReviewContext, RestoreResult, SetupDraftInspection,
    ThreadSyncComparison, ThreadSyncState,
};
pub use mallard_infrastructure::providers::{
    SessionEventDetail, SessionTimelineEventKind, SessionToolCategory,
};
pub use mallard_infrastructure::session_detail::{
    ProjectSessionDetail, ProjectSessionTimelinePage, ProjectSessionTimelineRequest,
    SessionDetailError, SessionDetailErrorCode,
};

/// The complete approval captured by a reviewed Push.
///
/// Callers cannot publish a conversation merely by selecting its resource:
/// the approval carries the short-lived plan ID and the exact reviewed action
/// IDs that the publication path validates again immediately before writing.
#[derive(Clone, Debug)]
pub struct PushApproval {
    pub recipe: BundleRecipe,
    pub project_content_review_token: Option<String>,
    pub project_content_removal_ids: Vec<ResourceId>,
    pub acknowledged_warning_digests: Vec<String>,
    pub conversation_push_plan_id: Option<PlanId>,
    pub approved_conversation_action_ids: Vec<ActionId>,
}

impl PushApproval {
    /// Build the same default selection presented by Push review: retain the
    /// destination recipe and select every currently selectable conversation.
    pub fn recommended(review: &PushReviewContext, storage_id: &StorageId) -> Self {
        let recipe = commands::recommended_review_recipe(review, storage_id);
        let selected_resources = recipe.entries.keys().cloned().collect::<BTreeSet<_>>();
        let approved_conversation_action_ids = review
            .thread_comparison
            .as_ref()
            .into_iter()
            .flat_map(|comparison| comparison.entries.iter())
            .filter(|entry| {
                entry.push_decision.selectable && selected_resources.contains(&entry.resource_id)
            })
            .filter_map(|entry| {
                review
                    .conversation_action_ids
                    .get(&entry.resource_id)
                    .cloned()
            })
            .collect();
        Self {
            recipe,
            project_content_review_token: None,
            project_content_removal_ids: Vec::new(),
            acknowledged_warning_digests: Vec::new(),
            conversation_push_plan_id: review.conversation_push_plan_id.clone(),
            approved_conversation_action_ids,
        }
    }
}

/// Production project-sync use cases with an explicitly selected machine
/// repository. Tauri resolves the repository from the user's home; tests and
/// other hosts can safely inject an isolated repository instead.
#[derive(Clone, Copy)]
pub struct ProjectSyncService<'a> {
    repository: &'a LocalStateRepository,
}

impl<'a> ProjectSyncService<'a> {
    pub fn new(repository: &'a LocalStateRepository) -> Self {
        Self { repository }
    }

    pub fn create_setup_draft(
        &self,
        project_root: &Path,
    ) -> Result<CreateSetupDraftResult, String> {
        let project_root = project_root
            .to_str()
            .ok_or_else(|| "project root is not valid UTF-8".to_string())?;
        commands::create_setup_draft_with_repository(self.repository, project_root)
    }

    pub fn update_setup_draft(
        &self,
        draft: ProjectSetupDraft,
    ) -> Result<ProjectSetupDraft, String> {
        commands::update_setup_draft_with_repository(self.repository, draft)
    }

    pub fn inspect_setup_draft(
        &self,
        draft_id: &SetupDraftId,
    ) -> Result<SetupDraftInspection, String> {
        commands::inspect_setup_draft_with_repository(self.repository, draft_id)
    }

    pub fn finalize_project_setup(
        &self,
        draft_id: &SetupDraftId,
        expected_revision: u64,
    ) -> Result<ProjectDetail, String> {
        commands::finalize_project_setup_with_repository(
            self.repository,
            draft_id,
            expected_revision,
        )
    }

    pub fn prepare_push_review(
        &self,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
    ) -> Result<PushReviewContext, String> {
        self.prepare_push_review_with_phases(local_project_id, storage_id, &mut |_, _| {})
    }

    pub fn prepare_push_review_with_phases(
        &self,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        on_phase: &mut dyn FnMut(&str, Duration),
    ) -> Result<PushReviewContext, String> {
        commands::prepare_push_review_with_progress(
            self.repository,
            local_project_id,
            storage_id,
            on_phase,
        )
    }

    pub fn push_reviewed(
        &self,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        approval: PushApproval,
    ) -> Result<ProjectOperationResult, String> {
        self.push_reviewed_with_progress(
            local_project_id,
            storage_id,
            approval,
            &mut |_| {},
            &mut |_, _| {},
        )
    }

    pub fn push_reviewed_with_progress(
        &self,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
        approval: PushApproval,
        on_progress: &mut dyn FnMut(FetchProgress),
        on_phase: &mut dyn FnMut(&str, Duration),
    ) -> Result<ProjectOperationResult, String> {
        commands::push_bundle_reviewed_with_progress(
            self.repository,
            local_project_id,
            storage_id,
            approval.recipe,
            ProjectContentPushReview {
                review_token: approval.project_content_review_token,
                removal_ids: approval.project_content_removal_ids.into_iter().collect(),
                acknowledged_warning_digests: approval
                    .acknowledged_warning_digests
                    .into_iter()
                    .collect(),
            },
            approval.conversation_push_plan_id,
            approval.approved_conversation_action_ids,
            on_progress,
            on_phase,
        )
    }

    pub fn plan_bundle_restore(
        &self,
        storage_id: &StorageId,
        bundle_id: &BundleId,
        binding: &ProjectBinding,
    ) -> Result<mallard_domain::RestorePlan, String> {
        self.plan_bundle_restore_with_progress(
            storage_id,
            bundle_id,
            binding,
            &mut |_| {},
            &mut |_, _| {},
        )
    }

    pub fn plan_bundle_restore_with_progress(
        &self,
        storage_id: &StorageId,
        bundle_id: &BundleId,
        binding: &ProjectBinding,
        on_progress: &mut dyn FnMut(FetchProgress),
        on_phase: &mut dyn FnMut(&str, Duration),
    ) -> Result<mallard_domain::RestorePlan, String> {
        commands::plan_bundle_restore_with_progress(
            self.repository,
            storage_id,
            bundle_id,
            binding,
            on_progress,
            on_phase,
        )
    }

    pub fn apply_bundle_restore(
        &self,
        plan_id: &PlanId,
        approved_action_ids: Vec<ActionId>,
    ) -> Result<RestoreResult, String> {
        self.apply_bundle_restore_with_progress(
            plan_id,
            approved_action_ids,
            &mut |_| {},
            &mut |_, _| {},
        )
    }

    pub fn apply_bundle_restore_with_progress(
        &self,
        plan_id: &PlanId,
        approved_action_ids: Vec<ActionId>,
        on_progress: &mut dyn FnMut(FetchProgress),
        on_phase: &mut dyn FnMut(&str, Duration),
    ) -> Result<RestoreResult, String> {
        commands::apply_bundle_restore_with_progress(
            self.repository,
            plan_id,
            approved_action_ids,
            on_progress,
            on_phase,
        )
    }

    pub fn compare_threads(
        &self,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
    ) -> Result<ThreadSyncComparison, String> {
        commands::get_project_thread_sync_comparison_with_progress(
            self.repository,
            local_project_id,
            storage_id,
            &mut |_, _| {},
        )
    }

    pub fn get_project_session_detail(
        &self,
        local_project_id: &LocalProjectId,
        resource_id: &ResourceId,
    ) -> Result<ProjectSessionDetail, SessionDetailError> {
        commands::get_project_session_detail_with_repository(
            self.repository,
            local_project_id,
            resource_id.as_str(),
        )
    }

    pub fn get_project_session_timeline_page(
        &self,
        local_project_id: &LocalProjectId,
        resource_id: &ResourceId,
        request: ProjectSessionTimelineRequest,
    ) -> Result<ProjectSessionTimelinePage, SessionDetailError> {
        commands::get_project_session_timeline_page_with_repository(
            self.repository,
            local_project_id,
            resource_id.as_str(),
            request,
        )
    }

    pub fn get_project_session_event_detail(
        &self,
        local_project_id: &LocalProjectId,
        resource_id: &ResourceId,
        snapshot_id: &str,
        event_id: &str,
    ) -> Result<SessionEventDetail, SessionDetailError> {
        commands::get_project_session_event_detail_with_repository(
            self.repository,
            local_project_id,
            resource_id.as_str(),
            snapshot_id,
            event_id,
        )
    }

    pub fn inspect_bundle(
        &self,
        storage_id: &StorageId,
        bundle_id: &BundleId,
    ) -> Result<BundleSnapshot, String> {
        let (_, engine) = commands::storage_engine(self.repository, storage_id)?;
        engine.inspect(bundle_id)
    }
}
