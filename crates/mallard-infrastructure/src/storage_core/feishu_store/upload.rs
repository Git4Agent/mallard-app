//! The write paths: one `upload_all` per small object, prepare/part/finish
//! above 20 MiB, and one `upload_all` for a publication record. A present
//! name is the object; nothing is overwritten and nothing is resent.
use super::*;
use reqwest::multipart::{Form, Part};

/// Feishu's documented block size; the prepare answer is checked against
/// this ceiling and the object's size.
const MAX_BLOCKS: u64 = 1024;

impl FeishuObjectStore {
    /// Resolve the requested directories together: cached ones as known,
    /// the rest through one listing of each physical parent, taking every
    /// same-named sibling. With `create`, a missing level is created and
    /// confirmed; without it, a missing directory is left out. The path
    /// cache keeps only directories that were explicitly resolved.
    pub(super) async fn resolve_directories(
        &self,
        directories: &[Vec<String>],
        create: bool,
    ) -> Result<BTreeMap<String, BTreeSet<String>>> {
        let mut resolved = BTreeMap::new();
        let mut groups: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
        for dirs in directories {
            let path = dirs.join("/");
            if let Some(tokens) = self.cached_folders(&path) {
                resolved.insert(path, tokens);
            } else {
                let (name, parent) = dirs.split_last().ok_or(Error::Invalid)?;
                groups
                    .entry(parent.to_vec())
                    .or_default()
                    .insert(name.clone());
            }
        }
        for (parent_dirs, names) in groups {
            let parent_path = parent_dirs.join("/");
            let Some(parents) = self.resolve(&parent_dirs, create, true).await? else {
                continue;
            };
            let target = parents.iter().next().ok_or(Error::Invalid)?;
            let mut entries = Vec::new();
            for parent in &parents {
                match self.list_folder(parent).await {
                    Ok(listed) => entries.extend(listed),
                    Err(Error::Missing) => {
                        self.forget(&parent_path);
                        return Err(Error::Missing);
                    }
                    Err(error) => return Err(error),
                }
            }
            for name in names {
                let matching = |entry: &&Entry| entry.kind == "folder" && entry.name == name;
                let mut tokens: BTreeSet<String> = entries
                    .iter()
                    .filter(matching)
                    .map(|entry| entry.token.clone())
                    .collect();
                if tokens.is_empty() {
                    if !create {
                        continue;
                    }
                    let created = self.create_folder(target, &name).await?;
                    // Confirm creation and include any same-named sibling.
                    // Replace only this physical parent's batch-local entries.
                    let listed = self.list_folder(target).await?;
                    entries.retain(|entry| entry.parent_token != *target);
                    entries.extend(listed);
                    tokens = entries
                        .iter()
                        .filter(matching)
                        .map(|entry| entry.token.clone())
                        .collect();
                    if !tokens.contains(&created) {
                        return Err(Error::InvalidResponse(
                            "drive/v1/files/create_folder",
                            "created folder missing from listing",
                        ));
                    }
                }
                let path = if parent_path.is_empty() {
                    name
                } else {
                    format!("{parent_path}/{name}")
                };
                self.remember_folders(&path, &tokens);
                resolved.insert(path, tokens);
            }
        }
        Ok(resolved)
    }

    fn upload_size(size: u64) -> Result<()> {
        if size > FEISHU_UPLOAD_LIMIT {
            Err(Error::UploadLimit)
        } else {
            Ok(())
        }
    }

    /// One `upload_all` request: the whole object in one multipart form.
    pub(super) async fn upload_all(
        &self,
        parent: &str,
        name: &str,
        bytes: Bytes,
    ) -> Result<String> {
        validate_id(parent).map_err(|_| Error::Invalid)?;
        if !valid_name(name) || bytes.len() as u64 > SMALL_UPLOAD_LIMIT {
            return Err(Error::Invalid);
        }
        let url = self.url("drive/v1/files/upload_all");
        let (parent, name) = (parent.to_owned(), name.to_owned());
        let data = self
            .rpc("drive/v1/files/upload_all", move |client| {
                let form = Form::new()
                    .text("file_name", name.clone())
                    .text("parent_type", "explorer")
                    .text("parent_node", parent.clone())
                    .text("size", bytes.len().to_string())
                    .part(
                        "file",
                        Part::stream_with_length(bytes.clone(), bytes.len() as u64)
                            .file_name(name.clone()),
                    );
                client.post(&url).multipart(form)
            })
            .await?;
        file_token(&data, "drive/v1/files/upload_all")
    }

