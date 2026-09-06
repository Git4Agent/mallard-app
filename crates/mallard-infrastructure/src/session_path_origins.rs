//! Exact inverse path edits for captured local records. No transcript copies.

use std::collections::BTreeMap;

use mallard_domain::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSpan {
    pub start: usize,
    pub length: usize,
    /// A closed path-reference or literal-escape token, never source text.
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathField {
    pub pointer: String,
    pub key: bool,
    pub spans: Vec<PathSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordPathOrigins {
    /// Rebuildable native display/layout record. Verified, then omitted on Push.
    pub derived: bool,
    pub local_sha256: String,
    pub fields: Vec<PathField>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathOrigins {
    /// Physical source IDs and non-empty record ordinals. A native fork can
    /// consume prefixes from several sources without guessing record alignment.
    pub sources: BTreeMap<String, Vec<RecordPathOrigins>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathOriginsOwner {
    pub host_id: mallard_domain::HostId,
    pub replica_id: mallard_domain::ReplicaId,
    pub local_project_id: mallard_domain::LocalProjectId,
    pub provider_profile_id: mallard_domain::LocalProviderProfileId,
    pub provider_thread_id: String,
}

impl PathOriginsOwner {
    pub fn for_binding(binding: &mallard_domain::LocalConversationBinding) -> Result<Self, String> {
        Ok(Self {
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            local_project_id: binding.local_project_id.clone(),
            provider_profile_id: binding.provider_profile_id.clone(),
            provider_thread_id: binding
                .provider_thread_id
                .clone()
                .ok_or("path origins require a physical thread")?,
        })
    }

    pub fn for_project(
        binding: &mallard_domain::ProjectBinding,
        profile: &mallard_domain::LocalProviderProfileId,
        provider_thread_id: &str,
    ) -> Self {
        Self {
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            local_project_id: binding.local_project_id.clone(),
            provider_profile_id: profile.clone(),
            provider_thread_id: provider_thread_id.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathOriginsFile {
    pub schema: u32,
    pub codec: String,
    pub owner: PathOriginsOwner,
    pub origins: PathOrigins,
}

pub(crate) fn record_digest(record: &Value) -> Result<String, String> {
    #[cfg(test)]
    RECORD_HASH_ATTEMPTS.with(|count| count.set(count.get() + 1));
    serde_json::to_vec(record)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|_| "cannot hash path-origin record".to_string())
}

#[cfg(test)]
thread_local! {
    static RECORD_HASH_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn record_hash_attempts() -> u64 {
    RECORD_HASH_ATTEMPTS.with(std::cell::Cell::get)
}

pub(crate) fn pointer_child(parent: &str, key: &str) -> String {
    format!("{}/{}", parent, key.replace('~', "~0").replace('/', "~1"))
}

impl RecordPathOrigins {
    pub(crate) fn capture(record: &Value, fields: Vec<PathField>) -> Result<Self, String> {
        Ok(Self {
            derived: false,
            local_sha256: record_digest(record)?,
            fields,
        })
    }

    pub fn verify_record(&self, local: &Value) -> Result<(), String> {
        if record_digest(local)? != self.local_sha256 {
            return Err("retained transcript record changed; path origins no longer match".into());
        }
        Ok(())
    }

    pub fn replay(&self, local: &Value) -> Result<Value, String> {
        self.verify_record(local)?;
        if self.derived {
            return Err("derived display record must not be path encoded".into());
        }
        replay_path_fields(local, &self.fields)
    }
}

/// Apply inverse edits to a record. The caller must have just constructed
/// these edits from this exact record, or verified its retained origin hash.
/// Range, token, pointer, and key-collision checks still apply in both cases.
pub(crate) fn replay_path_fields(local: &Value, fields: &[PathField]) -> Result<Value, String> {
    let mut output = local.clone();
    for field in fields.iter().filter(|field| !field.key) {
        let value = output
            .pointer_mut(&field.pointer)
            .ok_or("path-origin field is missing")?;
        let text = value.as_str().ok_or("path-origin field is not text")?;
        *value = Value::String(replay_text(text, &field.spans)?);
    }
    let mut groups: BTreeMap<&str, Vec<&PathField>> = BTreeMap::new();
    for field in fields.iter().filter(|field| field.key) {
        let (parent, _) = field
            .pointer
            .rsplit_once('/')
            .ok_or("path-origin key has no parent")?;
        groups.entry(parent).or_default().push(field);
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|(parent, _)| std::cmp::Reverse(parent.matches('/').count()));
    for (parent, fields) in groups {
        let object = output
            .pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .ok_or("path-origin key parent is missing")?;
        let mut renamed = Vec::new();
        for field in fields {
            let (_, encoded_key) = field
                .pointer
                .rsplit_once('/')
                .ok_or("invalid key pointer")?;
            let key = encoded_key.replace("~1", "/").replace("~0", "~");
            let mapped = replay_text(&key, &field.spans)?;
            let value = object.remove(&key).ok_or("path-origin key is missing")?;
            renamed.push((mapped, value));
        }
        // Remove all renamed keys first. A valid rename can use a key
        // whose previous occupant is also being renamed.
        for (key, value) in renamed {
            if object.insert(key, value).is_some() {
                return Err("path origins collide two object keys".into());
            }
        }
    }
    Ok(output)
}

fn replay_text(text: &str, spans: &[PathSpan]) -> Result<String, String> {
    let mut output = String::with_capacity(text.len());
    let mut end = 0;
    for span in spans {
        if !crate::session_path_map::is_wire_atom(&span.token) {
            return Err("path origin contains an undeclared token".into());
        }
        let next = span
            .start
            .checked_add(span.length)
            .ok_or("path-origin range overflow")?;
        if span.start < end || text.get(span.start..next).is_none() {
            return Err("path-origin range overlaps or splits UTF-8 text".into());
        }
        output.push_str(
            text.get(end..span.start)
                .ok_or("invalid path-origin boundary")?,
        );
        output.push_str(&span.token);
        end = next;
    }
    output.push_str(text.get(end..).ok_or("invalid path-origin suffix")?);
    Ok(output)
}

impl PathOrigins {
    pub fn verify_source_length(
        &self,
        source: &str,
        count: usize,
        prefix: bool,
    ) -> Result<(), String> {
        if !prefix
            && self
                .sources
                .get(source)
                .is_some_and(|records| records.len() > count)
        {
            return Err("retained transcript was truncated; path origins no longer match".into());
        }
        Ok(())
    }

    pub fn record(&self, source: &str, ordinal: usize) -> Option<&RecordPathOrigins> {
        self.sources
            .get(source)
            .and_then(|records| records.get(ordinal))
    }

    pub fn push(&mut self, source: &str, record: RecordPathOrigins) {
        self.sources
            .entry(source.to_string())
            .or_default()
            .push(record);
    }

    pub fn merge(&mut self, other: &Self) -> Result<(), String> {
        for (source, incoming) in &other.sources {
            let existing = self.sources.entry(source.clone()).or_default();
            if existing
                .iter()
                .zip(incoming)
                .any(|(left, right)| left != right)
            {
                return Err("inherited path origins disagree for the same source records".into());
            }
            if incoming.len() > existing.len() {
                *existing = incoming.clone();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_path_map::SessionPathMapper;
    use serde_json::json;

    #[test]
    fn origin_coverage_rejects_truncation_conflicting_inheritance_and_changed_displays() {
        let mapper = SessionPathMapper::new("/tmp/project", None, None).unwrap();
        let record = json!({"text":"literal /tmp/target/x"});
        let encoded = mapper.encode_record_with_origins(&record, None).unwrap();
        let mut origins = PathOrigins::default();
        origins.push("source", encoded.origins.clone());
        origins.push("source", encoded.origins.clone());
        assert!(origins.verify_source_length("source", 1, false).is_err());
        origins.verify_source_length("source", 1, true).unwrap();
        origins.verify_source_length("source", 3, false).unwrap();
        let mut inherited = PathOrigins::default();
        inherited.push("source", encoded.origins.clone());
        inherited.merge(&origins).unwrap();
        assert_eq!(inherited, origins);
        inherited.sources.get_mut("source").unwrap()[0].local_sha256 = "f".repeat(64);
        assert!(origins.merge(&inherited).is_err());
        let mut display = encoded.origins;
        display.derived = true;
        display.verify_record(&record).unwrap();
        assert!(display.replay(&record).is_err());
        assert!(display.verify_record(&json!({"text":"updated"})).is_err());
    }

    #[test]
    #[ignore = "manual path-origin cost sample; debug timings are not production benchmarks"]
    fn path_origin_cost_sample() {
        use std::time::Instant;
        let mapper = SessionPathMapper::new("/tmp/project", None, None).unwrap();
        let count = 10_000;
        let records: Vec<_> = (0..count)
            .map(|index| {
                json!({
                    "ordinal":index, "payload":{"text":format!("{} {}", "x".repeat(1000),
                        if index % 4 == 0 { "/tmp/project/file" } else { "ordinary text" })}
                })
            })
            .collect();
        let input_bytes: usize = records
            .iter()
            .map(|record| serde_json::to_vec(record).unwrap().len())
            .sum();
        let started = Instant::now();
        let encoded: Vec<_> = records
            .iter()
            .map(|record| mapper.encode_record_with_origins(record, None).unwrap())
            .collect();
        let first_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        for (record, encoded) in records.iter().zip(&encoded) {
            let replayed = mapper
                .encode_record_with_origins(record, Some(&encoded.origins))
                .unwrap();
            assert_eq!(replayed.wire, encoded.wire);
        }
        let replay_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        for encoded in &encoded {
            mapper.decode_record_with_origins(&encoded.wire).unwrap();
        }
        let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        let origins = PathOrigins {
            sources: BTreeMap::from([(
                "source".into(),
                encoded.into_iter().map(|record| record.origins).collect(),
            )]),
        };
        let bytes = serde_json::to_vec(&origins).unwrap();
        eprintln!("path-origin sample: records={count}, input_bytes={input_bytes}, origins_bytes={}, first_encode_ms={first_ms:.3}, retained_encode_ms={replay_ms:.3}, decode_ms={decode_ms:.3}", bytes.len());
    }
}
