//! Writes `schemas/<name>.schema.json` for every name of `hibi schema`
//! (spec section 2.7), and removes schema files of names that no longer
//! exist. Run from anywhere: `cargo run -p hibi-cli --example write-schemas`.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    std::fs::create_dir_all(&dir)?;
    let expected: Vec<String> = hibi_cli::schema::NAMES
        .iter()
        .map(|n| hibi_cli::schema::file_name(n))
        .collect();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".schema.json") && !expected.contains(&name) {
            std::fs::remove_file(entry.path())?;
        }
    }
    for name in hibi_cli::schema::NAMES {
        let text = hibi_cli::schema::text(name).expect("every listed name has a schema");
        std::fs::write(dir.join(hibi_cli::schema::file_name(name)), text)?;
    }
    Ok(())
}
