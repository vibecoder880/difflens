//! Streaming SHA-256 verification for downloaded artifacts.
//!
//! Every byte the download manager writes is verified against the pinned
//! manifest checksum before it is atomically moved into place; verification
//! reads the file back in chunks so multi-GB GGUFs never load into memory.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{CoreError, CoreResult};

/// Chunk size for streaming reads (64 KiB — cache friendly, no heap risk).
const CHUNK: usize = 64 * 1024;

/// `true` when `s` is a lowercase-or-mixed 64-char hex string.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Normalize a hex checksum for comparison (lowercase).
pub fn normalize_sha256(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// SHA-256 of an in-memory byte slice, lowercase hex.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// SHA-256 of a file, streamed in chunks. Errors when `path` is not a
/// regular readable file or the size exceeds `max_bytes` (0 = no cap).
pub fn sha256_file(path: &Path, max_bytes: u64) -> CoreResult<String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| CoreError::Download(format!("cannot stat {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(CoreError::Download(format!(
            "not a regular file: {}",
            path.display()
        )));
    }
    if max_bytes > 0 && meta.len() > max_bytes {
        return Err(CoreError::Download(format!(
            "file {} too large for verification: {} > {max_bytes} bytes",
            path.display(),
            meta.len()
        )));
    }
    let mut file = BufReader::with_capacity(CHUNK, File::open(path)?);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Verify a file's SHA-256 against the expected hex digest. Comparison is
/// case-insensitive; both sides normalized to lowercase.
pub fn verify_file(path: &Path, expected_sha256: &str, max_bytes: u64) -> CoreResult<()> {
    if !is_sha256_hex(expected_sha256) {
        return Err(CoreError::Download(format!(
            "manifest checksum is not a sha256 hex digest: {expected_sha256:?}"
        )));
    }
    let actual = sha256_file(path, max_bytes)?;
    if normalize_sha256(&actual) != normalize_sha256(expected_sha256) {
        return Err(CoreError::Download(format!(
            "checksum mismatch for {}: expected {expected_sha256}, got {actual}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sha256_of_known_vector_matches() {
        // echo -n "abc" | sha256sum
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_file_streams_largeish_input() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("blob.bin");
        let mut f = File::create(&p).unwrap();
        let block = vec![0xABu8; CHUNK];
        // 3 chunks + a partial one — exercises the chunk loop.
        for i in 0..4 {
            let n = if i == 3 { 137 } else { block.len() };
            f.write_all(&block[..n]).unwrap();
        }
        f.flush().unwrap();
        let expected = {
            let data = std::fs::read(&p).unwrap();
            sha256_bytes(&data)
        };
        assert_eq!(sha256_file(&p, 0).unwrap(), expected);
    }

    #[test]
    fn verify_rejects_mismatch_and_bad_hex() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.txt");
        std::fs::write(&p, b"hello").unwrap();
        let good = sha256_bytes(b"hello");
        verify_file(&p, &good, 0).unwrap();
        assert!(verify_file(&p, &good.to_uppercase(), 0).is_ok());
        assert!(verify_file(&p, &sha256_bytes(b"world"), 0).is_err());
        assert!(verify_file(&p, "not-hex", 0).is_err());
        assert!(verify_file(&p, "abcd", 0).is_err());
    }

    #[test]
    fn size_cap_enforced_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.bin");
        std::fs::write(&p, vec![0u8; 1024]).unwrap();
        let err = sha256_file(&p, 100).unwrap_err();
        assert!(err.to_string().contains("too large"));
    }
}
