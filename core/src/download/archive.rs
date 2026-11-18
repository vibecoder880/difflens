//! Hardened archive extraction for pinned server bundles.
//!
//! Red-team SEC2 hardening — every extracted entry must pass ALL gates:
//!
//! 1. **Type gate**: plain regular files and directories always OK.
//!    Tar symlinks/hardlinks are allowed ONLY when both endpoints stay
//!    inside the staging tree (relative target, no `..`, basename
//!    allowlisted) — real llama.cpp bundles need them for versioned
//!    `lib*.so.1` SONAME resolution. Zip symlinks are always rejected
//!    (the windows bundle contains none). Devices/fifos are rejected.
//! 2. **Path gate**: no absolute paths, no `..` components, no empty
//!    components (zip-slip / tar-traversal).
//! 3. **Name gate**: file basenames must match the llama.cpp bundle
//!    prefix allowlist (`llama-`, `ggml-`, `lib`, `mtmd`, `LICENSE`) — a
//!    checksummed archive should never contain anything else, so anything
//!    else is treated as hostile.
//! 4. **Size gates**: per-entry and total decompressed size caps enforced
//!    *while reading*, not just from headers (zip-bomb defense).
//! 5. **Staging + atomic rename**: extraction always targets a fresh,
//!    empty staging dir and is wiped on any failure, so only a complete
//!    extraction can ever be renamed into place.
//!
//! The archives themselves are SHA256-pinned in the committed manifest,
//! so the entry gates are defense-in-depth against a compromised
//! upstream release, a mistagged manifest entry, or a buggy mirror.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::download::manifest::ArchiveFormat;
use crate::error::{CoreError, CoreResult};

/// Decompressed-size caps. The real b12503 bundles decompress to ~65 MB.
#[derive(Debug, Clone, Copy)]
pub struct ExtractLimits {
    /// Cap on the sum of all extracted file sizes.
    pub max_total_bytes: u64,
    /// Cap on a single entry's decompressed size.
    pub max_entry_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 1024 * 1024 * 1024, // 1 GiB
            max_entry_bytes: 256 * 1024 * 1024,  // 256 MiB
        }
    }
}

/// Basename prefixes allowed inside a llama.cpp server bundle. Note
/// `llama`/`ggml` without a dash are deliberate: the bundles ship a bare
/// `llama` multi-binary launcher.
const ALLOWED_PREFIXES: [&str; 5] = ["llama", "ggml", "lib", "mtmd", "LICENSE"];

/// Extraction report: where files landed inside the staging dir.
#[derive(Debug, Clone)]
pub struct Extraction {
    pub files: Vec<PathBuf>,
    pub total_bytes: u64,
}

/// Extract `archive_path` (pinned server bundle) into a **fresh, empty**
/// `staging_dir`, applying all hardening gates. Returns the list of
/// extracted files (staging-relative absolute paths).
pub fn extract_archive(
    archive_path: &Path,
    format: ArchiveFormat,
    staging_dir: &Path,
    limits: &ExtractLimits,
) -> CoreResult<Extraction> {
    let meta = std::fs::metadata(archive_path)
        .map_err(|e| CoreError::Download(format!("archive {}: {e}", archive_path.display())))?;
    if meta.len() > limits.max_total_bytes {
        // Compressed archive itself must be within the decompressed cap —
        // sanity bound before we even open it.
        return Err(CoreError::Download(format!(
            "archive {} larger than total cap",
            archive_path.display()
        )));
    }

    if staging_dir.exists() {
        let mut entries = std::fs::read_dir(staging_dir)
            .map_err(|e| CoreError::Download(format!("staging {}: {e}", staging_dir.display())))?;
        if entries.next().is_some() {
            return Err(CoreError::Download(format!(
                "staging dir {} is not empty; refusing to extract",
                staging_dir.display()
            )));
        }
    }
    std::fs::create_dir_all(staging_dir)?;

    let mut acc = ExtractAccumulator::new(limits);
    let result = match format {
        ArchiveFormat::TarGz => extract_tar_gz(archive_path, staging_dir, &mut acc),
        ArchiveFormat::Zip => extract_zip(archive_path, staging_dir, &mut acc),
    };
    // Atomicity: on ANY failure the staging dir is wiped so no partial
    // extraction can ever be renamed into place.
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(staging_dir);
        return Err(e);
    }
    Ok(Extraction {
        files: acc.files,
        total_bytes: acc.total,
    })
}

