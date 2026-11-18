//! Versioned manifest of pinned download artifacts (llama-server releases
//! + GGUF models).
//!
//! **Storage decision (documented per phase-05 contract):** the manifest is
//! *checked into* `resources/manifests/llm-manifest.json` **and embedded
//! into the library** via `include_str!` (`embedded_manifest_str` /
//! [`LlmManifest::embedded`]). Embedding keeps core OS-agnostic and
//! testable headless (no resource-path resolution needed), while the
//! checked-in file stays the reviewable source of truth that CI and
//! humans diff. `LlmManifest::from_path` still exists so the Tauri shell
//! can load a fresher manifest shipped next to the binary if that ever
//! becomes necessary.

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// The checked-in manifest, embedded at compile time.
pub const EMBEDDED_MANIFEST_JSON: &str =
    include_str!("../../../resources/manifests/llm-manifest.json");

/// Supported archive container formats for server bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    /// gzip-compressed tar (llama.cpp linux/macos bundles).
    #[serde(rename = "tar.gz")]
    TarGz,
    /// plain zip (llama.cpp windows bundle).
    Zip,
}

/// Role a model entry plays. `fallback` is the only role allowed to carry
/// `verified: false` (red-team F6); `test` is used solely by the ignored
/// smoke test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Primary,
    Fallback,
    Test,
}

/// One pinned llama.cpp release asset (per OS triple).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerAsset {
    /// Short platform id, e.g. `linux-x64`.
    pub id: String,
    /// Rust target triple this asset maps to.
    pub target_triple: String,
    pub archive: ArchiveFormat,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    /// File name of the llama-server launcher inside the bundle.
    pub server_basename: String,
    /// `false` ⇒ checksum/provenance could not be verified at authoring
    /// time; such assets are never installed by the manager.
    pub verified: bool,
    /// How/when the checksum was verified (provenance evidence).
    #[serde(default)]
    pub verification: String,
}

/// llama.cpp release block: repo + tag + per-OS assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaServerRelease {
    pub repo: String,
    pub tag: String,
    #[serde(default)]
    pub commit: Option<String>,
    pub license: String,
    #[serde(default)]
    pub license_url: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub notes: String,
    pub assets: Vec<ServerAsset>,
}

/// One pinned GGUF model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub role: ModelRole,
    pub repo: String,
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub license: String,
    #[serde(default)]
    pub license_url: Option<String>,
    #[serde(default)]
    pub min_ram_gb: u64,
    pub verified: bool,
    #[serde(default)]
    pub verification: String,
    #[serde(default)]
    pub notes: String,
}

/// Root manifest document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmManifest {
    pub manifest_version: u32,
    pub generated: String,
    #[serde(default)]
    pub generator_notes: String,
    pub llama_server: LlamaServerRelease,
    pub models: Vec<ModelEntry>,
}

