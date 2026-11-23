//! Facts about this build that a caller reports, such as `hibi version`
//! (spec section 15.3).

/// The version of the hibi engine. Also the `engine` column of the cache
/// (section 11.3).
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The store format version: the `version` key of `.hibi/hibi.toml`
/// (section 11.1).
pub const STORE_VERSION: u32 = 1;

/// The grammar behind each code class: `(class, crate, version)`. The
/// versions are the `=` pins of the root `Cargo.toml` and are part of the
/// `ast-v1` definition (section 8.4). A test compares them with the
/// manifest.
pub const GRAMMARS: [(&str, &str, &str); 6] = [
    ("typescript", "tree-sitter-typescript", "0.23.2"),
    ("tsx", "tree-sitter-typescript", "0.23.2"),
    ("python", "tree-sitter-python", "0.25.0"),
    ("rust", "tree-sitter-rust", "0.24.2"),
    ("go", "tree-sitter-go", "0.25.0"),
    ("java", "tree-sitter-java", "0.23.5"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammars_equal_the_pins_of_the_root_manifest() {
        let manifest = include_str!("../../../Cargo.toml");
        let mut pinned = 0;
        for line in manifest.lines() {
            let Some((name, rest)) = line.split_once(" = ") else {
                continue;
            };
            if !name.starts_with("tree-sitter-") {
                continue;
            }
            pinned += 1;
            let version = rest
                .trim()
                .strip_prefix("\"=")
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or_else(|| panic!("{name} is not pinned with `=`: {rest}"));
            let listed: Vec<_> = GRAMMARS.iter().filter(|g| g.1 == name).collect();
            assert!(!listed.is_empty(), "{name} is not in GRAMMARS");
            for g in listed {
                assert_eq!(g.2, version, "{name}");
            }
        }
        assert_eq!(pinned, 5, "the manifest pins five grammar crates");
    }

    #[test]
    fn every_code_class_has_one_grammar() {
        let classes: Vec<&str> = GRAMMARS.iter().map(|g| g.0).collect();
        assert_eq!(
            classes,
            ["typescript", "tsx", "python", "rust", "go", "java"]
        );
    }

    #[test]
    fn the_engine_version_is_the_package_version() {
        assert_eq!(ENGINE_VERSION, env!("CARGO_PKG_VERSION"));
        assert_eq!(STORE_VERSION, 1);
    }
}
