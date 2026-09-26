//! The setup probe: the signed-in account, a write, a verified read-back,
//! a listing taken right after the write that returns it, and the refusal
//! of a second add-mode upload. The probe file carries a fresh generated
//! name and is removed best effort; nothing about it is recorded locally.
use super::*;
use mallard_domain::generated_named_id;

const PROBE_BYTES: &[u8] = b"mallard-probe-a";

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

    pub(crate) fn probe(&self) -> Result<()> {
        let setup_id = generated_named_id("setup").map_err(|_| Error::Local)?;
        let path = format!("/.mallard/v1/_setup_probes/{setup_id}.bin");
        self.runtime.block_on(async {
            self.check_account().await?;
            let written = self.upload(&path, Bytes::from_static(PROBE_BYTES)).await?;
            let result = self.check_written(&path, &written).await;
            // Best effort, whatever the checks found: a refused delete leaves
            // the probe file behind and changes no verdict. Never remove
            // folders.
            let _ = self.delete_path(&path, &written.rev).await;
            result
        })
    }

    /// What publication needs from a written file: a verified read-back, a
    /// listing taken right after the write that returns it, and the refusal
    /// of a claim on the occupied path.
    async fn check_written(&self, path: &str, written: &Metadata) -> Result<()> {
        let Some((_, read, hash)) = self.download(path, 1024).await? else {
            return Err(Error::Integrity);
        };
        if read.rev != written.rev || hash != sha256_hex(PROBE_BYTES) {
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
        // A claim is an add-mode upload Dropbox must refuse when the path
        // already holds other bytes: a folder that overwrites cannot
        // arbitrate one, and cannot be set up.
        if !matches!(
            self.upload_record(path, Bytes::from_static(b"mallard-probe-b"))
                .await,
            Err(Error::Conflict)
        ) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