/// Pure path gate: `Ok(Some(rel))` = extractable file, `Ok(None)` =
/// skip (root/dir), `Err` = hostile path (absolute, `..`, prefix-
/// disallowed basename). Unit-tested directly and exercised end-to-end
/// by the zip-slip tests.
pub(crate) fn gate_path(raw_path: &Path) -> CoreResult<Option<PathBuf>> {
    let mut rel = PathBuf::new();
    for c in raw_path.components() {
        match c {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() || part == "." {
                    continue;
                }
                rel.push(part.as_ref());
            }
            // Absolute, `..`, drive/prefix components are hostile.
            _ => {
                return Err(CoreError::Download(format!(
                    "hostile archive path rejected: {raw_path:?}"
                )));
            }
        }
    }
    if rel.as_os_str().is_empty() {
        return Ok(None); // the archive root itself
    }
    if rel.is_dir() || raw_path.to_string_lossy().ends_with('/') {
        return Ok(None); // directories are created implicitly on demand
    }
    let base = rel
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if !ALLOWED_PREFIXES.iter().any(|p| base.starts_with(p)) {
        return Err(CoreError::Download(format!(
            "entry basename not in allowlist: {base:?}"
        )));
    }
    Ok(Some(rel))
}

struct ExtractAccumulator<'a> {
    limits: &'a ExtractLimits,
    total: u64,
    files: Vec<PathBuf>,
}

impl<'a> ExtractAccumulator<'a> {
    fn new(limits: &'a ExtractLimits) -> Self {
        Self {
            limits,
            total: 0,
            files: Vec::new(),
        }
    }

    /// Apply path + name gates via [`gate_path`].
    fn gate(&self, raw_path: &Path) -> CoreResult<Option<PathBuf>> {
        gate_path(raw_path)
    }

    /// Account `n` decompressed bytes against the caps.
    fn account(&mut self, n: u64) -> CoreResult<()> {
        self.total += n;
        if self.total > self.limits.max_total_bytes {
            return Err(CoreError::Download(format!(
                "total decompressed size exceeds cap {}",
                self.limits.max_total_bytes
            )));
        }
        Ok(())
    }

    fn record(&mut self, staging_dir: &Path, rel: &Path) {
        self.files.push(staging_dir.join(rel));
    }
}

/// Read (up to) `expected` bytes from `r`, enforcing the per-entry cap and
/// aborting when the producer cannot finish. Writes into `out` (created
/// here) with `mode_bits` applied on unix.
fn write_capped(
    r: &mut dyn Read,
    expected: u64,
    out: &Path,
    mode_bits: u32,
    acc: &mut ExtractAccumulator,
) -> CoreResult<()> {
    if expected > acc.limits.max_entry_bytes {
        return Err(CoreError::Download(format!(
            "entry {out:?} size {expected} exceeds per-entry cap"
        )));
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(out)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > acc.limits.max_entry_bytes {
            // Header lied — decompressed bomb. Abort.
            return Err(CoreError::Download(format!(
                "entry {out:?} exceeded per-entry cap while reading"
            )));
        }
        acc.account(n as u64)?;
        file.write_all(&buf[..n])?;
    }
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Strip setuid/setgid/sticky; keep rwxrwxrwx, force owner-write and
        // preserve executable bits from the archive mode.
        let safe = (mode_bits & 0o777) | 0o644;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(safe)).ok();
    }
    Ok(())
}

