//! Writes `plugins/hibi-cli/skills/hibi/references/cli-reference.md` from the
//! clap definition (spec section 2.7). Run from anywhere:
//! `cargo run -p hibi-cli --example cli-reference`.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(hibi_cli::reference::PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, hibi_cli::reference::render())
}
