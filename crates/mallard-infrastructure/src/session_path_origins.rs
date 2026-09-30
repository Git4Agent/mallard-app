//! Exact inverse path edits for captured local records. No transcript copies.

use std::collections::{BTreeMap, HashMap};

use mallard_domain::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Why an encoding stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeRefusal {
    pub message: String,
    /// A record that has a saved origin changed, or is missing from its
    /// source. The saved origins are in order, and so is every record the
    /// encoding read before it stopped.
    pub saved_record_changed: bool,
}

impl EncodeRefusal {
    fn saved_record_changed(message: &str) -> Self {
        Self {
            message: message.to_string(),
            saved_record_changed: true,
        }
    }

    /// The same refusal, said of the record it was found at.
    pub fn at(self, place: impl std::fmt::Display) -> Self {
        Self {
            message: format!("{place}: {}", self.message),
            ..self
        }
    }
}

impl From<String> for EncodeRefusal {
    fn from(message: String) -> Self {
        Self {
            message,
            saved_record_changed: false,
        }
    }
}

impl From<&str> for EncodeRefusal {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl From<EncodeRefusal> for String {
    fn from(refusal: EncodeRefusal) -> Self {
        refusal.message
    }
}

impl std::fmt::Display for EncodeRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Where the saved origin of a record is looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginLookup {
    /// At the position of the record in its source. A record that is not
    /// the one saved there refuses the encoding, and so does a source with
    /// fewer records than were saved.
    ByPosition,
    /// Among the origins saved for the source, by the digest of the record.
    /// A record that finds none is scanned as a new record, and the saved
    /// length is not asked for. This is for a transcript that was rewritten
    /// after its origins were saved. The caller answers for what the
    /// rewrite did to the conversation.
    ByDigest,
}

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
    /// The origins of the record as its provider is known to rewrite it:
    /// the digest of that form, and the edits that lie in it. A Claude
    /// record that holds thinking beside other content has them, of the
    /// record without its thinking. They are found by digest only, after
    /// the record was rewritten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewritten: Option<Box<RecordPathOrigins>>,
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
            rewritten: None,
        })
    }

    pub fn verify_record(&self, local: &Value) -> Result<(), EncodeRefusal> {
        if record_digest(local)? != self.local_sha256 {
            return Err(EncodeRefusal::saved_record_changed(
                "retained transcript record changed; path origins no longer match",
            ));
        }
        Ok(())
    }

    pub fn replay(&self, local: &Value) -> Result<Value, EncodeRefusal> {
        self.verify_record(local)?;
        if self.derived {
            return Err("derived display record must not be path encoded".into());
        }
        Ok(replay_path_fields(local, &self.fields)?)
    }
}

/// The origins saved for one physical source, given to its records in file
/// order.
pub struct SavedOrigins<'a> {
    records: &'a [RecordPathOrigins],
    /// By position, the position of the next record. By digest, the
    /// position after the last origin given.
    next: usize,
    /// The positions of the saved origins under their digests, ascending.
    by_digest: Option<HashMap<&'a str, Vec<usize>>>,
}

