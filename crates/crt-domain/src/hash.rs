use std::fmt;

/// Identity of a piece of source text, used as the cache key for a
/// function's reading. Two functions with the same bytes share a reading.
///
/// This is FNV-1a (64-bit): dependency-free and stable across builds. It is
/// an identity for caching, not a security measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(u64);

impl ContentHash {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub fn of(bytes: &[u8]) -> Self {
        let mut h = Self::OFFSET;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(Self::PRIME);
        }
        Self(h)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
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
    fn matches_the_fnv1a_reference_vector() {
        // Known FNV-1a 64 value for the empty input and for "a".
        assert_eq!(ContentHash::of(b"").as_u64(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(ContentHash::of(b"a").as_u64(), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(ContentHash::of(b"a").to_string(), "af63dc4c8601ec8c");
    }
}
