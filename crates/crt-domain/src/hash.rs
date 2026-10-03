use std::fmt;

use sha2::{Digest, Sha256};

/// Identity of a piece of source text, used as part of the cache key for a
/// function's reading. Two functions with the same bytes share a reading.
///
/// SHA-256, so a repository cannot craft one function that collides with
/// another and be served the other's cached explanation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Parses the 64-character hex form produced by `Display`.
    pub fn parse_hex(hex: &str) -> Option<Self> {
        if hex.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_same_hash_different_bytes_different_hash() {
        assert_eq!(ContentHash::of(b"fn a() {}"), ContentHash::of(b"fn a() {}"));
        assert_ne!(ContentHash::of(b"fn a() {}"), ContentHash::of(b"fn b() {}"));
    }

    #[test]
    fn matches_the_sha256_reference_vector_and_round_trips_hex() {
        let h = ContentHash::of(b"abc");
        let hex = h.to_string();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(ContentHash::parse_hex(&hex), Some(h));
        assert_eq!(ContentHash::parse_hex("zz"), None);
    }
}