impl<'a> SavedOrigins<'a> {
    /// The saved origin of the next record of the source. By digest it is
    /// the first one of the record's digest that lies after the last one
    /// given: records that kept their order find their own origins, and a
    /// record that was moved ahead of others finds none. A record that its
    /// provider rewrote finds the origins saved of that form.
    pub fn origin_of(&mut self, record: &Value) -> Result<Option<&'a RecordPathOrigins>, String> {
        let Some(by_digest) = &self.by_digest else {
            let origin = self.records.get(self.next);
            self.next += 1;
            return Ok(origin);
        };
        let digest = record_digest(record)?;
        let Some(position) = by_digest.get(digest.as_str()).and_then(|positions| {
            positions
                .get(positions.partition_point(|position| *position < self.next))
                .copied()
        }) else {
            return Ok(None);
        };
        self.next = position + 1;
        Ok(self.records.get(position).and_then(|origin| {
            if origin.local_sha256 == digest {
                Some(origin)
            } else {
                origin.rewritten.as_deref()
            }
        }))
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
    ) -> Result<(), EncodeRefusal> {
        if !prefix
            && self
                .sources
                .get(source)
                .is_some_and(|records| records.len() > count)
        {
            return Err(EncodeRefusal::saved_record_changed(
                "retained transcript was truncated; path origins no longer match",
            ));
        }
        Ok(())
    }

    pub fn record(&self, source: &str, ordinal: usize) -> Option<&RecordPathOrigins> {
        self.sources
            .get(source)
            .and_then(|records| records.get(ordinal))
    }

    /// The origins saved for `source`, to be given to its records in file
    /// order.
    pub fn saved(&self, source: &str, lookup: OriginLookup) -> SavedOrigins<'_> {
        let records = self.sources.get(source).map_or(&[][..], Vec::as_slice);
        let by_digest = (lookup == OriginLookup::ByDigest).then(|| {
            let mut positions: HashMap<&str, Vec<usize>> = HashMap::new();
            for (position, record) in records.iter().enumerate() {
                let rewritten = record.rewritten.as_deref();
                for digest in std::iter::once(record)
                    .chain(rewritten)
                    .map(|origin| origin.local_sha256.as_str())
                {
                    positions.entry(digest).or_default().push(position);
                }
            }
            positions
        });
        SavedOrigins {
            records,
            next: 0,
            by_digest,
        }
    }

    /// Whether one physical source still holds the records its origins were
    /// saved from: every record with an origin has its digest, and none is
    /// missing. A Push makes this check record by record while it encodes.
    /// A reader that needs the verdict alone makes it here, and parses no
    /// record past the last saved origin. `records_of` names the origins of
    /// the records; `length_of` names the source whose saved length the
    /// records must still reach.
    pub fn accepted_records_hold(
        &self,
        records_of: &str,
        length_of: &str,
        complete_records: &[u8],
    ) -> bool {
        let mut ordinal = 0;
        for raw in complete_records.split(|byte| *byte == b'\n') {
            if raw.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if let Some(origin) = self.record(records_of, ordinal) {
                let unchanged = serde_json::from_slice::<Value>(raw)
                    .is_ok_and(|record| origin.verify_record(&record).is_ok());
                if !unchanged {
                    return false;
                }
            }
            ordinal += 1;
        }
        self.verify_source_length(length_of, ordinal, false).is_ok()
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

    /// The verdict a reader takes without encoding: growth and formatting
    /// keep it, an edit or a missing record loses it.
    #[test]
    fn accepted_records_hold_until_one_is_edited_or_missing() {
        let lines = [r#"{"id":1,"text":"first"}"#, r#"{"id":2,"text":"second"}"#];
        let mut origins = PathOrigins::default();
        for line in lines {
            let record: Value = serde_json::from_str(line).unwrap();
            origins.push(
                "source",
                RecordPathOrigins::capture(&record, Vec::new()).unwrap(),
            );
        }
        let holds = |text: &str| origins.accepted_records_hold("source", "source", text.as_bytes());
        let accepted = format!("{}\n{}\n", lines[0], lines[1]);

        assert!(holds(&accepted));
        assert!(holds(&format!("{accepted}{{\"id\":3,\"text\":\"new\"}}\n")));
        assert!(
            holds(&format!("{accepted}{{\"id\":3,\"text\":\"ne")),
            "an unfinished record is not read"
        );
        assert!(
            holds(&format!("\n{}\r\n  \n{}\n", lines[0], lines[1])),
            "blank lines carry no record"
        );
        assert!(
            holds(&accepted.replace(r#""id":1,"#, r#""id": 1, "#)),
            "the digest is of the record, not of its spacing"
        );

        assert!(!holds(&accepted.replace("first", "fir5t")), "edited");
        assert!(
            !holds(&format!(
                "{}{{\"id\":3,\"text\":\"new\"}}\n",
                accepted.replace("second", "sec0nd")
            )),
            "edited, with a new record behind it"
        );
        assert!(!holds(&format!("{}\n", lines[0])), "a record is missing");
        assert!(
            !holds(&format!("{}\n{}\n", lines[1], lines[0])),
            "out of order"
        );
        assert!(!holds(&format!("{}\nnot json\n", lines[0])));

        // Records of a source without origins were never accepted.
        assert!(origins.accepted_records_hold("other", "other", b"{}\n"));
        // The length is asked of the source the caller names.
        assert!(!origins.accepted_records_hold("other", "source", b"{}\n"));
    }

    /// The positions of the saved origins that the records of a file are
    /// given, in file order.
    fn given(origins: &PathOrigins, lookup: OriginLookup, file: &[&str]) -> Vec<Option<usize>> {
        let saved = &origins.sources["source"];
        let mut cursor = origins.saved("source", lookup);
        file.iter()
            .map(|text| {
                let origin = cursor.origin_of(&json!({"text": text})).unwrap();
                origin.map(|origin| {
                    saved
                        .iter()
                        .position(|candidate| std::ptr::eq(candidate, origin))
                        .unwrap()
                })
            })
            .collect()
    }

    #[test]
    fn a_record_finds_by_digest_the_origin_saved_of_its_content() {
        let mut origins = PathOrigins::default();
        for text in ["a", "b", "a", "c", "d"] {
            origins.push(
                "source",
                RecordPathOrigins::capture(&json!({"text": text}), Vec::new()).unwrap(),
            );
        }
        let by_digest = |file: &[&str]| given(&origins, OriginLookup::ByDigest, file);

        // An unchanged file is read as it is read by position.
        let unchanged = ["a", "b", "a", "c", "d", "new"];
        assert_eq!(
            by_digest(&unchanged),
            [Some(0), Some(1), Some(2), Some(3), Some(4), None]
        );
        assert_eq!(
            given(&origins, OriginLookup::ByPosition, &unchanged),
            by_digest(&unchanged)
        );
        // A record that was rewritten finds none. The records behind one
        // that was removed find their own.
        assert_eq!(
            by_digest(&["a", "b rewritten", "a", "d"]),
            [Some(0), None, Some(2), Some(4)]
        );
        // Records that repeat take the origins of their content in order,
        // and no origin is given twice.
        assert_eq!(by_digest(&["a", "a", "a"]), [Some(0), Some(2), None]);
        assert_eq!(by_digest(&["b", "a", "a"]), [Some(1), Some(2), None]);
        // A record that was moved ahead of others keeps its origin; the
        // ones it was moved ahead of are read as new.
        assert_eq!(
            by_digest(&["c", "a", "b", "d"]),
            [Some(3), None, None, Some(4)]
        );
        // By position the content is not asked for.
        assert_eq!(
            given(&origins, OriginLookup::ByPosition, &["d", "c"]),
            [Some(0), Some(1)]
        );
        // A source without saved origins has none to give.
        let mut other = origins.saved("other", OriginLookup::ByDigest);
        assert!(other.origin_of(&json!({"text": "a"})).unwrap().is_none());
    }

    #[test]
    fn a_rewritten_record_finds_by_digest_the_origins_saved_of_that_form() {
        let capture =
            |text: &str| RecordPathOrigins::capture(&json!({"text": text}), Vec::new()).unwrap();
        let mut origins = PathOrigins::default();
        for (text, rewritten) in [("a", None), ("b", Some("b rewritten")), ("c", None)] {
            let mut origin = capture(text);
            origin.rewritten = rewritten.map(|text| Box::new(capture(text)));
            origins.push("source", origin);
        }
        let saved = &origins.sources["source"];
        let given = |lookup, file: &[&str]| {
            let mut cursor = origins.saved("source", lookup);
            file.iter()
                .map(|text| cursor.origin_of(&json!({"text": text})).unwrap())
                .collect::<Vec<_>>()
        };
        let rewritten = saved[1].rewritten.as_deref();
        assert_eq!(
            given(OriginLookup::ByDigest, &["a", "b rewritten", "c"]),
            [Some(&saved[0]), rewritten, Some(&saved[2])]
        );
        // The record as it was saved finds its own origins, and the two
        // forms are one place: the second of them finds nothing.
        assert_eq!(
            given(OriginLookup::ByDigest, &["b", "b rewritten", "c"]),
            [Some(&saved[1]), None, Some(&saved[2])]
        );
        assert_eq!(
            given(OriginLookup::ByDigest, &["b rewritten", "b"]),
            [rewritten, None]
        );
        // By position the origins of the record itself are given, and they
        // refuse the rewritten record.
        let by_position = given(OriginLookup::ByPosition, &["a", "b rewritten"]);
        assert_eq!(by_position, [Some(&saved[0]), Some(&saved[1])]);
        assert!(
            by_position[1]
                .unwrap()
                .verify_record(&json!({"text": "b rewritten"}))
                .unwrap_err()
                .saved_record_changed
        );
        // A file without them is read as before.
        let bytes = serde_json::to_string(&saved[0]).unwrap();
        assert!(!bytes.contains("rewritten"));
        assert_eq!(
            serde_json::from_str::<RecordPathOrigins>(&bytes).unwrap(),
            saved[0]
        );
    }

    #[test]
    fn a_changed_or_missing_saved_record_is_told_apart_from_other_refusals() {
        let record = json!({"text": "saved"});
        let origin = RecordPathOrigins::capture(&record, Vec::new()).unwrap();
        origin.verify_record(&record).unwrap();
        let changed = origin
            .verify_record(&json!({"text": "edited"}))
            .unwrap_err();
        assert!(changed.saved_record_changed);
        assert_eq!(
            changed.clone().at("record 2").message,
            "record 2: retained transcript record changed; path origins no longer match"
        );
        assert!(changed.at("record 2").saved_record_changed);

        let mut origins = PathOrigins::default();
        origins.push("source", origin.clone());
        origins.push("source", origin.clone());
        let missing = origins
            .verify_source_length("source", 1, false)
            .unwrap_err();
        assert!(missing.saved_record_changed);

        let mut display = origin;
        display.derived = true;
        assert!(!display.replay(&record).unwrap_err().saved_record_changed);
        assert!(!EncodeRefusal::from("malformed").saved_record_changed);
        assert_eq!(String::from(EncodeRefusal::from("malformed")), "malformed");
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
