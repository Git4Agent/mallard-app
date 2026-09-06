//! Machine-local secret storage for provider configurations.
//!
//! A [`SecretMap`] is bounded, redacted from `Debug`, excluded from frontend
//! DTOs and portable data by type (it is never part of a view type), and
//! returned only to the selected adapter during an authorized backend action.
//! Values zeroize on drop.

use crate::keys::SecretKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use zeroize::Zeroizing;

pub const MAX_SECRET_ENTRIES: usize = 16;
pub const MAX_SECRET_VALUE_BYTES: usize = 64 * 1024;

/// One secret value. It has no `Display` and a redacted `Debug`.
#[derive(Clone)]
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = Zeroizing::new(value.into());
        if value.is_empty() {
            return Err("secret value is empty".to_string());
        }
        if value.len() > MAX_SECRET_VALUE_BYTES {
            return Err(format!(
                "secret value exceeds {} bytes",
                MAX_SECRET_VALUE_BYTES
            ));
        }
        if value.chars().any(char::is_control) {
            return Err("secret value contains a control character".to_string());
        }
        Ok(Self(value))
    }

    /// The secret itself. Callers hand it to an adapter or a persistence
    /// writer only; it must never reach a log, an error, or a DTO.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq for SecretValue {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl Eq for SecretValue {}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue(<redacted>)")
    }
}

impl Serialize for SecretValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for SecretValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Zeroizing::new(String::deserialize(deserializer)?);
        Self::parse(value.as_str()).map_err(serde::de::Error::custom)
    }
}

/// Bounded machine-local secrets keyed by the stable key grammar.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretMap(BTreeMap<SecretKey, SecretValue>);

impl SecretMap {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, key: &SecretKey) -> bool {
        self.0.contains_key(key)
    }

    pub fn get(&self, key: &SecretKey) -> Option<&SecretValue> {
        self.0.get(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &SecretKey> {
        self.0.keys()
    }

    pub fn insert(&mut self, key: SecretKey, value: SecretValue) -> Result<(), String> {
        if !self.0.contains_key(&key) && self.0.len() >= MAX_SECRET_ENTRIES {
            return Err(format!("secret map exceeds {} entries", MAX_SECRET_ENTRIES));
        }
        self.0.insert(key, value);
        Ok(())
    }

    pub fn remove(&mut self, key: &SecretKey) -> bool {
        self.0.remove(key).is_some()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Apply one presence operation. `Set` requires the secret to be absent
    /// and `Replace` requires it to be present, so a stale form cannot
    /// silently overwrite or resurrect a value.
    pub fn apply(&mut self, key: &SecretKey, operation: SecretOperation) -> Result<(), String> {
        match operation {
            SecretOperation::Set { value } => {
                if self.0.contains_key(key) {
                    return Err(format!("secret '{}' is already set", key));
                }
                self.insert(key.clone(), value)
            }
            SecretOperation::Replace { value } => {
                if !self.0.contains_key(key) {
                    return Err(format!("secret '{}' is not set", key));
                }
                self.insert(key.clone(), value)
            }
            SecretOperation::Clear => {
                self.0.remove(key);
                Ok(())
            }
            SecretOperation::Unchanged => Ok(()),
        }
    }
}

impl fmt::Debug for SecretMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretMap")
            .field("keys", &self.0.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Serialize for SecretMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = BTreeMap::<SecretKey, SecretValue>::deserialize(deserializer)?;
        if entries.len() > MAX_SECRET_ENTRIES {
            return Err(serde::de::Error::custom(format!(
                "secret map exceeds {} entries",
                MAX_SECRET_ENTRIES
            )));
        }
        Ok(Self(entries))
    }
}

/// A secret presence operation from the setup form. A saved value is never
/// returned to the frontend; the form reports only what should happen to it.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretOperation {
    Set { value: SecretValue },
    Replace { value: SecretValue },
    Clear,
    Unchanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> SecretKey {
        SecretKey::parse(name).unwrap()
    }

    fn value(text: &str) -> SecretValue {
        SecretValue::parse(text).unwrap()
    }

    #[test]
    fn values_are_bounded_and_redacted() {
        assert!(SecretValue::parse("").is_err());
        assert!(SecretValue::parse("a\u{7}").is_err());
        assert!(SecretValue::parse("a".repeat(MAX_SECRET_VALUE_BYTES)).is_ok());
        assert!(SecretValue::parse("a".repeat(MAX_SECRET_VALUE_BYTES + 1)).is_err());
        let secret = value("hunter2");
        assert_eq!(format!("{secret:?}"), "SecretValue(<redacted>)");
        let mut map = SecretMap::empty();
        map.insert(key("token"), secret).unwrap();
        let debug = format!("{map:?}");
        assert!(debug.contains("token"));
        assert!(!debug.contains("hunter2"));
    }

    #[test]
    fn map_is_bounded_on_insert_and_load() {
        let mut map = SecretMap::empty();
        for index in 0..MAX_SECRET_ENTRIES {
            map.insert(key(&format!("k{index}")), value("v")).unwrap();
        }
        assert!(map.insert(key("overflow"), value("v")).is_err());
        // Replacing an existing key at the limit is fine.
        map.insert(key("k0"), value("replaced")).unwrap();
        let json = serde_json::to_string(&map).unwrap();
        let loaded: SecretMap = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, map);
        let mut overflow: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        for index in 0..=MAX_SECRET_ENTRIES {
            overflow.insert(format!("k{index}"), serde_json::Value::String("v".into()));
        }
        assert!(serde_json::from_value::<SecretMap>(serde_json::Value::Object(overflow)).is_err());
        assert!(serde_json::from_str::<SecretMap>("{\"Bad Key\":\"v\"}").is_err());
        assert!(serde_json::from_str::<SecretMap>("{\"k\":\"\"}").is_err());
    }

    #[test]
    fn presence_operations_enforce_prior_state() {
        let mut map = SecretMap::empty();
        let token = key("token");
        map.apply(&token, SecretOperation::Unchanged).unwrap();
        assert!(map
            .apply(
                &token,
                SecretOperation::Replace {
                    value: value("new")
                }
            )
            .is_err());
        map.apply(
            &token,
            SecretOperation::Set {
                value: value("first"),
            },
        )
        .unwrap();
        assert!(map
            .apply(
                &token,
                SecretOperation::Set {
                    value: value("second")
                }
            )
            .is_err());
        map.apply(
            &token,
            SecretOperation::Replace {
                value: value("second"),
            },
        )
        .unwrap();
        assert_eq!(map.get(&token).unwrap().expose(), "second");
        map.apply(&token, SecretOperation::Clear).unwrap();
        assert!(!map.contains(&token));
        map.apply(&token, SecretOperation::Clear).unwrap();
    }

    #[test]
    fn operations_parse_from_tagged_json() {
        let set: SecretOperation =
            serde_json::from_str("{\"op\":\"set\",\"value\":\"x\"}").unwrap();
        assert_eq!(set, SecretOperation::Set { value: value("x") });
        let clear: SecretOperation = serde_json::from_str("{\"op\":\"clear\"}").unwrap();
        assert_eq!(clear, SecretOperation::Clear);
        assert!(serde_json::from_str::<SecretOperation>("{\"op\":\"set\"}").is_err());
        assert!(
            serde_json::from_str::<SecretOperation>("{\"op\":\"set\",\"value\":\"\"}").is_err()
        );
        assert!(serde_json::from_str::<SecretOperation>("{\"op\":\"delete\"}").is_err());
    }
}
