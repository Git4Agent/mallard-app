//! Bounded JSON objects: the stable envelope for provider-specific data.
//!
//! Provider settings, portable resource attributes, provenance fields, and
//! tombstone identities never use an unrestricted `serde_json::Value` at an
//! execution boundary. A bounded object is validated from raw JSON with a
//! custom visitor — duplicate keys are rejected instead of silently keeping
//! the last one, and depth is a design bound rather than a crash guard — and
//! then held, persisted, and digested in one canonical form: sorted keys and
//! no insignificant whitespace.
//!
//! Bounded data is inert. It can never define a command, a target path, or
//! executable behavior; an adapter or codec decodes it into a typed structure
//! for the exact registered key and version, rejecting unknown fields.

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Number;
use std::collections::BTreeMap;
use std::fmt;

/// Canonical encoded size limit.
pub const MAX_BOUNDED_OBJECT_BYTES: usize = 64 * 1024;
/// Nesting depth limit, counting the root object as depth 1.
pub const MAX_BOUNDED_OBJECT_DEPTH: usize = 8;
/// Total object fields plus array elements, root fields included.
pub const MAX_BOUNDED_OBJECT_NODES: usize = 1_024;
pub const MAX_BOUNDED_OBJECT_KEY_BYTES: usize = 256;
pub const MAX_BOUNDED_OBJECT_STRING_BYTES: usize = 4_096;
pub const MAX_BOUNDED_OBJECT_ARRAY_LENGTH: usize = 256;

/// One validated JSON value inside a bounded object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundedValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<BoundedValue>),
    Object(BTreeMap<String, BoundedValue>),
}

impl BoundedValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(number) => number.as_u64(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, BoundedValue>> {
        match self {
            Self::Object(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[BoundedValue]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The equivalent `serde_json::Value`, for typed decoding.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(value) => serde_json::Value::Bool(*value),
            Self::Number(number) => serde_json::Value::Number(number.clone()),
            Self::String(value) => serde_json::Value::String(value.clone()),
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Object(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_json()))
                    .collect(),
            ),
        }
    }
}

impl Serialize for BoundedValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(number) => number.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(items) => items.serialize(serializer),
            Self::Object(fields) => fields.serialize(serializer),
        }
    }
}

/// Which additional string rules apply while validating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedProfile {
    Config,
    Portable,
}

#[derive(Default)]
struct Budget {
    nodes: usize,
}

impl Budget {
    fn take_node(&mut self) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_BOUNDED_OBJECT_NODES {
            return Err(format!(
                "bounded object exceeds {} fields and elements",
                MAX_BOUNDED_OBJECT_NODES
            ));
        }
        Ok(())
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("bounded object key is empty".to_string());
    }
    if key.len() > MAX_BOUNDED_OBJECT_KEY_BYTES {
        return Err(format!(
            "bounded object key exceeds {} bytes",
            MAX_BOUNDED_OBJECT_KEY_BYTES
        ));
    }
    if key.chars().any(char::is_control) {
        return Err("bounded object key contains a control character".to_string());
    }
    Ok(())
}

fn validate_string(value: &str, profile: BoundedProfile) -> Result<(), String> {
    if value.len() > MAX_BOUNDED_OBJECT_STRING_BYTES {
        return Err(format!(
            "bounded object string exceeds {} bytes",
            MAX_BOUNDED_OBJECT_STRING_BYTES
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("bounded object string contains a control character".to_string());
    }
    if profile == BoundedProfile::Portable {
        if let Some(reason) = portable_string_violation(value) {
            return Err(format!("portable object string {}", reason));
        }
    }
    Ok(())
}

/// Machine-local absolute paths must not enter portable data. This is a
/// structural check for leading path separators, drive letters, and
/// home-directory prefixes; it is not a proof that a string is safe.
pub fn portable_string_violation(value: &str) -> Option<&'static str> {
    let bytes = value.as_bytes();
    if bytes
        .first()
        .is_some_and(|byte| *byte == b'/' || *byte == b'\\')
    {
        return Some("starts with a path separator");
    }
    if bytes.first() == Some(&b'~') {
        return Some("starts with a home-directory prefix");
    }
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
    {
        return Some("starts with a drive letter");
    }
    let lowered = value.get(..16).unwrap_or(value).to_ascii_lowercase();
    if lowered.starts_with("$home")
        || lowered.starts_with("${home}")
        || lowered.starts_with("%userprofile%")
        || lowered.starts_with("%home%")
    {
        return Some("starts with a home-directory variable");
    }
    None
}

struct ValueSeed<'b> {
    budget: &'b mut Budget,
    depth: usize,
    profile: BoundedProfile,
}