    /// Prepare, parts in order, finish. Nothing about the session is kept:
    /// a failed large upload is abandoned and the next Push starts over.
    async fn upload_multipart(&self, parent: &str, name: &str, bytes: Bytes) -> Result<String> {
        validate_id(parent).map_err(|_| Error::Invalid)?;
        if !valid_name(name) {
            return Err(Error::Invalid);
        }
        let size = bytes.len() as u64;
        let prepare_url = self.url("drive/v1/files/upload_prepare");
        let body = serde_json::json!({
            "file_name": name, "parent_type": "explorer", "parent_node": parent, "size": size
        });
        let plan = self
            .rpc("drive/v1/files/upload_prepare", move |client| {
                client.post(&prepare_url).json(&body)
            })
            .await?;
        let invalid = |field| Error::InvalidResponse("drive/v1/files/upload_prepare", field);
        let upload_id = plan["upload_id"]
            .as_str()
            .ok_or_else(|| invalid("upload_id"))?
            .to_owned();
        validate_id(&upload_id).map_err(|_| invalid("upload_id format"))?;
        let block_size = plan["block_size"]
            .as_u64()
            .ok_or_else(|| invalid("block_size"))?;
        let block_num = plan["block_num"]
            .as_u64()
            .ok_or_else(|| invalid("block_num"))?;
        if block_size == 0
            || block_size > SMALL_UPLOAD_LIMIT
            || block_num == 0
            || block_num > MAX_BLOCKS
            || block_num != size.div_ceil(block_size)
        {
            return Err(invalid("block size or count"));
        }
        let part_url = self.url("drive/v1/files/upload_part");
        for seq in 0..block_num {
            let start = (seq * block_size) as usize;
            let end = ((seq + 1) * block_size).min(size) as usize;
            let part = bytes.slice(start..end);
            let (part_url, upload_id) = (part_url.clone(), upload_id.clone());
            self.rpc_ack("drive/v1/files/upload_part", move |client| {
                let form = Form::new()
                    .text("upload_id", upload_id.clone())
                    .text("seq", seq.to_string())
                    .text("size", part.len().to_string())
                    .part(
                        "file",
                        Part::stream_with_length(part.clone(), part.len() as u64).file_name("part"),
                    );
                client.post(&part_url).multipart(form)
            })
            .await?;
        }
        let finish_url = self.url("drive/v1/files/upload_finish");
        let body = serde_json::json!({"upload_id": upload_id, "block_num": block_num});
        let data = self
            .rpc("drive/v1/files/upload_finish", move |client| {
                client.post(&finish_url).json(&body)
            })
            .await?;
        file_token(&data, "drive/v1/files/upload_finish")
    }

    async fn upload(&self, parent: &str, name: &str, bytes: Bytes) -> Result<String> {
        Self::upload_size(bytes.len() as u64)?;
        if bytes.len() as u64 <= SMALL_UPLOAD_LIMIT {
            self.upload_all(parent, name, bytes).await
        } else {
            self.upload_multipart(parent, name, bytes).await
        }
    }

