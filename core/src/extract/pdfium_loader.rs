//! Runtime binding to the PDFium dynamic library.
//!
//! pdfium-render does not compile PDFium; it loads `libpdfium.so`
//! (`pdfium.dll` on Windows, `libpdfium.dylib` on macOS) at runtime. The
//! library is staged by `scripts/fetch-pdfium.sh` into
//! `resources/binaries/pdfium/` (gitignored).
//!
//! Resolution order:
//! 1. `DIFFLENS_PDFIUM_PATH` environment variable — full path to the library
//!    *file*. When set, it must bind; we never silently fall back from an
//!    explicit override (this is what CI sets to assert PDF tests run).
//! 2. Repo-relative staged copy: `<repo>/resources/binaries/pdfium/…`,
//!    located relative to this crate's manifest dir (works for
//!    `cargo test`/`build` from the workspace) then relative to the current
//!    directory (works for packaged runs rooted at the repo).
//! 3. System library search path via `bind_to_system_library` (warned).

use pdfium_render::prelude::*;
use std::path::PathBuf;

/// Name of the dynamic library file per operating system.
const LIBRARY_FILE: &str = if cfg!(target_os = "windows") {
    "bin/pdfium.dll"
} else if cfg!(target_os = "macos") {
    "lib/libpdfium.dylib"
} else {
    "lib/libpdfium.so"
};

/// Environment variable holding the full path to the PDFium library file.
pub const DIFFLENS_PDFIUM_PATH_ENV: &str = "DIFFLENS_PDFIUM_PATH";

/// A bound PDFium instance plus any loader warnings.
pub struct LoadedPdfium {
    pub pdfium: Pdfium,
    pub warnings: Vec<String>,
}

/// Candidate staged-library paths, in resolution order.
fn staged_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        out.push(
            PathBuf::from(manifest)
                .join("..")
                .join("resources")
                .join("binaries")
                .join("pdfium")
                .join(LIBRARY_FILE),
        );
    }
    out.push(
        PathBuf::from("resources")
            .join("binaries")
            .join("pdfium")
            .join(LIBRARY_FILE),
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

    for candidate in staged_candidates() {
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
            "PDFium library not found: no {DIFFLENS_PDFIUM_PATH_ENV} override, no staged copy \
             (run scripts/fetch-pdfium.sh), and system bind failed: {e:?}"
        )),
    }
}

fn bind(path: &std::path::Path, warnings: Vec<String>) -> Result<LoadedPdfium, String> {
    let bindings = Pdfium::bind_to_library(path).map_err(|e| e.to_string())?;
    Ok(LoadedPdfium {
        pdfium: Pdfium::new(bindings),
        warnings,
    })
}
