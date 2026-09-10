//! One write path for payloads, control objects, and setup probes.
use super::*;

const MAX_SESSION_ID_BYTES: usize = 16 * 1024;

#[derive(Clone)]
struct Upload {
    path: String,
    mode: Value,
    bytes: Bytes,
    /// Dropbox block hash of `bytes`, computed once at preflight on the
    /// calling thread so transfer workers never hash.
    hash: String,
}
impl Upload {
    fn new(path: String, mode: Value, bytes: Bytes) -> Result<Self> {
        canonical_path(&path)?;
        upload_size(bytes.len() as u64)?;
        let fields = mode.as_object().ok_or(Error::Invalid)?;
        match mode[".tag"].as_str() {
            Some("add") if fields.len() == 1 => {}
            Some("update")
                if fields.len() == 2 && mode["update"].as_str().is_some_and(valid_rev) => {}
            _ => return Err(Error::Invalid),
        }
        let hash = content_hash(&bytes);
        Ok(Self {
            path,
            mode,
            bytes,
            hash,
        })
    }
}

impl DropboxObjectStore {
    async fn start_upload(&self, upload: &Upload) -> Result<String> {
        let argument = json!({"close": true, "content_hash": upload.hash}).to_string();
        let response = self
            .send_checked("files/upload_session/start", Endpoint::Content, |request| {
                request
                    .header("Content-Type", "application/octet-stream")
                    .header("Dropbox-API-Arg", argument.clone())
                    .body(upload.bytes.clone())
            })
            .await?;
        let body = Self::bounded(response, RPC_LIMIT).await?;
        let result: Value = serde_json::from_slice(&body).map_err(|_| Error::Invalid)?;
        result["session_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= MAX_SESSION_ID_BYTES)
            .map(str::to_owned)
            .ok_or(Error::Invalid)
    }

    /// Prove that the immutable object at `upload.path` already holds these
    /// bytes without downloading them. Dropbox's content hash is SHA-256
    /// based, and the commit success path already trusts it.
    async fn prove_present(&self, upload: &Upload) -> Result<()> {
        let Some(metadata) = self.metadata(&upload.path).await? else {
            return Err(Error::Integrity);
        };
        metadata.check(&upload.path, false)?;
        if metadata.size != upload.bytes.len() as u64 || metadata.content_hash != upload.hash {
            return Err(Error::Integrity);
        }
        Ok(())
    }

    /// Only commit errors are returned per entry. A staging error must not be
    /// interpreted as an ambiguous file mutation. Dropbox creates missing
    /// parent folders on commit, with the canonical case supplied here.
    async fn upload_window(&self, uploads: &[Upload]) -> Result<Vec<Result<Metadata>>> {
        if uploads.is_empty() || uploads.len() > 16 {
            return Err(Error::Limit);
        }
        let sessions = self
            .concurrent(
                uploads.to_vec(),
                uploads.len(),
                |store, upload| async move { store.start_upload(&upload).await },
            )
            .await?;
        let mut ids = BTreeSet::new();
        let mut entries = Vec::with_capacity(uploads.len());
        for (upload, session) in uploads.iter().zip(&sessions) {
            if !ids.insert(session.as_str()) {
                return Err(Error::Invalid);
            }
            entries.push(json!({
                "cursor": {"session_id": session, "offset": upload.bytes.len()},
                "commit": {
                    "path": upload.path, "mode": upload.mode,
                    "autorename": false, "mute": true, "strict_conflict": true
                }
            }));
        }
        let args = serde_json::to_vec(&json!({"entries": entries})).map_err(|_| Error::Invalid)?;
        if args.len() as u64 > RPC_LIMIT {
            return Err(Error::Limit);
        }
        // Cloned handles serialize their commits here.
        let _commit = self.commit_gate.lock().await;
        let result = match self
            .rpc_bytes("files/upload_session/finish_batch_v2", args)
            .await
        {
            Ok(result) => result,
            Err(Error::Unknown) => return Ok(vec![Err(Error::Unknown); uploads.len()]),
            Err(error) => return Err(error),
        };
        let entries = result["entries"].as_array().ok_or(Error::Invalid)?;
        if entries.len() != uploads.len() {
            return Err(Error::Invalid);
        }
        Ok(entries
            .iter()
            .zip(uploads)
            .map(|(entry, upload)| match entry[".tag"].as_str() {
                Some("success") => {
                    // A success entry carries the FileMetadata fields beside
                    // its own `.tag`: Stone flattens a struct payload into
                    // the union object. Only `failure` nests, being a union.
                    let mut fields = entry.clone();
                    fields.as_object_mut().ok_or(Error::Invalid)?.remove(".tag");
                    let metadata = Metadata::from_file_value(fields)?;
                    metadata.check(&upload.path, false)?;
                    if metadata.size != upload.bytes.len() as u64
                        || metadata.content_hash != upload.hash
                    {
                        return Err(Error::Integrity);
                    }
                    Ok(metadata)
                }
                Some("failure") => Err(classify_conflict(&entry["failure"])),
                _ => Err(Error::Invalid),
            })
            .collect())
    }

    pub(super) async fn upload(&self, path: &str, mode: Value, bytes: Bytes) -> Result<Metadata> {
        let upload = Upload::new(path.to_owned(), mode, bytes)?;
        self.upload_window(&[upload])
            .await?
            .pop()
            .ok_or(Error::Invalid)?
    }

    pub(super) async fn put_many_async(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
        concurrency: usize,
    ) -> Result<Vec<ImmutablePutOutcome>> {
        if !(1..=16).contains(&concurrency) {
            return Err(Error::Limit);
        }
        // Validate the whole input, including later windows, before any I/O.
        let mut unique: Vec<Upload> = Vec::new();
        let mut by_path: BTreeMap<String, usize> = BTreeMap::new();
        let mut indices = Vec::with_capacity(objects.len());
        for (key, bytes) in objects {
            let upload = Upload::new(object_path(&key)?, json!({".tag":"add"}), bytes)?;
            let index = if let Some(&index) = by_path.get(&upload.path) {
                if unique[index].bytes != upload.bytes {
                    return Err(Error::Integrity);
                }
                index
            } else {
                let index = unique.len();
                by_path.insert(upload.path.clone(), index);
                unique.push(upload);
                index
            };
            indices.push(index);
        }
        let mut outcomes = Vec::with_capacity(unique.len());
        for window in unique.chunks(concurrency) {
            let results = self.upload_window(window).await?;
            let mut unproven = Vec::new();
            for (upload, result) in window.iter().zip(results) {
                outcomes.push(match result {
                    Ok(_) => ImmutablePutOutcome::Written,
                    // The key is taken or the commit result was lost: the
                    // bytes there must be exactly these before this counts.
                    Err(Error::Conflict | Error::Unknown) => {
                        unproven.push(upload.clone());
                        ImmutablePutOutcome::AlreadyPresent
                    }
                    Err(error) => return Err(error),
                });
            }
            self.concurrent(unproven, window.len(), |store, upload| async move {
                store.prove_present(&upload).await
            })
            .await?;
        }
        let mut seen = BTreeSet::new();
        Ok(indices
            .into_iter()
            .map(|index| {
                if seen.insert(index) {
                    outcomes[index].clone()
                } else {
                    ImmutablePutOutcome::AlreadyPresent
                }
            })
            .collect())
    }
}
