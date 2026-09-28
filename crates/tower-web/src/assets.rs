//! The Topcoat browser runtime, served from memory (spike S4.A): the
//! vendored copy of `topcoat-runtime`'s `browser/dist/index.js` plus an
//! in-code catalog, so the binary needs neither an asset directory nor the
//! `topcoat asset bundle` build step.

use axum::http::header;
use axum::response::IntoResponse;
use topcoat::asset::{AssetConfig, Manifest, ManifestEntry};

/// URL prefix the catalog resolves assets under.
pub const PREFIX: &str = "/ui/assets";
/// Versioned name: the URL changes with the pinned crate, so the long
/// immutable cache below is safe.
pub const SCRIPT_FILE: &str = "topcoat-runtime-0.9.0.js";
const SCRIPT: &str = include_str!("../assets/topcoat-runtime-0.9.0.js");

pub fn config() -> AssetConfig {
    AssetConfig::hosted_at(
        PREFIX,
        Manifest {
            version: topcoat::asset::MANIFEST_VERSION,
            assets: vec![ManifestEntry {
                id: topcoat::runtime::SCRIPT.id(),
                file: SCRIPT_FILE.to_string(),
                hash: "vendored".to_string(),
                content_type: "text/javascript".to_string(),
            }],
        },
    )
}

pub async fn runtime_script() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        SCRIPT,
    )
}

#[cfg(test)]
mod tests {
    use topcoat::asset::RawAsset;

    #[test]
    fn vendored_runtime_matches_the_pinned_crate() {
        // The asset record for `topcoat::runtime::SCRIPT` is embedded in
        // this binary (config() references it) and names the source file
        // in the registry copy of the crate this build compiled against.
        let _ = super::config();
        let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let raw = RawAsset::find_in_binary(&exe)
            .into_iter()
            .find(|a| a.id() == topcoat::runtime::SCRIPT.id())
            .expect("runtime script asset record in the test binary");
        let upstream = std::fs::read_to_string(raw.resolved_path()).unwrap();
        assert!(
            upstream == super::SCRIPT,
            "assets/{} differs from {} — re-vendor it (see assets/README.md)",
            super::SCRIPT_FILE,
            raw.resolved_path().display()
        );
    }
}
