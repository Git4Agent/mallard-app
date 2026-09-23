use super::*;
use crate::storage_providers::traits::StorageProbeJournal;
use crate::storage_setup_journal::StorageSetupId;
use mallard_domain::BoundedConfigObject;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    revision: Option<String>,
    size: u64,
    sha256: String,
}
fn checkpoint(
    journal: &mut dyn StorageProbeJournal,
    revision: Option<String>,
    bytes: &[u8],
) -> Result<()> {
    journal
        .checkpoint(
            BoundedConfigObject::encode(&Recovery {
                revision,
                size: bytes.len() as u64,
                sha256: sha256_hex(bytes),
            })
            .map_err(|_| Error::Local)?,
        )
        .map_err(|_| Error::Local)
}
fn probe_path(setup_id: &str) -> Result<String> {
    StorageSetupId::parse(setup_id).map_err(|_| Error::Invalid)?;
    Ok(format!("/.mallard/v1/_setup_probes/{setup_id}.bin"))
}
impl DropboxObjectStore {
    async fn check_account(&self) -> Result<()> {
        // User-linked token only. No team member selection and no home fallback.
        let response = self
            .send_checked("users/get_current_account", Endpoint::Account, |request| {
                request
                    .header("Content-Type", "application/json")
                    .body("null")
            })
            .await?;
        let body = Self::bounded(response, RPC_LIMIT).await?;
        let account: Value = serde_json::from_slice(&body).map_err(|_| Error::Invalid)?;
        let root = &account["root_info"];
        let home = root["home_namespace_id"].as_str().ok_or(Error::Invalid)?;
        let root_id = root["root_namespace_id"].as_str().ok_or(Error::Invalid)?;
        if account["account_id"].as_str().is_none()
            || self.namespace == home
            || self.namespace == root_id && root[".tag"] != "team"
        {
            return Err(Error::Namespace);
        }
        Ok(())
    }
    async fn cleanup(&self, path: &str, journal: &mut dyn StorageProbeJournal) -> Result<()> {
        let Some(metadata) = self.metadata(path).await? else {
            return Ok(());
        };
        metadata.check(path, false)?;
        let recovery: Recovery = journal.recovery().decode().map_err(|_| Error::Cleanup)?;
        if recovery.revision.as_deref() != Some(metadata.rev.as_str()) || recovery.size > 1024 {
            return Err(Error::Cleanup);
        }
        let Some((_, downloaded, hash)) = self.download(path, 1024).await? else {
            return Ok(());
        };
        if downloaded.rev != metadata.rev
            || downloaded.size != recovery.size
            || hash != recovery.sha256
        {
            return Err(Error::Cleanup);
        }
        match self.delete_path(path, &metadata.rev).await? {
            DeleteOutcome::Deleted | DeleteOutcome::Missing => Ok(()),
            DeleteOutcome::Conflict { .. } => Err(Error::Cleanup),
        }
    }
    async fn probe_write(
        &self,
        path: &str,
        bytes: &[u8],
        journal: &mut dyn StorageProbeJournal,
    ) -> Result<Metadata> {
        // Keep the previous known revision on disk until the new one is acknowledged.
        // If a response is lost, cleanup refuses the unrecorded revision.
        let recovery = journal.recovery().clone();
        journal.checkpoint(recovery).map_err(|_| Error::Local)?;
        let metadata = self.upload(path, Bytes::copy_from_slice(bytes)).await?;
        checkpoint(journal, Some(metadata.rev.clone()), bytes)?;
        Ok(metadata)
    }
    pub(crate) fn probe(
        &self,
        setup_id: &str,
        journal: &mut dyn StorageProbeJournal,
    ) -> Result<()> {
        let path = probe_path(setup_id)?;
        self.runtime.block_on(async {
            self.check_account().await?;
            if self.metadata(&path).await?.is_some() {
                return Err(Error::Cleanup);
            }
            checkpoint(journal, None, b"")?;
            // What publication needs: a write, a verified read-back, and a
            // listing taken right after the write that returns it. Storage
            // clear needs the revision-checked delete.
            let result: Result<()> = async {
                let written = self.probe_write(&path, b"mallard-probe-a", journal).await?;
                let Some((_, read, hash)) = self.download(&path, 1024).await? else {
                    return Err(Error::Integrity);
                };
                if read.rev != written.rev || hash != sha256_hex(b"mallard-probe-a") {
                    return Err(Error::Integrity);
                }
                let parent = path.rsplit_once('/').ok_or(Error::Invalid)?.0;
                let page = self
                    .rpc(
                        "files/list_folder",
                        json!({"path":parent,"recursive":false,"limit":1000}),
                    )
                    .await?;
                let entries = page["entries"].as_array().ok_or(Error::Invalid)?;
                if !entries
                    .iter()
                    .any(|e| e["path_lower"] == path && e["rev"] == written.rev)
                {
                    return Err(Error::Invalid);
                }
                // A claim is an add-mode upload Dropbox must refuse when the
                // path already holds other bytes: a folder that overwrites
                // cannot arbitrate one, and cannot be set up.
                if !matches!(
                    self.upload_record(&path, Bytes::from_static(b"mallard-probe-b"))
                        .await,
                    Err(Error::Conflict)
                ) {
                    return Err(Error::Invalid);
                }
                if !matches!(
                    self.delete_path(&path, "0123456789abcdef").await?,
                    DeleteOutcome::Conflict { .. }
                ) {
                    return Err(Error::Invalid);
                }
                Ok(())
            }
            .await;
            // Confirm cleanup even when the capability test fails. Never remove folders.
            if self.cleanup(&path, journal).await.is_err() {
                return Err(Error::Cleanup);
            }
            result
        })
    }
    pub(crate) fn reconcile_probe(
        &self,
        setup_id: &str,
        journal: &mut dyn StorageProbeJournal,
    ) -> Result<()> {
        let path = probe_path(setup_id)?;
        self.runtime
            .block_on(self.cleanup(&path, journal))
            .map_err(|_| Error::Cleanup)
    }
}
