//! Trait implementations bridging U256 and bitcoin types.
//!
//! These live here rather than in u256.rs to avoid coupling the generic
//! integer type to bitcoin.

use bitcoin::hashes::Hash;
use bitcoin::pow::Target;

use super::BlockHash;
use crate::u256::U256;

impl From<Target> for U256 {
    fn from(target: Target) -> Self {
        Self::from_le_bytes(target.to_le_bytes())
    }
}

impl From<U256> for Target {
    fn from(u: U256) -> Self {
        Target::from_le_bytes(u.to_le_bytes())
    }
}

impl From<&BlockHash> for U256 {
    fn from(hash: &BlockHash) -> Self {
        Self::from_le_bytes(*hash.as_byte_array())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_u256_roundtrip() {
        let target = Target::MAX;
        let u = U256::from(target);
        let back = Target::from(u);
        assert_eq!(target, back);
    }
}