fn extract_tar_gz(
    archive_path: &Path,
    staging_dir: &Path,
    acc: &mut ExtractAccumulator,
) -> CoreResult<()> {
    let f = File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut archive = tar::Archive::new(gz);
    archive.set_preserve_permissions(false);

    for entry in archive
        .entries()
        .map_err(|e| CoreError::Download(format!("tar read: {e}")))?
    {
        let mut entry = entry.map_err(|e| CoreError::Download(format!("tar entry: {e}")))?;

        // Gate 1: type. Regular files + dirs always fine. Symlinks and
        // hardlinks are allowed ONLY when both endpoints stay inside the
        // staging tree (relative, no `..`, basename allowlisted) — real
        // llama.cpp bundles need them for versioned `lib*.so.1` SONAME
        // resolution. Everything else (devices, fifos, …) is hostile.
        let etype = entry.header().entry_type();

        // Gate 2: path traversal on the entry itself.
        let raw_path = entry
            .path()
            .map_err(|e| CoreError::Download(format!("tar path: {e}")))?
            .into_owned();
        let rel = match acc.gate(&raw_path)? {
            Some(rel) => rel,
            None => continue,
        };
        let out = staging_dir.join(&rel);

        match etype {
            tar::EntryType::Symlink => {
                let target = entry
                    .link_name()
                    .map_err(|e| CoreError::Download(format!("tar link target: {e}")))?
                    .unwrap_or_default();
                validate_internal_link(&target, "symlink")?;
                #[cfg(unix)]
                {
                    if let Some(parent) = out.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::os::unix::fs::symlink(&target, &out)?;
                    // Recorded so the install-time flatten keeps the link
                    // (versioned libs like libllama-common.so.1 are links).
                    acc.record(staging_dir, &rel);
                }
                #[cfg(not(unix))]
                {
                    let _ = out;
                    return Err(CoreError::Download(
                        "symlink entries unsupported on this platform".into(),
                    ));
                }
                continue;
            }
            tar::EntryType::Link => {
                let target = entry
                    .link_name()
                    .map_err(|e| CoreError::Download(format!("tar link target: {e}")))?
                    .unwrap_or_default();
                validate_internal_link(&target, "hardlink")?;
                // Hardlinks reference an already-extracted archive member
                // relative to the staging root.
                let target_abs = staging_dir.join(&target);
                if !target_abs.is_file() {
                    return Err(CoreError::Download(format!(
                        "hardlink target {} not extracted before use",
                        target.display()
                    )));
                }
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::hard_link(&target_abs, &out)?;
                acc.record(staging_dir, &rel);
                continue;
            }
            tar::EntryType::Regular | tar::EntryType::GNUSparse => {}
            tar::EntryType::Directory => continue,
            other => {
                return Err(CoreError::Download(format!(
                    "non-regular archive entry rejected: {:?} ({other:?})",
                    raw_path
                )));
            }
        }

        // Gate 4: caps (header claim + enforced while reading).
        let size = entry.header().size()?;
        let mode = entry.header().mode().unwrap_or(0o644);
        write_capped(&mut entry, size, &out, mode, acc)?;
        acc.record(staging_dir, &rel);
    }
    Ok(())
}

