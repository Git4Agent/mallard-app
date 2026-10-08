//! The storage layout marker at `.mallard/_storage.json`.
//!
//! Both protocols verify the marker before touching a storage; whichever
//! writer arrives first creates it. The marker's bytes are constant, so
//! concurrent first writers write identical bytes and creation is idempotent
//! without any exclusivity.

use serde::{Deserialize, Serialize};

use super::{
    parse_json_bytes, ObjectKey, ObjectStore, MAX_STORAGE_MARKER_BYTES, STORAGE_MARKER_KEY,
};

pub(crate) const STORAGE_MARKER_FORMAT: &str = "mallard-append-storage";
pub(crate) const STORAGE_LAYOUT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageMarker {
    format: String,
    layout_version: u32,
}

impl StorageMarker {
    pub fn layout_version(&self) -> u32 {
        self.layout_version
    }

    pub fn current() -> Self {
        Self {
            format: STORAGE_MARKER_FORMAT.to_string(),
            layout_version: STORAGE_LAYOUT_VERSION,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != STORAGE_MARKER_FORMAT {
            return Err(format!(
                "storage marker format '{}' is not supported; expected '{}'",
                self.format, STORAGE_MARKER_FORMAT
            ));
        }
        if self.layout_version > STORAGE_LAYOUT_VERSION {
            return Err(format!(
                "This storage was written by a newer Mallard (layout version {}; this Mallard reads version {}). Update Mallard to use it.",
                self.layout_version, STORAGE_LAYOUT_VERSION
            ));
        }
        if self.layout_version != STORAGE_LAYOUT_VERSION {
            return Err(format!(
                "This storage uses an older Mallard layout (version {}; this Mallard reads version {}), which this version cannot read.",
                self.layout_version, STORAGE_LAYOUT_VERSION
            ));
        }
        Ok(())
    }
}

/// Verify the storage marker, creating it when the storage is new. Safe under
/// concurrent first writers: every one of them writes the same constant
/// bytes, and the read-back re-validates whatever actually landed.
pub fn ensure_storage_marker<S: ObjectStore + ?Sized>(store: &S) -> Result<(), String> {
    let key = ObjectKey::parse(STORAGE_MARKER_KEY)?;
    if let Some(stored) = store.get_metadata(&key, MAX_STORAGE_MARKER_BYTES)? {
        let marker: StorageMarker = parse_json_bytes(&stored.bytes, "Mallard storage marker")?;
        marker.validate()?;
        return Ok(());
    }

    let bytes = serde_json::to_vec(&StorageMarker::current())
        .map_err(|error| format!("serialize Mallard storage marker: {}", error))?;
    store.put_immutable(&key, &bytes)?;
    let stored = store
        .get_metadata(&key, MAX_STORAGE_MARKER_BYTES)?
        .ok_or_else(|| "Mallard storage marker disappeared after creation".to_string())?;
    let marker: StorageMarker = parse_json_bytes(&stored.bytes, "Mallard storage marker")?;
    marker.validate()
}

#[cfg(test)]
mod tests {
    use super::super::{LocalObjectStore, LOCAL_STORAGE_LOCK, MALLARD_ROOT};
    use super::*;
    use std::fs;

    #[test]
    fn mallard_storage_marker_is_created_idempotently_and_validated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("store");
        let store = LocalObjectStore::open(&root).unwrap();
        ensure_storage_marker(&store).unwrap();

        let marker_path = root.join(STORAGE_MARKER_KEY);
        let marker: StorageMarker =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        assert_eq!(marker, StorageMarker::current());
        assert!(root.join(MALLARD_ROOT).join(LOCAL_STORAGE_LOCK).is_file());

        // A second writer finds the marker already present and validates it.
        ensure_storage_marker(&store).unwrap();

        fs::write(
            &marker_path,
            br#"{"format":"mallard-append-storage","layout_version":2}"#,
        )
        .unwrap();
        let error = ensure_storage_marker(&store).unwrap_err();
        assert!(error.contains("layout version 2"));
    }