    /// Store every object, one directory at a time: one listing of the
    /// directory, then one upload per absent name. A present name is the
    /// object. An upload whose answer was lost is settled by one more
    /// listing, never by a resend. `on_stored` hears each input's size once
    /// its object is settled, present or written.
    pub(super) async fn put_many_async(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
        on_stored: &mut dyn FnMut(u64),
    ) -> Result<Vec<ImmutablePutOutcome>> {
        // Validate the whole input before any I/O.
        let mut placed: Vec<(Location, Bytes)> = Vec::with_capacity(objects.len());
        let mut by_path: BTreeMap<String, usize> = BTreeMap::new();
        let mut indices = Vec::with_capacity(objects.len());
        for (key, bytes) in objects {
            let location = Location::of(&key)?;
            Self::upload_size(bytes.len() as u64)?;
            // A payload's name is its digest: a present name stands for the
            // object only because these bytes are the ones it names.
            if location.content_addressed() && mallard_domain::sha256_hex(&bytes) != location.name {
                return Err(Error::Integrity);
            }
            let path = location.path();
            let index = if let Some(&index) = by_path.get(&path) {
                if placed[index].1 != bytes {
                    return Err(Error::Integrity);
                }
                index
            } else {
                let index = placed.len();
                by_path.insert(path, index);
                placed.push((location, bytes));
                index
            };
            indices.push(index);
        }
        let mut copies = vec![0_usize; placed.len()];
        for &index in &indices {
            copies[index] += 1;
        }
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, (location, _)) in placed.iter().enumerate() {
            groups
                .entry(location.dirs.join("/"))
                .or_default()
                .push(index);
        }
        let mut outcomes: Vec<Option<ImmutablePutOutcome>> = vec![None; placed.len()];
        let directories: Vec<_> = groups
            .values()
            .map(|members| placed[members[0]].0.dirs.clone())
            .collect();
        let resolved = self.resolve_directories(&directories, true).await?;
        for members in groups.into_values() {
            let dirs = placed[members[0]].0.dirs.clone();
            let folders = resolved.get(&dirs.join("/")).ok_or(Error::Invalid)?;
            let (files, count, target) = self.files_in_resolved(&dirs, folders).await?;
            let mut present: BTreeSet<String> = files.into_keys().collect();
            let mut children = count;
            for index in members {
                let (location, bytes) = &placed[index];
                let outcome = if present.contains(&location.name) {
                    ImmutablePutOutcome::AlreadyPresent
                } else {
                    if children >= MAX_FOLDER_CHILDREN {
                        return Err(Error::Quota);
                    }
                    let outcome = match self.upload(&target, &location.name, bytes.clone()).await {
                        Ok(token) => {
                            self.remember_file(&location.path(), &token);
                            ImmutablePutOutcome::Written
                        }
                        Err(Error::Unknown) => {
                            // One listing settles it; the object's name says
                            // whether the upload landed.
                            let landed = self
                                .list_folder(&target)
                                .await?
                                .into_iter()
                                .filter(|entry| entry.kind == "file" && entry.name == location.name)
                                .map(|entry| entry.token)
                                .min();
                            match landed {
                                Some(token) => {
                                    self.remember_file(&location.path(), &token);
                                    ImmutablePutOutcome::AlreadyPresent
                                }
                                None => return Err(Error::Unknown),
                            }
                        }
                        Err(error) => return Err(error),
                    };
                    present.insert(location.name.clone());
                    children += 1;
                    outcome
                };
                outcomes[index] = Some(outcome);
                for _ in 0..copies[index] {
                    on_stored(bytes.len() as u64);
                }
            }
        }
        let mut seen = BTreeSet::new();
        indices
            .into_iter()
            .map(|index| {
                if seen.insert(index) {
                    outcomes[index].clone().ok_or(Error::Invalid)
                } else {
                    Ok(ImmutablePutOutcome::AlreadyPresent)
                }
            })
            .collect()
    }

    /// A publication record is one `upload_all` after the caller's own
    /// listing, never retried here: every attempt to write a record follows
    /// a listing, and a record's name holds the digest of its bytes.
    pub(super) async fn upload_record(&self, key: &ObjectKey, bytes: Bytes) -> Result<String> {
        let location = Location::of(key)?;
        if bytes.len() as u64 > SMALL_UPLOAD_LIMIT {
            return Err(Error::UploadLimit);
        }
        let folders = self
            .resolve(&location.dirs, true, true)
            .await?
            .ok_or(Error::Invalid)?;
        let target = folders.iter().next().ok_or(Error::Invalid)?.clone();
        let token = self.upload_all(&target, &location.name, bytes).await?;
        self.remember_file(&location.path(), &token);
        Ok(token)
    }
}

fn file_token(data: &Value, operation: &'static str) -> Result<String> {
    let invalid = Error::InvalidResponse(operation, "file_token");
    let token = data["file_token"].as_str().ok_or(invalid)?;
    validate_id(token).map_err(|_| invalid)?;
    Ok(token.to_owned())
}
