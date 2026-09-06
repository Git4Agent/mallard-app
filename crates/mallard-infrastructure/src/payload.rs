//! Immutable in-memory payload bytes with their content digest.
//!
//! The private fields keep the SHA-256 bound to the exact allocation that
//! moves from capture through publication. Cloning this value clones only a
//! `Bytes` handle and the bounded digest string; it never copies the payload.

use bytes::Bytes;
use mallard_domain::hex_lower;
use sha2::{Digest, Sha256};
use std::ops::Deref;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadBytes {
    bytes: Bytes,
    sha256: String,
    sha256_digest: [u8; 32],
}

impl PayloadBytes {
    pub fn from_bytes(bytes: Bytes) -> Self {
        let sha256_digest: [u8; 32] = Sha256::digest(&bytes).into();
        let sha256 = hex_lower(&sha256_digest);
        Self {
            bytes,
            sha256,
            sha256_digest,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The same content digest in binary form, for framed artifact hashes.
    pub(crate) fn sha256_digest(&self) -> &[u8; 32] {
        &self.sha256_digest
    }

    pub fn into_bytes(self) -> Bytes {
        self.bytes
    }
}

impl From<Vec<u8>> for PayloadBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from_bytes(Bytes::from(bytes))
    }
}

impl From<Bytes> for PayloadBytes {
    fn from(bytes: Bytes) -> Self {
        Self::from_bytes(bytes)
    }
}

impl AsRef<[u8]> for PayloadBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Deref for PayloadBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::sha256_hex;

    #[test]
    fn vec_conversion_and_clones_share_the_payload_allocation() {
        let source = vec![0x5a; 64 * 1024];
        let source_pointer = source.as_ptr();
        let payload = PayloadBytes::from(source);
        assert_eq!(payload.as_bytes().as_ptr(), source_pointer);
        assert_eq!(payload.sha256(), sha256_hex(payload.as_bytes()));
        assert_eq!(hex_lower(payload.sha256_digest()), payload.sha256());

        let cloned = payload.clone();
        assert_eq!(cloned.as_bytes().as_ptr(), payload.as_bytes().as_ptr());
    }
}
