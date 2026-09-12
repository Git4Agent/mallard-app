//! The storage layout marker at `.mallard/_storage.json`.
//!
//! Both protocols verify the marker before touching a storage; whichever
//! writer arrives first creates it idempotently.

use serde::{Deserialize, Serialize};

use super::{
    parse_json_bytes, ObjectKey, ObjectStore, MAX_STORAGE_MARKER_BYTES, STORAGE_MARKER_KEY,
};

pub(crate) const STORAGE_MARKER_FORMAT: &str = "mallard-storage";
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
/// concurrent first writers: the immutable put tolerates an identical marker
/// and the read-back re-validates whatever actually landed.
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
            br#"{"format":"mallard-storage","layout_version":2}"#,
        )
        .unwrap();
        let error = ensure_storage_marker(&store).unwrap_err();
        assert!(error.contains("layout version 2"));
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


pub fn classify_storage_marker(bytes: &[u8]) -> (MarkerState, Option<String>) {
    let Ok(marker) = serde_json::from_slice::<StorageMarker>(bytes) else {
        return (MarkerState::Invalid, None);
    };
    let layout = Some(format!("layout {}", marker.layout_version()));
    if marker.validate().is_ok() {
        return (MarkerState::Storage, layout);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or(serde_json::Value::Null);
    if value["format"] == "mallard-storage" {
        (MarkerState::Unsupported, layout)
    } else {
        (MarkerState::Invalid, None)
    }
}
