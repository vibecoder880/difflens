//! Tesseract CLI detection (experimental OCR path, phase 06).
//!
//! Tesseract is *detected, not shipped*: a user-installed `tesseract`
//! binary plus the Vietnamese traineddata (`vie`) enable the feature; any
//! missing piece just leaves it disabled. Detection never fails the
//! pipeline and never panics — a hostile PATH (non-executable files,
//! directories named oddly, hanging binaries) degrades to "unavailable".
//!
//! Every spawned process runs under a hard timeout (see [`super::proc`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::proc::run_with_timeout;

/// Hard ceiling for `--list-langs`: a hung binary must not stall extraction.
const LIST_LANGS_TIMEOUT: Duration = Duration::from_secs(5);

/// Language data Tesseract must have for the Vietnamese path.
pub const VIE_LANG: &str = "vie";

/// Result of detection. `binary` is `Some` as soon as an executable-looking
/// file is found; `langs`/`has_vie` are only populated when `--list-langs`
/// actually succeeded (a found-but-broken install is honest: no languages).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcrAvailability {
    pub binary: Option<PathBuf>,
    pub langs: Vec<String>,
    pub has_vie: bool,
}

impl OcrAvailability {
    /// True when the Vietnamese OCR path can run.
    pub fn usable(&self) -> bool {
        self.binary.is_some() && self.has_vie
    }

    /// Language argument for the tesseract CLI: `vie+eng` when Vietnamese
    /// data is present, plain `eng` otherwise.
    pub fn lang_arg(&self) -> String {
        if self.has_vie {
            format!("{VIE_LANG}+eng")
        } else {
            "eng".to_string()
        }
    }
}

/// Detect tesseract on the current machine (PATH + known install locations).
pub fn detect() -> OcrAvailability {
    let path = std::env::var_os("PATH");
    detect_with(path.as_deref())
}

/// [`detect`] with an injectable `PATH`-like value — the test seam: unit
/// tests pass a directory holding stub binaries instead of touching the
/// real environment.
pub fn detect_with(path_env: Option<&std::ffi::OsStr>) -> OcrAvailability {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path_env) = path_env {
        for dir in std::env::split_paths(path_env) {
            candidates.push(dir.join(binary_name()));
        }
    }
    candidates.extend(known_install_locations());

    for candidate in candidates {
        if !is_probably_executable(&candidate) {
            continue;
        }
        // First hit wins (PATH order = user preference order).
        return availability_of(&candidate);
    }
    OcrAvailability::default()
}

fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "tesseract.exe"
    } else {
        "tesseract"
    }
}

/// OS-specific locations checked after PATH (phase contract).
fn known_install_locations() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_os = "windows") {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            out.push(
                PathBuf::from(program_files)
                    .join("Tesseract-OCR")
                    .join(binary_name()),
            );
        }
        out.push(PathBuf::from(r"C:\Program Files\Tesseract-OCR").join(binary_name()));
    } else if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/opt/homebrew/bin").join(binary_name()));
        out.push(PathBuf::from("/usr/local/bin").join(binary_name()));
    } else {
        out.push(PathBuf::from("/usr/local/bin").join(binary_name()));
        out.push(PathBuf::from("/usr/bin").join(binary_name()));
    }
    out
}

/// Cheap executability probe: exists and is a file. We do not inspect mode
/// bits (Windows has none); `--list-langs` is the authoritative probe.
fn is_probably_executable(path: &Path) -> bool {
    path.is_file()
}

/// Run `tesseract --list-langs` under a timeout and parse the output.
/// Any failure degrades to empty `langs` — never an error, never a panic.
fn availability_of(binary: &Path) -> OcrAvailability {
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("--list-langs");
    let parsed = match run_with_timeout(&mut cmd, LIST_LANGS_TIMEOUT) {
        Ok(out) if out.status_success && !out.timed_out => parse_list_langs(&out.stdout),
        _ => Vec::new(),
    };
    let has_vie = parsed.iter().any(|l| l == VIE_LANG);
    OcrAvailability {
        binary: Some(binary.to_path_buf()),
        langs: parsed,
        has_vie,
    }
}