impl<'de> DeserializeSeed<'de> for ValueSeed<'_> {
    type Value = BoundedValue;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ValueSeed<'_> {
    type Value = BoundedValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(BoundedValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(BoundedValue::Number(Number::from(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(BoundedValue::Number(Number::from(value)))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(BoundedValue::Number)
            .ok_or_else(|| E::custom("bounded object number is not finite"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        validate_string(value, self.profile).map_err(E::custom)?;
        Ok(BoundedValue::String(value.to_string()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        validate_string(&value, self.profile).map_err(E::custom)?;
        Ok(BoundedValue::String(value))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(BoundedValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(BoundedValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        if self.depth > MAX_BOUNDED_OBJECT_DEPTH {
            return Err(de::Error::custom(format!(
                "bounded object exceeds nesting depth {}",
                MAX_BOUNDED_OBJECT_DEPTH
            )));
        }
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(ValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
            profile: self.profile,
        })? {
            if items.len() >= MAX_BOUNDED_OBJECT_ARRAY_LENGTH {
                return Err(de::Error::custom(format!(
                    "bounded object array exceeds {} elements",
                    MAX_BOUNDED_OBJECT_ARRAY_LENGTH
                )));
            }
            self.budget.take_node().map_err(de::Error::custom)?;
            items.push(item);
        }
        Ok(BoundedValue::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        if self.depth > MAX_BOUNDED_OBJECT_DEPTH {
            return Err(de::Error::custom(format!(
                "bounded object exceeds nesting depth {}",
                MAX_BOUNDED_OBJECT_DEPTH
            )));
        }
        let mut fields = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            validate_key(&key).map_err(de::Error::custom)?;
            self.budget.take_node().map_err(de::Error::custom)?;
            let value = map.next_value_seed(ValueSeed {
                budget: self.budget,
                depth: self.depth + 1,
                profile: self.profile,
            })?;
            if fields.insert(key, value).is_some() {
                return Err(de::Error::custom("bounded object has a duplicate key"));
            }
        }
        Ok(BoundedValue::Object(fields))
    }
}

fn deserialize_root<'de, D: Deserializer<'de>>(
    deserializer: D,
    profile: BoundedProfile,
) -> Result<BTreeMap<String, BoundedValue>, D::Error> {
    let mut budget = Budget::default();
    let value = deserializer.deserialize_map(ValueSeed {
        budget: &mut budget,
        depth: 1,
        profile,
    })?;
    let fields = match value {
        BoundedValue::Object(fields) => fields,
        _ => return Err(de::Error::custom("bounded object must be a JSON object")),
    };
    let encoded = serde_json::to_vec(&fields).map_err(de::Error::custom)?;
    if encoded.len() > MAX_BOUNDED_OBJECT_BYTES {
        return Err(de::Error::custom(format!(
            "bounded object exceeds {} encoded bytes",
            MAX_BOUNDED_OBJECT_BYTES
        )));
    }
    Ok(fields)
}

macro_rules! bounded_object {
    ($(#[$meta:meta])* $name:ident, $profile:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name(BTreeMap<String, BoundedValue>);

        impl $name {
            pub fn empty() -> Self {
                Self(BTreeMap::new())
            }

            /// Validate raw JSON text into the canonical form.
            pub fn from_json_str(text: &str) -> Result<Self, String> {
                serde_json::from_str(text).map_err(|error| error.to_string())
            }

            /// Validate a programmatically built value. The value is
            /// re-encoded so it passes through the same bounded visitor as
            /// raw text.
            pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
                let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
                Self::from_json_str(&text)
            }

            /// Encode a typed structure into a bounded object.
            pub fn encode<T: Serialize>(value: &T) -> Result<Self, String> {
                let json = serde_json::to_value(value).map_err(|error| error.to_string())?;
                Self::from_json(&json)
            }

            /// Decode into the exact typed structure a registered adapter or
            /// codec owns. `T` should reject unknown fields.
            pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
                serde_json::from_value(self.to_json()).map_err(|error| error.to_string())
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            pub fn len(&self) -> usize {
                self.0.len()
            }

            pub fn get(&self, key: &str) -> Option<&BoundedValue> {
                self.0.get(key)
            }

            pub fn get_str(&self, key: &str) -> Option<&str> {
                self.0.get(key).and_then(BoundedValue::as_str)
            }

            pub fn iter(&self) -> impl Iterator<Item = (&String, &BoundedValue)> {
                self.0.iter()
            }

            pub fn to_json(&self) -> serde_json::Value {
                BoundedValue::Object(self.0.clone()).to_json()
            }

            /// The canonical encoding: sorted keys, no insignificant
            /// whitespace. This is the form that is persisted and digested.
            pub fn canonical_json(&self) -> Vec<u8> {
                serde_json::to_vec(&self.0).expect("bounded objects always serialize")
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                deserialize_root(deserializer, $profile).map(Self)
            }
        }
    };
}

bounded_object!(
    /// Machine-local provider configuration: safe settings that the frontend
    /// may read and edit. Secrets never live here.
    BoundedConfigObject,
    BoundedProfile::Config
);

bounded_object!(
    /// Portable provider or contract data inside cloud records. It carries
    /// the config limits plus the portable rules: no machine-local absolute
    /// paths, no command definitions, and no field that grants executable
    /// behavior.
    BoundedPortableObject,
    BoundedProfile::Portable
);

impl From<BoundedPortableObject> for BoundedConfigObject {
    fn from(value: BoundedPortableObject) -> Self {
        Self(value.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(text: &str) -> Result<BoundedConfigObject, String> {
        BoundedConfigObject::from_json_str(text)
    }

    fn nested_object(depth: usize) -> String {
        let mut text = String::new();
        for _ in 0..depth {
            text.push_str("{\"k\":");
        }
        text.push('1');
        for _ in 0..depth {
            text.push('}');
        }
        text
    }

    #[test]
    fn canonical_form_sorts_keys_and_drops_whitespace() {
        let object =
            config("{ \"b\" : 1 , \"a\" : { \"z\" : [ 1 , 2 ] , \"y\" : \"v\" } }").unwrap();
        assert_eq!(
            String::from_utf8(object.canonical_json()).unwrap(),
            "{\"a\":{\"y\":\"v\",\"z\":[1,2]},\"b\":1}"
        );
        let reparsed = config(std::str::from_utf8(&object.canonical_json()).unwrap()).unwrap();
        assert_eq!(reparsed, object);
        assert_eq!(reparsed.canonical_json(), object.canonical_json());
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        assert!(config("{\"a\":1,\"a\":2}")
            .unwrap_err()
            .contains("duplicate key"));
        assert!(config("{\"o\":{\"a\":1,\"a\":2}}")
            .unwrap_err()
            .contains("duplicate key"));
    }

    #[test]
    fn root_must_be_an_object() {
        assert!(config("[1]").is_err());
        assert!(config("\"x\"").is_err());
        assert!(config("null").is_err());
        assert_eq!(config("{}").unwrap(), BoundedConfigObject::empty());
    }

    #[test]
    fn depth_limit_counts_the_root_as_one() {
        assert!(config(&nested_object(MAX_BOUNDED_OBJECT_DEPTH)).is_ok());
        assert!(config(&nested_object(MAX_BOUNDED_OBJECT_DEPTH + 1))
            .unwrap_err()
            .contains("nesting depth"));
        // Arrays count toward depth too.
        let mut arrays = String::from("{\"k\":");
        for _ in 0..MAX_BOUNDED_OBJECT_DEPTH - 1 {
            arrays.push('[');
        }
        arrays.push('1');
        for _ in 0..MAX_BOUNDED_OBJECT_DEPTH - 1 {
            arrays.push(']');
        }
        arrays.push('}');
        assert!(config(&arrays).is_ok());
        let deeper = arrays.replacen("1", "[1]", 1);
        assert!(config(&deeper).unwrap_err().contains("nesting depth"));
    }

    #[test]
    fn node_limit_counts_fields_and_elements() {
        let fields = (0..MAX_BOUNDED_OBJECT_NODES)
            .map(|index| format!("\"k{index}\":0"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(config(&format!("{{{fields}}}")).is_ok());
        assert!(config(&format!("{{{fields},\"extra\":0}}"))
            .unwrap_err()
            .contains("fields and elements"));
        // 1 root field + 4 arrays of 256 = 1025 nodes.
        let array = format!("[{}]", vec!["0"; 256].join(","));
        let text = format!("{{\"a\":[{array},{array},{array},{array}]}}");
        assert!(config(&text).is_err());
        let text = format!("{{\"a\":[{array},{array},{array}]}}");
        assert!(config(&text).is_ok());
    }

    #[test]
    fn key_and_string_length_limits() {
        let key = "k".repeat(MAX_BOUNDED_OBJECT_KEY_BYTES);
        assert!(config(&format!("{{\"{key}\":1}}")).is_ok());
        assert!(config(&format!("{{\"{key}x\":1}}"))
            .unwrap_err()
            .contains("key exceeds"));
        assert!(config("{\"\":1}").unwrap_err().contains("key is empty"));
        let value = "v".repeat(MAX_BOUNDED_OBJECT_STRING_BYTES);
        assert!(config(&format!("{{\"k\":\"{value}\"}}")).is_ok());
        assert!(config(&format!("{{\"k\":\"{value}x\"}}"))
            .unwrap_err()
            .contains("string exceeds"));
    }

    #[test]
    fn array_length_limit() {
        let items = vec!["1"; MAX_BOUNDED_OBJECT_ARRAY_LENGTH].join(",");
        assert!(config(&format!("{{\"k\":[{items}]}}")).is_ok());
        assert!(config(&format!("{{\"k\":[{items},1]}}"))
            .unwrap_err()
            .contains("array exceeds"));
    }

    #[test]
    fn control_characters_are_rejected_in_keys_and_strings() {
        assert!(config("{\"a\\u0007\":1}").is_err());
        assert!(config("{\"a\":\"x\\u0000y\"}").is_err());
        assert!(config("{\"a\":\"tab\\tinside\"}").is_err());
        assert!(config("{\"a\":\"plain text\"}").is_ok());
    }

    #[test]
    fn numbers_must_be_finite() {
        assert!(config("{\"n\":1.5}").is_ok());
        assert!(config("{\"n\":-3}").is_ok());
        assert!(config("{\"n\":1e400}").is_err());
        assert!(BoundedConfigObject::from_json(&json!({"n": f64::MAX})).is_ok());
    }

    #[test]
    fn encoded_size_limit_is_checked_on_the_canonical_form() {
        let string = "s".repeat(MAX_BOUNDED_OBJECT_STRING_BYTES);
        // 16 strings of 4096 bytes plus keys exceed 64 KiB.
        let fields = (0..16)
            .map(|index| format!("\"k{index:02}\":\"{string}\""))
            .collect::<Vec<_>>()
            .join(",");
        assert!(config(&format!("{{{fields}}}"))
            .unwrap_err()
            .contains("encoded bytes"));
        let fields = (0..15)
            .map(|index| format!("\"k{index:02}\":\"{string}\""))
            .collect::<Vec<_>>()
            .join(",");
        assert!(config(&format!("{{{fields}}}")).is_ok());
    }

    #[test]
    fn portable_objects_reject_machine_local_paths() {
        for value in [
            "/Users/someone/project",
            "\\\\server\\share",
            "C:\\Users",
            "c:/Users",
            "~/project",
            "~",
            "$HOME/x",
            "${HOME}/x",
            "%USERPROFILE%\\x",
        ] {
            let text = serde_json::to_string(&json!({"path": value})).unwrap();
            assert!(
                BoundedPortableObject::from_json_str(&text).is_err(),
                "{value:?} must be rejected"
            );
            assert!(BoundedConfigObject::from_json_str(&text).is_ok());
        }
        for value in ["relative/path", "title: /help", "C", "c:", "home"] {
            let text = serde_json::to_string(&json!({"v": value})).unwrap();
            assert!(
                BoundedPortableObject::from_json_str(&text).is_ok(),
                "{value:?}"
            );
        }
        // Nested strings are checked too.
        let text = serde_json::to_string(&json!({"o": {"list": ["/abs"]}})).unwrap();
        assert!(BoundedPortableObject::from_json_str(&text).is_err());
    }

    #[test]
    fn typed_round_trip_rejects_unknown_fields() {
        #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
        #[serde(deny_unknown_fields)]
        struct Settings {
            directory: String,
        }
        let object = BoundedConfigObject::encode(&Settings {
            directory: "/tmp/x".to_string(),
        })
        .unwrap();
        assert_eq!(
            object.decode::<Settings>().unwrap(),
            Settings {
                directory: "/tmp/x".to_string()
            }
        );
        let extra = config("{\"directory\":\"/tmp/x\",\"extra\":1}").unwrap();
        assert!(extra.decode::<Settings>().is_err());
    }

    #[test]
    fn nested_document_round_trips_byte_identically() {
        #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Document {
            settings: BoundedConfigObject,
            revision: u64,
        }
        let document = Document {
            settings: config("{\"z\":1,\"a\":{\"c\":true,\"b\":null}}").unwrap(),
            revision: 3,
        };
        let pretty = serde_json::to_string_pretty(&document).unwrap();
        let loaded: Document = serde_json::from_str(&pretty).unwrap();
        assert_eq!(loaded, document);
        assert_eq!(serde_json::to_string_pretty(&loaded).unwrap(), pretty);
        assert!(pretty.find("\"a\"").unwrap() < pretty.find("\"z\"").unwrap());
    }
}
