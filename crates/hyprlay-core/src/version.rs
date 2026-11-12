//! Strict `major.minor.patch` version comparison for the tray's
//! restart-for-the-new-build notice.
//!
//! Deliberately not semver: only the three numeric components are understood.
//! A pre-release or otherwise decorated token parses as `None`, and an
//! unparsable side never counts as older — a drift notice is a hint, so a
//! version this reader cannot judge stays silent (the running daemon may be
//! a local build with a `-dirty`-style suffix).

/// Parse `0.4.0` or `v0.4.0`; anything else, including suffixes such as
/// `-rc1`, is `None`.
pub fn parse(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    // A fourth component means this is not the shape we understand.
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether the running version predates the on-disk one. False when either
/// side is empty or unparsable.
pub fn is_older(running: &str, on_disk: &str) -> bool {
    match (parse(running), parse(on_disk)) {
        (Some(running), Some(on_disk)) => running < on_disk,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_v_prefixed_versions() {
        assert_eq!(parse("0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse("1.20.3"), Some((1, 20, 3)));
    }

    #[test]
    fn rejects_suffixes_and_junk() {
        assert_eq!(parse("0.4.0-rc1"), None);
        assert_eq!(parse("0.4"), None);
        assert_eq!(parse("0.4.0.1"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("nonsense"), None);
    }

    #[test]
    fn orders_numeric_components_not_text() {
        assert!(is_older("0.9.9", "0.10.0"));
        assert!(!is_older("0.10.0", "0.9.9"));
    }

    #[test]
    fn equal_versions_are_not_older() {
        assert!(!is_older("0.4.0", "0.4.0"));
        assert!(!is_older("v0.4.0", "0.4.0"));
    }

    #[test]
    fn unparsable_or_empty_sides_are_never_older() {
        assert!(!is_older("", "0.4.0"));
        assert!(!is_older("0.4.0", ""));
        assert!(!is_older("junk", "0.4.0"));
        assert!(!is_older("0.3.0", "junk"));
    }
}