    #[test]
    fn two_first_writers_converge_on_identical_marker_bytes() {
        use super::super::plain_store::PlainStore;
        let storage = PlainStore::default();
        let (first, second) = (storage.clone(), storage.clone());
        ensure_storage_marker(&first).unwrap();
        let key = ObjectKey::parse(STORAGE_MARKER_KEY).unwrap();
        let written = first.get_metadata(&key, 4096).unwrap().unwrap().bytes;
        // The second writer did not see the first one's marker: on a store
        // with plain overwrite it writes again, and nothing changes.
        storage.remove(&key);
        ensure_storage_marker(&second).unwrap();
        assert_eq!(
            second.get_metadata(&key, 4096).unwrap().unwrap().bytes,
            written
        );
        assert_eq!(
            written,
            br#"{"format":"mallard-append-storage","layout_version":1}"#
        );
        assert_eq!(
            classify_storage_marker(&written),
            (MarkerState::Storage, Some(MarkerDetail::layout(1)))
        );
        // The CLI and the setup screens print the English; the code and the
        // version travel beside it for the interface to word.
        let detail = MarkerDetail::layout(1);
        assert_eq!(detail.text, "layout 1");
        assert_eq!(
            serde_json::to_value(&detail).unwrap(),
            serde_json::json!({"code": "layout", "layout_version": 1, "text": "layout 1"})
        );
        assert!(detail.is_valid(64));
        assert!(!MarkerDetail::reason(MarkerDetailCode::Layout, "layout ?").is_valid(64));
        assert!(!MarkerDetail {
            layout_version: Some(1),
            ..MarkerDetail::reason(MarkerDetailCode::TimedOut, "timed out")
        }
        .is_valid(64));
        assert_eq!(
            classify_storage_marker(br#"{"format":"mallard-append-storage","layout_version":2}"#).0,
            MarkerState::Unsupported
        );
        assert_eq!(
            classify_storage_marker(br#"{"format":"something-else","layout_version":1}"#).0,
            MarkerState::Invalid
        );
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkerState {
    Storage,
    Missing,
    Unsupported,
    Invalid,
    Unreadable,
}

/// What a marker read's detail is, for the interface to word: the layout
/// the marker declares, or the fixed reason it could not be read.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkerDetailCode {
    Layout,
    TimedOut,
    ReadFailed,
    AccessDenied,
    CouldNotRead,
    ScanTimedOut,
}

/// A marker read's detail: the provider's own English, which the CLI and
/// the setup screens print, beside the code the interface translates it by.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MarkerDetail {
    pub code: MarkerDetailCode,
    /// The version a `layout` detail names; no other detail carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_version: Option<u32>,
    pub text: String,
}

impl MarkerDetail {
    pub fn layout(version: u32) -> Self {
        Self {
            code: MarkerDetailCode::Layout,
            layout_version: Some(version),
            text: format!("layout {version}"),
        }
    }

    /// A fixed reason, in the provider's own words.
    pub fn reason(code: MarkerDetailCode, text: &str) -> Self {
        Self {
            code,
            layout_version: None,
            text: text.to_string(),
        }
    }

    /// A version beside a layout and nowhere else, and one short line.
    pub fn is_valid(&self, max_chars: usize) -> bool {
        (self.code == MarkerDetailCode::Layout) == self.layout_version.is_some()
            && !self.text.is_empty()
            && self.text.chars().count() <= max_chars
            && !self.text.chars().any(char::is_control)
    }
}

pub fn classify_storage_marker(bytes: &[u8]) -> (MarkerState, Option<MarkerDetail>) {
    let Ok(marker) = serde_json::from_slice::<StorageMarker>(bytes) else {
        return (MarkerState::Invalid, None);
    };
    let layout = Some(MarkerDetail::layout(marker.layout_version()));
    if marker.validate().is_ok() {
        return (MarkerState::Storage, layout);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null);
    if value["format"] == STORAGE_MARKER_FORMAT {
        (MarkerState::Unsupported, layout)
    } else {
        (MarkerState::Invalid, None)
    }
}