/// Validate a link target: must be relative, contain no `..` and pass the
/// basename allowlist, so both endpoints stay inside the staging tree.
fn validate_internal_link(target: &Path, kind: &str) -> CoreResult<()> {
    if target.is_absolute() {
        return Err(CoreError::Download(format!(
            "{kind} with absolute target rejected: {}",
            target.display()
        )));
    }
    for c in target.components() {
        match c {
            std::path::Component::Normal(part) => {
                let base = part.to_string_lossy();
                if !ALLOWED_PREFIXES.iter().any(|p| base.starts_with(p)) {
                    return Err(CoreError::Download(format!(
                        "{kind} target basename not in allowlist: {base:?}"
                    )));
                }
            }
            _ => {
                return Err(CoreError::Download(format!(
                    "{kind} escaping the bundle rejected: {}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

fn extract_zip(
    archive_path: &Path,
    staging_dir: &Path,
    acc: &mut ExtractAccumulator,
) -> CoreResult<()> {
    let f = File::open(archive_path)?;
    let mut archive =
        zip::ZipArchive::new(f).map_err(|e| CoreError::Download(format!("zip open: {e}")))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| CoreError::Download(format!("zip entry {i}: {e}")))?;

        // Gate 1: type. `is_symlink` covers unix mode bits; anything that
        // is neither file nor directory is rejected via the name gate
        // below plus the dir_file check.
        let entry_name = entry.name().map(|n| n.to_string()).unwrap_or_default();
        if entry.is_symlink() {
            return Err(CoreError::Download(format!(
                "symlink archive entry rejected: {entry_name}"
            )));
        }

        // Gate 2: path traversal (zip crate's own sanitizer, plus ours).
        let rel: PathBuf = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                return Err(CoreError::Download(format!(
                    "hostile zip path rejected: {entry_name}"
                )));
            }
        };
        let rel = match acc.gate(&rel)? {
            Some(rel) => rel,
            None => continue,
        };

        // Gate 4: caps.
        let size = entry.size();
        let mode = entry.unix_mode().unwrap_or(0o644);
        let out = staging_dir.join(&rel);
        if entry.is_file() {
            write_capped(&mut entry, size, &out, mode, acc)?;
            acc.record(staging_dir, &rel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new().prefix(name).tempdir().unwrap()
    }

    fn make_tar_gz(entries: &[(&str, tar::EntryType, Option<&[u8]>)], dir: &Path) -> PathBuf {
        let file = File::create(dir.join("bundle.tar.gz")).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        for (name, etype, data) in entries {
            let mut header = tar::Header::new_gnu();
            match etype {
                tar::EntryType::Directory => {
                    header.set_size(0);
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_mode(0o755);
                    b.append_data(&mut header, name, std::io::empty()).unwrap();
                }
                tar::EntryType::Symlink => {
                    header.set_size(0);
                    header.set_entry_type(tar::EntryType::Symlink);
                    b.append_link(&mut header, name, "/etc/passwd").unwrap();
                }
                tar::EntryType::Link => {
                    header.set_entry_type(tar::EntryType::Link);
                    b.append_link(&mut header, name, "llama-b/llama-server")
                        .unwrap();
                }
                _ => {
                    let data = data.unwrap_or_default();
                    header.set_size(data.len() as u64);
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_mode(0o755);
                    header.set_cksum();
                    b.append_data(&mut header, name, data).unwrap();
                }
            }
        }
        b.into_inner().unwrap().finish().unwrap();
        dir.join("bundle.tar.gz")
    }

    fn make_zip(entries: &[(&str, Option<&[u8]>)], dir: &Path) -> PathBuf {
        let path = dir.join("bundle.zip");
        let file = File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        for (name, data) in entries {
            match data {
                Some(bytes) => {
                    let opts: zip::write::SimpleFileOptions =
                        zip::write::SimpleFileOptions::default().unix_permissions(0o755);
                    w.start_file(*name, opts).unwrap();
                    w.write_all(bytes).unwrap();
                }
                None => {
                    w.add_directory(*name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                }
            }
        }
        w.finish().unwrap();
        path
    }

    /// Build a tar.gz from raw 512-byte ustar headers — the only way to
    /// craft entries the safe `tar::Builder` refuses to write (hostile
    /// paths). Enough of ustar for regular-file entries.
    fn raw_tar_gz(entries: &[(&str, &[u8])], dir: &Path) -> PathBuf {
        fn block() -> [u8; 512] {
            [0u8; 512]
        }
        fn put(buf: &mut [u8; 512], off: usize, s: &str) {
            buf[off..off + s.len()].copy_from_slice(s.as_bytes());
        }
        let mut out: Vec<u8> = Vec::new();
        for (name, data) in entries {
            let mut b = block();
            put(&mut b, 0, name);
            put(&mut b, 100, "0000644\0"); // mode
            put(&mut b, 108, "0000000\0"); // uid
            put(&mut b, 116, "0000000\0"); // gid
            put(&mut b, 124, &format!("{:011o}\0", data.len())); // size
            put(&mut b, 136, "00000000000\0"); // mtime
            b[156] = b'0'; // typeflag: regular
            put(&mut b, 257, "ustar\0");
            put(&mut b, 263, "00");
            // Checksum: spaces while computing, then octal.
            b[148..156].fill(b' ');
            let sum: u64 = b.iter().map(|&x| x as u64).sum();
            put(&mut b, 148, &format!("{:06o}\0 ", sum));
            out.extend_from_slice(&b);
            out.extend_from_slice(data);
            let pad = (512 - data.len() % 512) % 512;
            out.extend(std::iter::repeat_n(0u8, pad));
        }
        out.extend_from_slice(&[0u8; 1024]); // end-of-archive
        let path = dir.join("raw.tar.gz");
        let f = File::create(&path).unwrap();
        let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        enc.write_all(&out).unwrap();
        enc.finish().unwrap();
        path
    }

    #[test]
    fn gate_rejects_hostile_paths_directly() {
        assert!(gate_path(Path::new("../evil-llama-server")).is_err());
        assert!(gate_path(Path::new("/etc/llama-evil")).is_err());
        assert!(gate_path(Path::new("a/../../llama-x")).is_err());
        assert!(gate_path(Path::new("C:\\llama-evil.exe")).is_err());
        assert_eq!(
            gate_path(Path::new("llama-b1/llama-server")).unwrap(),
            Some(PathBuf::from("llama-b1/llama-server"))
        );
        assert_eq!(gate_path(Path::new("llama-b1/")).unwrap(), None);
        assert!(gate_path(Path::new("llama-b1/malware.sh")).is_err());
    }

    #[test]
    fn extracts_plain_tar_gz_bundle() {
        let src = tmp_dir("x-tar-ok");
        let entries = vec![
            ("llama-b1/", tar::EntryType::Directory, None),
            (
                "llama-b1/llama-server",
                tar::EntryType::Regular,
                Some(b"#!/bin/false\n" as &[u8]),
            ),
            (
                "llama-b1/libggml.so.1",
                tar::EntryType::Regular,
                Some(b"\0lib\0"),
            ),
            ("llama-b1/LICENSE", tar::EntryType::Regular, Some(b"MIT")),
        ];
        let archive = make_tar_gz(&entries, src.path());
        let staging = tmp_dir("x-tar-stage");
        let res = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap();
        assert_eq!(res.files.len(), 3);
        assert!(staging.path().join("llama-b1/llama-server").exists());
        assert!(staging.path().join("llama-b1/libggml.so.1").exists());
        assert!(staging.path().join("llama-b1/LICENSE").exists());

        // Re-extract into the now-non-empty staging dir must fail.
        assert!(
            extract_archive(
                &archive,
                ArchiveFormat::TarGz,
                staging.path(),
                &ExtractLimits::default()
            )
            .is_err()
        );
    }

    #[test]
    fn extracts_plain_zip_bundle() {
        let src = tmp_dir("x-zip-ok");
        let archive = make_zip(
            &[
                ("llama-b1/", None),
                ("llama-b1/llama-server.exe", Some(b"MZ fake")),
                ("llama-b1/llama-server-impl.dll", Some(b"\0dll\0")),
            ],
            src.path(),
        );
        let staging = tmp_dir("x-zip-stage");
        let res = extract_archive(
            &archive,
            ArchiveFormat::Zip,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap();
        assert_eq!(res.files.len(), 2);
        assert!(staging.path().join("llama-b1/llama-server.exe").exists());
        assert!(
            staging
                .path()
                .join("llama-b1/llama-server-impl.dll")
                .exists()
        );
    }

    // ---- Zip-slip / hostile entry tests (red-team SEC2) ----

    #[test]
    fn tar_parent_traversal_rejected() {
        let src = tmp_dir("x-tar-slip");
        // Raw header: `tar::Builder` refuses to write `..` paths itself.
        let archive = raw_tar_gz(&[("../evil-llama-server", b"pwned")], src.path());
        let staging = tmp_dir("x-tar-slip-stage");
        let err = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("hostile archive path"), "{err}");
        assert!(!staging.path().join("evil-llama-server").exists());
        assert!(!src.path().join("evil-llama-server").exists());
        // Atomic failure: staging wiped.
        assert!(!staging.path().exists());
    }

    #[test]
    fn tar_absolute_path_rejected() {
        let src = tmp_dir("x-tar-abs");
        let archive = raw_tar_gz(&[("/etc/llama-evil", b"pwned")], src.path());
        let staging = tmp_dir("x-tar-abs-stage");
        let err = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("hostile archive path"), "{err}");
        assert!(!staging.path().exists());
    }

    #[test]
    fn tar_symlink_escape_rejected() {
        let src = tmp_dir("x-tar-link");
        let archive = make_tar_gz(
            &[
                (
                    "llama-b1/llama-server",
                    tar::EntryType::Regular,
                    Some(b"x" as &[u8]),
                ),
                // Symlink pointing OUTSIDE the bundle (absolute target) —
                // the classic tar link escape.
                (
                    "llama-b1/llama-server-evil-link",
                    tar::EntryType::Symlink,
                    None,
                ),
            ],
            src.path(),
        );
        let staging = tmp_dir("x-tar-link-stage");
        let err = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("absolute target"), "{err}");
        // Atomic failure: staging wiped, nothing usable left behind.
        assert!(!staging.path().exists());
    }

    #[test]
    fn tar_hardlink_escape_rejected() {
        // `append_link` refuses to store an escaping target, so assert the
        // escape guard directly on the validator (the extraction path
        // calls it on every link target before creating anything).
        assert!(validate_internal_link(Path::new("../../etc/passwd"), "hardlink").is_err());
        assert!(validate_internal_link(Path::new("/etc/passwd"), "hardlink").is_err());
        assert!(validate_internal_link(Path::new("../llama-server"), "symlink").is_err());
        assert!(validate_internal_link(Path::new("llama-b1/../evil.so"), "symlink").is_err());
        assert!(validate_internal_link(Path::new("malware.sh"), "hardlink").is_err());
        // Benign internal targets pass.
        assert!(validate_internal_link(Path::new("libmtmd.so.1.0.0"), "symlink").is_ok());
        assert!(validate_internal_link(Path::new("llama-b1/libggml.so.1"), "hardlink").is_ok());
    }

    #[test]
    fn tar_internal_links_allowed_and_resolved() {
        // Real llama.cpp bundles ship versioned libs as internal symlinks:
        // libmtmd.so.1 -> libmtmd.so.1.0.0.
        let src = tmp_dir("x-tar-int");
        let archive_path = src.path().join("internal-links.tar.gz");
        let archive = {
            let f = File::create(&archive_path).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let mut h = tar::Header::new_gnu();
            h.set_size(10);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(
                &mut h,
                "llama-b1/libmtmd.so.1.0.0",
                b"\0real-lib\0".as_slice(),
            )
            .unwrap();
            let mut hs = tar::Header::new_gnu();
            hs.set_entry_type(tar::EntryType::Symlink);
            hs.set_size(0);
            hs.set_cksum();
            b.append_link(&mut hs, "llama-b1/libmtmd.so.1", "libmtmd.so.1.0.0")
                .unwrap();
            b.into_inner().unwrap().finish().unwrap();
            archive_path
        };
        let staging = tmp_dir("x-tar-int-stage");
        extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap();
        let link = staging.path().join("llama-b1/libmtmd.so.1");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        let resolved = std::fs::read_link(&link).unwrap();
        assert_eq!(resolved, PathBuf::from("libmtmd.so.1.0.0"));
        // Resolves to the real file inside the bundle.
        assert!(staging.path().join("llama-b1").join(&resolved).is_file());
    }

    #[test]
    fn tar_disallowed_basename_rejected() {
        let src = tmp_dir("x-tar-name");
        let archive = make_tar_gz(
            &[(
                "llama-b1/malware.sh",
                tar::EntryType::Regular,
                Some(b"#!/bin/sh" as &[u8]),
            )],
            src.path(),
        );
        let staging = tmp_dir("x-tar-name-stage");
        let err = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not in allowlist"));
    }

    #[test]
    fn tar_oversized_entry_rejected() {
        let src = tmp_dir("x-tar-big");
        let big = vec![0u8; 64 * 1024 + 1];
        let archive = make_tar_gz(
            &[(
                "llama-b1/llama-server",
                tar::EntryType::Regular,
                Some(big.as_slice()),
            )],
            src.path(),
        );
        let staging = tmp_dir("x-tar-big-stage");
        let limits = ExtractLimits {
            max_total_bytes: 1 << 30,
            max_entry_bytes: 64 * 1024,
        };
        let err =
            extract_archive(&archive, ArchiveFormat::TarGz, staging.path(), &limits).unwrap_err();
        assert!(err.to_string().contains("per-entry cap"));
    }

    #[test]
    fn tar_total_cap_rejected() {
        let src = tmp_dir("x-tar-total");
        let chunk = vec![0u8; 32 * 1024];
        let archive = make_tar_gz(
            &[
                (
                    "llama-b1/liba.so",
                    tar::EntryType::Regular,
                    Some(chunk.as_slice()),
                ),
                (
                    "llama-b1/libb.so",
                    tar::EntryType::Regular,
                    Some(chunk.as_slice()),
                ),
                (
                    "llama-b1/libc.so",
                    tar::EntryType::Regular,
                    Some(chunk.as_slice()),
                ),
            ],
            src.path(),
        );
        let staging = tmp_dir("x-tar-total-stage");
        let limits = ExtractLimits {
            max_total_bytes: 64 * 1024,
            max_entry_bytes: 1 << 30,
        };
        let err =
            extract_archive(&archive, ArchiveFormat::TarGz, staging.path(), &limits).unwrap_err();
        assert!(err.to_string().contains("total decompressed"));
    }

    #[test]
    fn zip_parent_traversal_rejected() {
        let src = tmp_dir("x-zip-slip");
        let archive = make_zip(
            &[("../evil-llama-server.exe", Some(b"pwned" as &[u8]))],
            src.path(),
        );
        let staging = tmp_dir("x-zip-slip-stage");
        let err = extract_archive(
            &archive,
            ArchiveFormat::Zip,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("hostile zip path") || msg.contains("hostile archive path"),
            "{msg}"
        );
        assert!(!src.path().join("evil-llama-server.exe").exists());
    }

    #[test]
    fn zip_disallowed_basename_rejected() {
        let src = tmp_dir("x-zip-name");
        let archive = make_zip(
            &[("llama-b1/malware.exe", Some(b"MZ" as &[u8]))],
            src.path(),
        );
        let staging = tmp_dir("x-zip-name-stage");
        let err = extract_archive(
            &archive,
            ArchiveFormat::Zip,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not in allowlist"));
    }

    #[test]
    fn zip_symlink_entry_rejected() {
        let src = tmp_dir("x-zip-link");
        let path = src.path().join("link.zip");
        {
            let f = File::create(&path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            w.add_symlink(
                "llama-b1/llama-server-link",
                "/etc/passwd",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            w.finish().unwrap();
        }
        let staging = tmp_dir("x-zip-link-stage");
        let err = extract_archive(
            &path,
            ArchiveFormat::Zip,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("symlink"));
    }

    #[test]
    fn zip_oversized_header_rejected() {
        // Craft a zip whose stored size exceeds a tiny cap: the header
        // check fires before any read.
        let src = tmp_dir("x-zip-big");
        let payload = vec![0u8; 64 * 1024 + 1];
        let archive = make_zip(
            &[("llama-b1/libggml.dll", Some(payload.as_slice()))],
            src.path(),
        );
        let staging = tmp_dir("x-zip-big-stage");
        let limits = ExtractLimits {
            max_total_bytes: 1 << 30,
            max_entry_bytes: 64 * 1024,
        };
        let err =
            extract_archive(&archive, ArchiveFormat::Zip, staging.path(), &limits).unwrap_err();
        assert!(err.to_string().contains("per-entry cap"));
    }

    #[test]
    fn staging_not_empty_refused() {
        let src = tmp_dir("x-stage-full");
        let archive = make_tar_gz(
            &[(
                "llama-b1/llama-server",
                tar::EntryType::Regular,
                Some(b"x" as &[u8]),
            )],
            src.path(),
        );
        let staging = tmp_dir("x-stage-full-target");
        std::fs::write(staging.path().join("existing.txt"), b"keep out").unwrap();
        let err = extract_archive(
            &archive,
            ArchiveFormat::TarGz,
            staging.path(),
            &ExtractLimits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not empty"));
        assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 1);
    }
}
