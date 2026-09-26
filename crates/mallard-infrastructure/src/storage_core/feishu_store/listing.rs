//! Fresh batch discovery. Each physical folder is visited once; only the
//! requested branches are followed. This path neither reads nor fills the
//! path cache.

use super::*;
use crate::storage_core::ObjectQuery;
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

enum Query {
    Prefix(String),
    Key(Location),
}

fn below(path: &str, parent: &str) -> bool {
    parent.is_empty() || path == parent || path.starts_with(&format!("{parent}/"))
}

impl Query {
    fn visits(&self, logical: &str, physical: &str) -> bool {
        match self {
            Self::Prefix(prefix) => below(prefix, logical) || below(logical, prefix),
            Self::Key(location) => below(&location.dirs.join("/"), physical),
        }
    }
}

impl FeishuObjectStore {
    pub(super) fn list_batch(
        &self,
        queries: &[ObjectQuery],
        max_objects: usize,
    ) -> Vec<std::result::Result<Vec<ListedObject>, String>> {
        let result = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(120),
                self.discover_batch(queries, max_objects),
            )
            .await
            .map_err(|_| Error::Limit)?
        });
        match result {
            Ok(results) => results
                .into_iter()
                .map(|result| result.map_err(|error| self.describe(error)))
                .collect(),
            Err(error) => vec![Err(self.describe(error)); queries.len()],
        }
    }

    async fn discover_batch(
        &self,
        requests: &[ObjectQuery],
        max_objects: usize,
    ) -> Result<Vec<Result<Vec<ListedObject>>>> {
        let queries = requests
            .iter()
            .map(|request| match request {
                ObjectQuery::Prefix(prefix) => {
                    if !prefix.as_str().split('/').all(valid_name) {
                        return Err(Error::Invalid);
                    }
                    Ok(Query::Prefix(prefix.as_str().to_owned()))
                }
                ObjectQuery::Key(key) => Location::of(key).map(Query::Key),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut files = vec![BTreeMap::<ObjectKey, String>::new(); queries.len()];
        let mut errors = vec![None; queries.len()];
        // Logical path, physical path, token, transparent shard (if any).
        let mut queue = VecDeque::from([(
            String::new(),
            String::new(),
            self.root.clone(),
            None::<String>,
        )]);
        let mut visited = BTreeSet::new();
        while let Some((logical, physical, token, shard)) = queue.pop_front() {
            let active = queries
                .iter()
                .enumerate()
                .filter(|(index, query)| {
                    errors[*index].is_none() && query.visits(&logical, &physical)
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if active.is_empty() || !visited.insert(token.clone()) {
                continue;
            }
            if visited.len() > MAX_LIST_FOLDERS {
                return Err(Error::Limit);
            }
            let entries = match self.list_folder(&token).await {
                Ok(entries) => entries,
                Err(error) => {
                    for index in active {
                        errors[index] = Some(error);
                    }
                    continue;
                }
            };
            let family = family_of(&logical);
            for entry in entries {
                if !valid_name(&entry.name) {
                    continue;
                }
                let child = |parent: &str| {
                    if parent.is_empty() {
                        entry.name.clone()
                    } else {
                        format!("{parent}/{}", entry.name)
                    }
                };
                if entry.kind == "folder" {
                    let transparent = shard.is_none()
                        && family.is_some_and(|family| is_shard_name(family, &entry.name));
                    let next_logical = if transparent {
                        logical.clone()
                    } else {
                        child(&logical)
                    };
                    let next_physical = child(&physical);
                    if active
                        .iter()
                        .any(|index| queries[*index].visits(&next_logical, &next_physical))
                    {
                        queue.push_back((
                            next_logical,
                            next_physical,
                            entry.token,
                            transparent.then_some(entry.name),
                        ));
                    }
                    continue;
                }
                if entry.kind != "file" {
                    continue;
                }
                let placed = match (family, &shard) {
                    (Some(family), Some(shard)) => {
                        shard_of(family, &entry.name).as_deref() == Some(shard.as_str())
                    }
                    (Some(_), None) => false,
                    (None, _) => true,
                };
                if !placed {
                    continue;
                }
                let Ok(key) = ObjectKey::parse(child(&logical)) else {
                    continue;
                };
                for &index in &active {
                    let matches = match &queries[index] {
                        Query::Prefix(prefix) => below(&logical, prefix),
                        Query::Key(location) => {
                            location.dirs.join("/") == physical && location.name == entry.name
                        }
                    };
                    if !matches || errors[index].is_some() {
                        continue;
                    }
                    let current = files[index]
                        .entry(key.clone())
                        .or_insert_with(|| entry.token.clone());
                    if entry.token < *current {
                        *current = entry.token.clone();
                    }
                    let limit = match queries[index] {
                        Query::Prefix(_) => max_objects.min(MAX_LIST_ENTRIES),
                        Query::Key(_) => 1,
                    };
                    if files[index].len() > limit {
                        errors[index] = Some(Error::Limit);
                        files[index].clear();
                    }
                }
            }
        }
        Ok(files
            .into_iter()
            .zip(errors)
            .map(|(files, error)| {
                if let Some(error) = error {
                    return Err(error);
                }
                Ok(files
                    .into_iter()
                    .map(|(key, token)| ListedObject {
                        key,
                        feishu: Some(super::super::object_store::FeishuReadAddress {
                            root: self.root.clone(),
                            api: self.api.clone(),
                            token,
                        }),
                    })
                    .collect())
            })
            .collect())
    }
}
