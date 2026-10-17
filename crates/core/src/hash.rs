//! SHA-256 helpers used for the run ledger.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::CoreResult;

/// SHA-256 of a byte slice, lowercase hex.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// SHA-256 of a file, streamed (never loads the whole file into memory).
pub fn sha256_file(path: &Path) -> CoreResult<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// First 12 hex chars, for file cards.
pub fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_vector() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(short(&sha256_bytes(b"abc")), "ba7816bf8f01");
    }
}
