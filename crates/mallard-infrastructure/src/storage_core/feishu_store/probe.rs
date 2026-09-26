//! The setup probe: the signed-in user, the storage folder, a write, a
//! verified read-back, and a listing that returns the write. The probe
//! file carries a fresh generated name and stays (no delete scope is
//! requested); nothing about it is recorded locally.
use super::*;
use mallard_domain::{generated_named_id, sha256_hex};

const PROBE_BYTES: &[u8] = b"mallard-probe";

impl FeishuObjectStore {
    /// The signed-in user, through the store's own gate.
    pub(crate) async fn check_account(&self) -> Result<()> {
        let url = self.url("authen/v1/user_info");
        let data = self
            .rpc("authen/v1/user_info", move |client| client.get(&url))
            .await?;
        let open_id = data["open_id"].as_str().ok_or(Error::Invalid)?;
        validate_id(open_id).map_err(|_| Error::Invalid)?;
        if data["name"]
            .as_str()
            .is_none_or(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    pub(crate) fn probe(&self) -> Result<()> {
        let setup_id = generated_named_id("setup").map_err(|_| Error::Local)?;
        let name = format!("{setup_id}.bin");
        let dirs: Vec<String> = [".mallard", "v1", "_setup_probes"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let location = Location {
            dirs: dirs.clone(),
            name: name.clone(),
        };
        self.runtime.block_on(async {
            self.authorization
                .header()
                .await
                .map(|_| ())
                .map_err(Error::from)?;
            self.check_account().await?;
            // Resolve from the root without cached folders: this listing
            // also verifies root access, so no separate root check is needed.
            let folders = self
                .resolve(&dirs, true, false)
                .await?
                .ok_or(Error::Invalid)?;
            let (_, count, target) = self.files_in_resolved(&dirs, &folders).await?;
            if count >= MAX_FOLDER_CHILDREN {
                return Err(Error::Quota);
            }
            let token = self
                .upload_all(&target, &name, Bytes::from_static(PROBE_BYTES))
                .await?;
            let mut read = Vec::new();
            let Some((size, hash)) = self
                .download_into(&location, &token, 1024, &mut read)
                .await?
            else {
                return Err(Error::Integrity);
            };
            if size != PROBE_BYTES.len() as u64
                || hash != sha256_hex(PROBE_BYTES)
                || read != PROBE_BYTES
            {
                return Err(Error::Integrity);
            }
            // What publication needs: a listing taken right after the write
            // returns it.
            if !self
                .list_folder(&target)
                .await?
                .iter()
                .any(|entry| entry.token == token && entry.name == name)
            {
                return Err(Error::Invalid);
            }
            Ok(())
        })
    }
}
