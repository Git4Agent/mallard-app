//! Verified transcript facts bound to immutable bytes. Never serialized.

use crate::conversation_codec::{CodecLineage, CodexTranscriptAnalysis, CODEX_LINEAGE_CODEC_KEY};
use crate::payload::PayloadBytes;
use crate::providers::{SessionAdapter, SessionAnalysis};
use mallard_domain::ProviderKey;

#[derive(Clone, Debug)]
pub struct VerifiedTranscript {
    bytes: PayloadBytes,
    provider: ProviderKey,
    codec: String,
    provider_thread_id: String,
    cwd: String,
    lineage_digest: String,
    semantic_digest: String,
}

impl VerifiedTranscript {
    /// Analyze new bytes. Builders use their completed analysis instead.
    pub fn analyze(adapter: &dyn SessionAdapter, bytes: PayloadBytes) -> Result<Self, String> {
        let analysis = adapter.analyze(&bytes);
        Self::from_analysis(adapter, bytes, &analysis)
    }

    pub(crate) fn from_analysis(
        adapter: &dyn SessionAdapter,
        bytes: PayloadBytes,
        analysis: &SessionAnalysis,
    ) -> Result<Self, String> {
        Self::from_parts(
            adapter.descriptor().key,
            adapter.codec_key(),
            bytes,
            analysis.source_provider_session_id.as_deref(),
            analysis.source_cwd.as_deref(),
            &analysis.lineage,
        )
    }

    pub(crate) fn from_codex_analysis(
        bytes: PayloadBytes,
        analysis: &CodexTranscriptAnalysis,
    ) -> Result<Self, String> {
        Self::from_parts(
            crate::providers::codex::provider_key(),
            CODEX_LINEAGE_CODEC_KEY,
            bytes,
            analysis.source_thread_id.as_deref(),
            analysis.source_cwd.as_deref(),
            &analysis.lineage,
        )
    }

    fn from_parts(
        provider: ProviderKey,
        codec: &str,
        bytes: PayloadBytes,
        provider_thread_id: Option<&str>,
        cwd: Option<&str>,
        lineage: &CodecLineage,
    ) -> Result<Self, String> {
        let CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } = lineage
        else {
            return Err("transcript verification requires a complete analysis".to_string());
        };
        Ok(Self {
            bytes,
            provider,
            codec: codec.to_string(),
            provider_thread_id: provider_thread_id
                .ok_or("verified transcript lacks its provider identity")?
                .to_string(),
            cwd: cwd
                .ok_or("verified transcript lacks its project identity")?
                .to_string(),
            lineage_digest: lineage_digest.clone(),
            semantic_digest: semantic_digest.clone(),
        })
    }

    pub fn bytes(&self) -> &PayloadBytes {
        &self.bytes
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn matches_target(
        &self,
        provider: &ProviderKey,
        codec: &str,
        provider_thread_id: Option<&str>,
        project_root: &str,
        canonical_project_root: &str,
        lineage_digest: &str,
        semantic_digest: &str,
        bytes: &PayloadBytes,
    ) -> bool {
        &self.provider == provider
            && self.codec == codec
            && Some(self.provider_thread_id.as_str()) == provider_thread_id
            && (self.cwd == project_root || self.cwd == canonical_project_root)
            && self.lineage_digest == lineage_digest
            && self.semantic_digest == semantic_digest
            && self.bytes.len() == bytes.len()
            && self.bytes.sha256() == bytes.sha256()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::codex::CodexAdapter;

    fn fixture() -> PayloadBytes {
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"verified-thread\",\"cwd\":\"/fixture/project\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"id\":\"m1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hello\"}]}}\n",
        ).as_bytes().to_vec().into()
    }

    #[test]
    fn proof_binds_bytes_and_every_materialization_claim() {
        let proof = VerifiedTranscript::analyze(&CodexAdapter::new(), fixture()).unwrap();
        let matches = |candidate: &VerifiedTranscript, bytes: &PayloadBytes| {
            candidate.matches_target(
                &proof.provider,
                &proof.codec,
                Some(&proof.provider_thread_id),
                "/fixture/project",
                "/canonical/project",
                &proof.lineage_digest,
                &proof.semantic_digest,
                bytes,
            )
        };
        assert!(matches(&proof, proof.bytes()));
        assert!(!matches(
            &proof,
            &PayloadBytes::from(b"different bytes".to_vec())
        ));
        let mut same_length = proof.bytes().to_vec();
        let last = same_length.len() - 2;
        same_length[last] = b' ';
        assert!(!matches(&proof, &same_length.into()));
        for field in 0..6 {
            let mut changed = proof.clone();
            match field {
                0 => changed.provider = crate::providers::claude::provider_key(),
                1 => changed.codec.push_str("-other"),
                2 => changed.provider_thread_id.push_str("-other"),
                3 => changed.cwd.push_str("-other"),
                4 => changed.lineage_digest = "0".repeat(64),
                5 => changed.semantic_digest = "0".repeat(64),
                _ => unreachable!(),
            }
            assert!(!matches(&changed, proof.bytes()), "claim {field}");
        }
    }

    #[test]
    fn incomplete_analysis_cannot_create_a_proof() {
        assert!(
            VerifiedTranscript::analyze(&CodexAdapter::new(), b"not-json\n".to_vec().into())
                .is_err()
        );
    }
}
