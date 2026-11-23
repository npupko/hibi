//! The Claude Code plugin manifest carries the workspace version.

use std::path::Path;

/// `version` in plugins/hibi-cli/.claude-plugin/plugin.json equals the
/// workspace version, so a release ships a plugin with the same version as
/// the binary.
#[test]
fn plugin_manifest_version_equals_the_workspace_version() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/hibi-cli/.claude-plugin/plugin.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let manifest: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "`version` in plugins/hibi-cli/.claude-plugin/plugin.json must equal \
         `[workspace.package] version` in Cargo.toml. Update plugin.json in the \
         same change as the version bump."
    );
}