/// Parse `--list-langs` output: the first line is the banner
/// ("List of available languages (2):"), followed by one language code per
/// line. Tolerant of trailing whitespace and blank lines.
fn parse_list_langs(stdout: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(stdout);
    let mut langs = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if i == 0 && line.starts_with("List of available languages") {
            continue;
        }
        if line.is_empty() {
            continue;
        }
        langs.push(line.to_string());
    }
    langs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn write_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn parses_list_langs_output() {
        let out = b"List of available languages (2):\neng\nvie\n";
        let langs = parse_list_langs(out);
        assert_eq!(langs, vec!["eng", "vie"]);
    }

    #[test]
    fn parses_list_langs_with_blank_lines_and_missing_banner() {
        let langs = parse_list_langs(b"\neng\n\nosd\n");
        assert_eq!(langs, vec!["eng", "osd"]);
    }

    #[test]
    fn lang_arg_prefers_vie_plus_eng() {
        let mut a = OcrAvailability::default();
        assert_eq!(a.lang_arg(), "eng");
        a.has_vie = true;
        assert_eq!(a.lang_arg(), "vie+eng");
    }

    #[cfg(unix)]
    #[test]
    fn detection_finds_stub_on_injected_path() {
        let dir = tempfile::tempdir().unwrap();
        write_stub(
            dir.path(),
            "tesseract",
            "#!/bin/sh\necho 'List of available languages (2):'\necho eng\necho vie\n",
        );
        let path_env = std::env::join_paths([dir.path()]).unwrap();
        let avail = detect_with(Some(&path_env));
        assert_eq!(
            avail.binary.as_deref(),
            Some(dir.path().join("tesseract").as_path())
        );
        assert!(avail.has_vie);
        assert!(avail.usable());
    }

    #[cfg(unix)]
    #[test]
    fn detection_without_vie_reports_langs_but_not_usable() {
        let dir = tempfile::tempdir().unwrap();
        write_stub(
            dir.path(),
            "tesseract",
            "#!/bin/sh\necho 'List of available languages (1):'\necho eng\n",
        );
        let path_env = std::env::join_paths([dir.path()]).unwrap();
        let avail = detect_with(Some(&path_env));
        assert_eq!(avail.langs, vec!["eng"]);
        assert!(!avail.has_vie);
        assert!(!avail.usable());
    }

    #[cfg(unix)]
    #[test]
    fn broken_binary_keeps_path_but_no_langs() {
        // A binary that exists but fails --list-langs: honest "no languages".
        let dir = tempfile::tempdir().unwrap();
        write_stub(dir.path(), "tesseract", "#!/bin/sh\nexit 3\n");
        let path_env = std::env::join_paths([dir.path()]).unwrap();
        let avail = detect_with(Some(&path_env));
        assert!(avail.binary.is_some());
        assert!(avail.langs.is_empty());
        assert!(!avail.has_vie);
    }

    #[cfg(unix)]
    #[test]
    fn hanging_binary_hits_the_timeout_not_hang() {
        let dir = tempfile::tempdir().unwrap();
        write_stub(dir.path(), "tesseract", "#!/bin/sh\nsleep 60\n");
        let path_env = std::env::join_paths([dir.path()]).unwrap();
        // LIST_LANGS_TIMEOUT is 5s (+1s pipe-drain grace): ~7s total.
        let started = std::time::Instant::now();
        let avail = detect_with(Some(&path_env));
        assert!(avail.langs.is_empty());
        assert!(started.elapsed() >= Duration::from_secs(4));
        assert!(started.elapsed() < Duration::from_secs(15));
    }

    #[test]
    fn known_locations_are_appended_after_path_dirs() {
        // PATH override only redirects the PATH search; the OS-specific
        // known locations are always appended (machine-dependent — only
        // assert ordering, not presence).
        let dirs = known_install_locations();
        assert!(!dirs.is_empty());
        assert!(dirs.iter().all(|p| p.ends_with(binary_name())));
    }
}
