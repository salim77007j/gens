//! FNV-1a 64-bit hashing — the engine's content fingerprint.
//!
//! The engine already uses FNV-1a 64 for evidence hashes (replay pins,
//! UI frames, smoke JSONs); assets use the same primitive so content
//! identity is comparable across systems. FNV-1a is not cryptographic —
//! it detects *accidental* change (the only kind an asset pipeline must
//! handle). Deliberate tampering is out of scope; see ADR-0020.

/// FNV-1a 64-bit offset basis.
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Hashes `bytes` with FNV-1a 64.
///
/// Pure function of the input; identical bytes always yield the
/// identical hash on every platform (byte-wise integer arithmetic, no
/// random seeding — the opposite of ahash's runtime-rng).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Hashes two byte slices as one sequence (left, then right) without
/// concatenating — used for composite fingerprints.
pub fn fnv1a64_pair(a: &[u8], b: &[u8]) -> u64 {
    let mut hash = OFFSET_BASIS;
    for &byte in a.iter().chain(b.iter()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    // Published FNV-1a 64 test vectors.
    #[test]
    fn matches_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn pair_equals_concatenation() {
        let joined = fnv1a64(b"hello,world");
        assert_eq!(fnv1a64_pair(b"hello,", b"world"), joined);
    }

    #[test]
    fn single_bit_flip_changes_hash() {
        let a = fnv1a64(b"walker.gltf");
        let b = fnv1a64(b"walker.gltg");
        assert_ne!(a, b);
    }
}