impl LlmManifest {
    /// Parse + validate a manifest from JSON text. Never panics on
    /// malformed input — every problem surfaces as `CoreError::Download`.
    pub fn parse(json: &str) -> CoreResult<Self> {
        let manifest: LlmManifest = serde_json::from_str(json)
            .map_err(|e| CoreError::Download(format!("manifest parse failed: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Parse + validate a manifest from a file path.
    pub fn from_path(path: &Path) -> CoreResult<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| {
            CoreError::Download(format!("cannot read manifest {}: {e}", path.display()))
        })?;
        Self::parse(&json)
    }

    /// The embedded, compile-time-pinned manifest (parsed once).
    pub fn embedded() -> &'static LlmManifest {
        static EMBEDDED: OnceLock<LlmManifest> = OnceLock::new();
        EMBEDDED.get_or_init(|| {
            // The embedded manifest is checked in and CI-tested; a parse
            // failure here is a build bug. Avoid panicking at runtime in
            // production: fall back to an empty-but-valid manifest? No —
            // an unparsable embedded manifest means the release is broken;
            // we surface the error loudly via expect, which CI would have
            // caught. (Documented trade-off; no silent bad state.)
            LlmManifest::parse(EMBEDDED_MANIFEST_JSON)
                .unwrap_or_else(|e| panic!("embedded llm-manifest.json is invalid: {e}"))
        })
    }

    /// The asset for the current build's target triple.
    pub fn server_asset_for_current_os(&self) -> Option<&ServerAsset> {
        let triple = current_target_triple();
        self.server_asset_for_triple(triple)
    }

    /// The asset matching `triple` (exact match).
    pub fn server_asset_for_triple(&self, triple: &str) -> Option<&ServerAsset> {
        self.llama_server
            .assets
            .iter()
            .find(|a| a.target_triple == triple)
    }

    /// Model entry by id.
    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Full validation: checksum shape, https URLs, provenance fields,
    /// uniqueness, verified-flag policy. See module docs for why
    /// `verified == false` is only legal for `fallback`/`test` models.
    pub fn validate(&self) -> CoreResult<()> {
        if self.manifest_version != 1 {
            return Err(CoreError::Download(format!(
                "unsupported manifest_version {} (expected 1)",
                self.manifest_version
            )));
        }
        if self.llama_server.repo.is_empty() || self.llama_server.tag.is_empty() {
            return Err(CoreError::Download(
                "llama_server release missing repo/tag provenance".into(),
            ));
        }
        if self.llama_server.license.is_empty() {
            return Err(CoreError::Download(
                "llama_server release missing license".into(),
            ));
        }
        if self.llama_server.assets.is_empty() {
            return Err(CoreError::Download("llama_server has no assets".into()));
        }
        let mut seen_ids = std::collections::HashSet::new();
        for a in &self.llama_server.assets {
            require_url(&a.url)?;
            require_sha(&a.sha256)?;
            if a.bytes == 0 {
                return Err(CoreError::Download(format!("asset {} has bytes=0", a.id)));
            }
            if a.server_basename.is_empty() {
                return Err(CoreError::Download(format!(
                    "asset {} missing server_basename",
                    a.id
                )));
            }
            if a.verification.is_empty() {
                return Err(CoreError::Download(format!(
                    "asset {} missing verification provenance",
                    a.id
                )));
            }
            if !seen_ids.insert(a.id.clone()) {
                return Err(CoreError::Download(format!("duplicate asset id {}", a.id)));
            }
            if !a.verified {
                return Err(CoreError::Download(format!(
                    "server asset {} is unverified; unverified assets must not ship",
                    a.id
                )));
            }
        }
        if self.models.is_empty() {
            return Err(CoreError::Download("manifest has no models".into()));
        }
        seen_ids.clear();
        let mut primary_count = 0usize;
        for m in &self.models {
            require_url(&m.url)?;
            require_sha(&m.sha256)?;
            if m.bytes == 0 {
                return Err(CoreError::Download(format!("model {} has bytes=0", m.id)));
            }
            if m.repo.is_empty() || m.file.is_empty() || m.license.is_empty() {
                return Err(CoreError::Download(format!(
                    "model {} missing provenance (repo/file/license)",
                    m.id
                )));
            }
            if m.verification.is_empty() {
                return Err(CoreError::Download(format!(
                    "model {} missing verification provenance",
                    m.id
                )));
            }
            if !m.verified && !matches!(m.role, ModelRole::Fallback | ModelRole::Test) {
                return Err(CoreError::Download(format!(
                    "model {} is unverified but has role {:?}; only fallback/test entries may be unverified",
                    m.id, m.role
                )));
            }
            if !seen_ids.insert(m.id.clone()) {
                return Err(CoreError::Download(format!("duplicate model id {}", m.id)));
            }
            if m.role == ModelRole::Primary {
                primary_count += 1;
            }
        }
        if primary_count != 1 {
            return Err(CoreError::Download(format!(
                "manifest must declare exactly one primary model, found {primary_count}"
            )));
        }
        Ok(())
    }
}

fn require_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("https://") {
        return Err(CoreError::Download(format!(
            "manifest url must be https: {url:?}"
        )));
    }
    Ok(())
}

fn require_sha(sha: &str) -> CoreResult<()> {
    if !crate::download::checksum::is_sha256_hex(sha) {
        return Err(CoreError::Download(format!(
            "manifest sha256 malformed: {sha:?}"
        )));
    }
    Ok(())
}

