//! Release metadata for `hyprlay update`: the GitHub `releases/latest`
//! payload, the asset-name table the CI matrix publishes, version comparison,
//! and digest verification. Everything here is pure, so the network and
//! filesystem boundaries stay in [`host`](super::host).

use serde::Deserialize;

/// The repository the releases come from.
pub const REPO: &str = "FAZuH/hyprlay";

/// Latest non-draft, non-prerelease release.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/FAZuH/hyprlay/releases/latest";

/// One downloadable release asset. GitHub reports the checksum it computed at
/// upload time in `digest`, so no separate checksums file is needed.
#[derive(Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
}

/// The subset of the release payload the updater reads.
#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn parse(body: &str) -> Result<Self, String> {
        serde_json::from_str(body).map_err(|error| format!("unexpected release response: {error}"))
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }

    /// The tag without its `v` prefix, for display.
    pub fn version(&self) -> &str {
        self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name)
    }
}

/// The `(cli, daemon)` asset names for a target, mirroring the names the
/// release workflow uploads. `None` means no prebuilt binaries exist for this
/// platform, so updating would have nothing to download.
pub fn asset_names(os: &str, arch: &str) -> Option<(String, String)> {
    let suffix = match (os, arch) {
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "windows-x64.exe",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        _ => return None,
    };
    Some((format!("hyprlay-{suffix}"), format!("hyprlayd-{suffix}")))
}

/// Parse `vMAJOR.MINOR.PATCH` (the leading `v` optional). A prerelease or
/// build suffix is not a release this updater installs, so it parses as
/// `None`.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `candidate` is a version worth installing over `current`.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    match (parse_version(current), parse_version(candidate)) {
        (Some(current), Some(candidate)) => candidate > current,
        _ => false,
    }
}

/// Verify `bytes` against GitHub's `sha256:<hex>` digest. A missing or
/// unrecognised digest fails closed.
pub fn digest_matches(bytes: &[u8], digest: &str) -> bool {
    use sha2::Digest;
    use sha2::Sha256;

    let Some(expected) = digest.strip_prefix("sha256:") else {
        return false;
    };
    let actual = Sha256::digest(bytes);
    let mut hex = String::with_capacity(actual.len() * 2);
    for byte in actual {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex.eq_ignore_ascii_case(expected.trim())
}

#[cfg(test)]
mod tests {
    use super::Release;
    use super::asset_names;
    use super::digest_matches;
    use super::is_newer;
    use super::parse_version;

    #[test]
    fn asset_names_mirror_the_release_matrix() {
        assert_eq!(
            asset_names("linux", "x86_64"),
            Some(("hyprlay-linux-x64".into(), "hyprlayd-linux-x64".into()))
        );
        assert_eq!(
            asset_names("windows", "x86_64"),
            Some((
                "hyprlay-windows-x64.exe".into(),
                "hyprlayd-windows-x64.exe".into()
            ))
        );
        assert_eq!(
            asset_names("macos", "x86_64"),
            Some(("hyprlay-macos-x64".into(), "hyprlayd-macos-x64".into()))
        );
        assert_eq!(
            asset_names("macos", "aarch64"),
            Some(("hyprlay-macos-arm64".into(), "hyprlayd-macos-arm64".into()))
        );
    }

    #[test]
    fn a_target_without_prebuilt_binaries_has_no_asset_names() {
        assert_eq!(asset_names("linux", "aarch64"), None);
        assert_eq!(asset_names("freebsd", "x86_64"), None);
    }

    #[test]
    fn versions_parse_with_or_without_the_tag_prefix() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
    }

    #[test]
    fn prerelease_and_malformed_tags_are_not_releases() {
        for tag in ["v0.4.0-rc1", "v0.4", "v0.4.0.1", "latest", "v"] {
            assert_eq!(parse_version(tag), None, "{tag}");
        }
    }

    #[test]
    fn numeric_order_beats_lexicographic_order() {
        assert!(is_newer("0.4.0", "v0.10.0"));
        assert!(is_newer("0.9.9", "v1.0.0"));
        assert!(!is_newer("0.10.0", "v0.9.9"));
        assert!(!is_newer("0.4.0", "v0.4.0"));
    }

    #[test]
    fn an_unparsable_tag_is_never_newer() {
        assert!(!is_newer("0.4.0", "nightly"));
    }

    #[test]
    fn a_matching_digest_is_accepted() {
        // sha256 of the empty input
        let digest = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(digest_matches(b"", digest));
        assert!(!digest_matches(b"x", digest));
    }

    #[test]
    fn a_missing_or_foreign_digest_fails_closed() {
        assert!(!digest_matches(b"", ""));
        assert!(!digest_matches(b"", "md5:abc"));
        assert!(!digest_matches(b"", "sha256:zzzz"));
    }

    #[test]
    fn the_release_payload_parses_into_assets() {
        let body = r#"{
            "tag_name": "v0.5.0",
            "assets": [
                {
                    "name": "hyprlay-linux-x64",
                    "browser_download_url": "https://example.invalid/hyprlay-linux-x64",
                    "digest": "sha256:abc"
                }
            ]
        }"#;
        let release = Release::parse(body).expect("payload parses");
        assert_eq!(release.version(), "0.5.0");
        let asset = release.asset("hyprlay-linux-x64").expect("asset found");
        assert_eq!(asset.digest.as_deref(), Some("sha256:abc"));
        assert!(release.asset("hyprlayd-linux-x64").is_none());
    }
}
