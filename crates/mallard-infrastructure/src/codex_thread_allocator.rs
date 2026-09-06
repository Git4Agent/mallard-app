//! Local Codex provider thread ID allocation during Pull Apply.
//!
//! IDs retain the existing random version-7 format and validation. Allocation
//! checks the caller's occupied IDs and every ID minted in the same batch.
//! It never starts Codex or reads a provider home.

use mallard_domain::hex_lower;

const MAX_COLLISION_RETRIES: usize = 16;

/// Validate one canonical lowercase RFC 9562 UUIDv7.
pub fn validate_uuid_v7(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let dashes = [8_usize, 13, 18, 23];
    if bytes.len() != 36 {
        return Err("Codex thread id is not a canonical UUID".to_string());
    }
    for (index, byte) in bytes.iter().enumerate() {
        if dashes.contains(&index) {
            if *byte != b'-' {
                return Err("Codex thread id is not a canonical UUID".to_string());
            }
        } else if !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte) {
            return Err("Codex thread id is not a lowercase hexadecimal UUID".to_string());
        }
    }
    if bytes[14] != b'7' {
        return Err("Codex thread id is not a version 7 UUID".to_string());
    }
    if !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
        return Err("Codex thread id has an invalid UUID variant".to_string());
    }
    Ok(())
}

/// Generate an identifier with the existing random version-7 format.
pub fn generate_random_uuid_v7() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("random Codex thread id: {}", error))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex_lower(&bytes);
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    validate_uuid_v7(&uuid)?;
    Ok(uuid)
}

/// Allocate `count` fresh Codex provider thread IDs locally. `is_taken`
/// checks occupied provider IDs; each batch also rejects its own duplicates.
pub fn allocate_codex_thread_ids_local(
    count: usize,
    is_taken: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    let mut allocated: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        let mut accepted = None;
        for _ in 0..MAX_COLLISION_RETRIES {
            let candidate = generate_random_uuid_v7()?;
            if is_taken(&candidate) || allocated.iter().any(|existing| existing == &candidate) {
                continue;
            }
            accepted = Some(candidate);
            break;
        }
        allocated.push(accepted.ok_or_else(|| {
            "Codex thread id allocation exhausted its collision retries".to_string()
        })?);
    }
    Ok(allocated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeSet;

    #[test]
    fn random_uuid_v7_values_are_canonical_and_unique() {
        let batch = allocate_codex_thread_ids_local(35, &|_| false).unwrap();
        assert_eq!(batch.len(), 35);
        assert_eq!(batch.iter().collect::<BTreeSet<_>>().len(), 35);
        for id in &batch {
            validate_uuid_v7(id).unwrap();
        }
    }

    #[test]
    fn uuid_validation_rejects_noncanonical_ids() {
        let valid = "019811aa-0000-7000-8000-000000000001";
        validate_uuid_v7(valid).unwrap();
        for invalid in [
            valid.to_uppercase(),
            valid.replacen("-7000-", "-4000-", 1),
            valid.replacen("-8000-", "-0000-", 1),
            valid.replace('-', ""),
            format!("{valid}0"),
        ] {
            assert!(validate_uuid_v7(&invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn allocation_rejects_occupied_ids_and_bounds_collision_retries() {
        let checked = RefCell::new(Vec::new());
        let batch = allocate_codex_thread_ids_local(1, &|candidate| {
            let mut checked = checked.borrow_mut();
            checked.push(candidate.to_string());
            checked.len() <= 2
        })
        .unwrap();
        let checked = checked.into_inner();
        assert_eq!(checked.len(), 3);
        assert_eq!(batch, vec![checked[2].clone()]);
        assert!(!checked[..2].contains(&batch[0]));

        let attempts = Cell::new(0);
        let error = allocate_codex_thread_ids_local(1, &|_| {
            attempts.set(attempts.get() + 1);
            true
        })
        .unwrap_err();
        assert_eq!(attempts.get(), 16);
        assert!(error.contains("collision retries"), "{error}");
    }
}