/// Build-target triple of the running binary. Compiled from the standard
/// `target_os`/`target_arch` cfgs (no external env dependency).
pub fn current_target_triple() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_parses_and_validates() {
        let m = LlmManifest::embedded();
        // Pinned tag + provenance present on every entry.
        assert!(m.llama_server.tag.starts_with('b'));
        assert_eq!(m.llama_server.license, "MIT");
        assert_eq!(m.llama_server.assets.len(), 4);
        assert!(m.server_asset_for_current_os().is_some());
        // Exactly one primary; fallback carries its own checksum.
        let primary = m.models.iter().filter(|m| m.role == ModelRole::Primary);
        assert_eq!(primary.count(), 1);
        assert!(m.model("qwen3-4b-instruct-2507-q4_k_m").is_some());
        assert!(m.model("qwen2.5-3b-instruct-q4_k_m").is_some());
        assert!(m.model("qwen3-0.6b-q8_0-test").is_some());
        // Everything shipped today is verified.
        assert!(m.models.iter().all(|m| m.verified));
        assert!(m.llama_server.assets.iter().all(|a| a.verified));
        // Test model checksum matches what we byte-verified on 2026-11-18.
        let test_model = m.model("qwen3-0.6b-q8_0-test").unwrap();
        assert_eq!(
            test_model.sha256,
            "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031"
        );
        assert_eq!(test_model.bytes, 639_446_688);
    }

    #[test]
    fn malformed_json_is_error_not_panic() {
        assert!(LlmManifest::parse("{ not json").is_err());
        assert!(LlmManifest::parse("null").is_err());
        assert!(LlmManifest::parse("{}").is_err());
    }

    fn base_manifest() -> LlmManifest {
        LlmManifest::parse(EMBEDDED_MANIFEST_JSON).unwrap()
    }

    #[test]
    fn validation_rejects_missing_provenance() {
        let mut m = base_manifest();
        m.models[0].verification.clear();
        assert!(m.validate().is_err());

        let mut m = base_manifest();
        m.llama_server.assets[0].verification.clear();
        assert!(m.validate().is_err());

        let mut m = base_manifest();
        m.llama_server.license.clear();
        assert!(m.validate().is_err());
    }

    #[test]
    fn validation_rejects_non_https_and_bad_sha() {
        let mut m = base_manifest();
        let asset = &mut m.llama_server.assets[0];
        let flipped = asset.url.replacen("https://", "http://", 1);
        asset.url = flipped;
        assert!(m.validate().is_err());

        let mut m = base_manifest();
        m.models[0].sha256 = "deadbeef".into();
        assert!(m.validate().is_err());
    }

    #[test]
    fn unverified_primary_or_asset_rejected() {
        let mut m = base_manifest();
        m.models[0].verified = false;
        assert!(m.validate().is_err());

        let mut m = base_manifest();
        m.llama_server.assets[0].verified = false;
        assert!(m.validate().is_err());
    }

    #[test]
    fn unverified_fallback_or_test_allowed() {
        let mut m = base_manifest();
        let fallback = m
            .models
            .iter_mut()
            .find(|m| m.role == ModelRole::Fallback)
            .unwrap();
        fallback.verified = false;
        assert!(m.validate().is_ok());
        let test = m
            .models
            .iter_mut()
            .find(|m| m.role == ModelRole::Test)
            .unwrap();
        test.verified = false;
        assert!(m.validate().is_ok());
    }

    #[test]
    fn duplicate_model_ids_rejected() {
        let mut m = base_manifest();
        let mut dup = m.models[1].clone();
        dup.role = ModelRole::Fallback;
        m.models.push(dup);
        assert!(m.validate().is_err());
    }

    #[test]
    fn two_primaries_rejected() {
        let mut m = base_manifest();
        m.models[1].role = ModelRole::Primary;
        assert!(m.validate().is_err());
    }

    #[test]
    fn wrong_manifest_version_rejected() {
        let mut m = base_manifest();
        m.manifest_version = 99;
        assert!(m.validate().is_err());
    }

    #[test]
    fn target_triple_lookup() {
        let m = base_manifest();
        assert!(
            m.server_asset_for_triple("x86_64-unknown-linux-gnu")
                .is_some()
        );
        assert!(
            m.server_asset_for_triple("x86_64-pc-windows-msvc")
                .is_some()
        );
        assert!(m.server_asset_for_triple("nope").is_none());
    }
}
