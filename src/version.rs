const GIT_HASH: &str = env!("GIT_HASH");

/// `0.1.0 (a1b2c3d)`: the crate version and the commit the binary was built from.
pub fn label() -> &'static str {
    static LABEL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LABEL.get_or_init(|| format!("{} ({})", env!("CARGO_PKG_VERSION"), &GIT_HASH[..GIT_HASH.len().min(7)]))
}
