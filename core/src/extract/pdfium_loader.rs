//! Runtime binding to the PDFium dynamic library.
//!
//! pdfium-render does not compile PDFium; it loads `libpdfium.so`
//! (`pdfium.dll` on Windows, `libpdfium.dylib` on macOS) at runtime.
//!
//! Resolution order:
//! 1. `DIFFLENS_PDFIUM_PATH` environment variable — full path to the library
//!    *file*. When set, it must bind; we never silently fall back from an
//!    explicit override (this is what CI sets to assert PDF tests run).
//! 2. **Bundled app resources**: `DIFFLENS_RESOURCE_DIR` (set by the Tauri
//!    shell at startup to the app's resource directory) →
//!    `<dir>/pdfium/<lib>`; then a fallback next to the current executable
//!    (`<exe_dir>/pdfium/<lib>`) for portable layouts.
//! 3. Repo-relative staged copy: `<repo>/resources/binaries/pdfium/…`,
//!    located relative to this crate's manifest dir (works for
//!    `cargo test`/`build` from the workspace) then relative to the current
//!    directory (works for packaged runs rooted at the repo). This is the
//!    developer path staged by `scripts/fetch-pdfium.sh`.
//! 4. System library search path via `bind_to_system_library` (warned).
//!
//! The bundled copy ships inside every release package (see
//! `scripts/fetch-pdfium.sh --bundle` + `release.yml`), so installed apps
//! never depend on the developer script or a system install.

use pdfium_render::prelude::*;
use std::path::PathBuf;

/// Bundled library file name (flat, under `pdfium/`), per operating system.
const BUNDLED_FILE: &str = if cfg!(target_os = "windows") {
    "pdfium.dll"
} else if cfg!(target_os = "macos") {
    "libpdfium.dylib"
} else {
    "libpdfium.so"
};

/// Name of the dynamic library file per operating system within the
/// developer-staged bblanchon archive layout.
const STAGED_FILE: &str = if cfg!(target_os = "windows") {
    "bin/pdfium.dll"
} else if cfg!(target_os = "macos") {
    "lib/libpdfium.dylib"
} else {
    "lib/libpdfium.so"
};

/// Environment variable holding the full path to the PDFium library file.
pub const DIFFLENS_PDFIUM_PATH_ENV: &str = "DIFFLENS_PDFIUM_PATH";
/// Environment variable set by the Tauri shell: the app resource directory
/// (contains `pdfium/<BUNDLED_FILE>` in packaged builds).
pub const DIFFLENS_RESOURCE_DIR_ENV: &str = "DIFFLENS_RESOURCE_DIR";

/// A bound PDFium instance plus any loader warnings.
pub struct LoadedPdfium {
    pub pdfium: Pdfium,
    pub warnings: Vec<String>,
}

/// Candidate bundled-library paths (flat layout inside app resources).
fn bundled_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(resource_dir) = std::env::var(DIFFLENS_RESOURCE_DIR_ENV)
        && !resource_dir.trim().is_empty()
    {
        out.push(
            PathBuf::from(resource_dir)
                .join("pdfium")
                .join(BUNDLED_FILE),
        );
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        out.push(exe_dir.join("pdfium").join(BUNDLED_FILE));
    }
    out
}

/// Candidate staged-library paths (developer checkout layout).
fn staged_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        out.push(
            PathBuf::from(manifest)
                .join("..")
                .join("resources")
                .join("binaries")
                .join("pdfium")
                .join(STAGED_FILE),
        );
    }
    out.push(
        PathBuf::from("resources")
            .join("binaries")
            .join("pdfium")
            .join(STAGED_FILE),
    );
    out
}

/// Bind to PDFium, following the resolution order above. Never panics; all
/// failure modes return a descriptive error string.
pub fn load() -> Result<LoadedPdfium, String> {
    let mut warnings = Vec::new();

    if let Ok(explicit) = std::env::var(DIFFLENS_PDFIUM_PATH_ENV) {
        let path = PathBuf::from(explicit);
        return bind(&path, warnings)
            .map_err(|e| format!("{DIFFLENS_PDFIUM_PATH_ENV}={path:?} could not be bound: {e}"));
    }

    for candidate in bundled_candidates().into_iter().chain(staged_candidates()) {
        if candidate.is_file() {
            return bind(&candidate, warnings)
                .map_err(|e| format!("staged PDFium at {:?} could not be bound: {e}", candidate));
        }
    }

    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            warnings.push(
                "PDFium bound from system library search path (no staged copy found)".to_string(),
            );
            Ok(LoadedPdfium {
                pdfium: Pdfium::new(bindings),
                warnings,
            })
        }
        Err(e) => Err(format!(
            "PDFium library not found: no {DIFFLENS_PDFIUM_PATH_ENV} override, no bundled or \
             staged copy, and system bind failed: {e:?}. This is a packaging bug in an installed \
             app — please report it at https://github.com/vibecoder880/difflens/issues"
        )),
    }
}

fn bind(path: &std::path::Path, warnings: Vec<String>) -> Result<LoadedPdfium, String> {
    let bindings = match Pdfium::bind_to_library(path) {
        Ok(bindings) => bindings,
        // pdfium-render keeps one bindings instance per thread; a second
        // load() on the same thread (the Phase-06 OCR fallback re-opens the
        // same document after extraction) must reuse it rather than fail —
        // it is the same staged library in every resolution order.
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => {
            return Ok(LoadedPdfium {
                pdfium: Pdfium::default(),
                warnings,
            });
        }
        Err(e) => return Err(e.to_string()),
    };
    Ok(LoadedPdfium {
        pdfium: Pdfium::new(bindings),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_candidates_use_resource_dir_when_set() {
        // SAFETY: test process; env races are not a concern here because
        // this test both writes and reads the variable.
        unsafe {
            std::env::set_var(DIFFLENS_RESOURCE_DIR_ENV, "/tmp/difflens-test-resources");
        }
        let cands = bundled_candidates();
        // SAFETY: cleanup of the same test-owned variable.
        unsafe {
            std::env::remove_var(DIFFLENS_RESOURCE_DIR_ENV);
        }
        let expected = std::path::PathBuf::from("/tmp/difflens-test-resources")
            .join("pdfium")
            .join(BUNDLED_FILE);
        assert!(cands.contains(&expected), "candidates: {cands:?}");
    }

    #[test]
    fn bundled_file_name_matches_host_os() {
        if cfg!(target_os = "windows") {
            assert_eq!(BUNDLED_FILE, "pdfium.dll");
        } else if cfg!(target_os = "macos") {
            assert_eq!(BUNDLED_FILE, "libpdfium.dylib");
        } else {
            assert_eq!(BUNDLED_FILE, "libpdfium.so");
        }
    }
}
