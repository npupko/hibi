//! The fingerprint profiles `prose-v1` and `ast-v1` and the fingerprint
//! string format (spec section 8).
//!
//! Always compiled, also without feature `file`: an external provider such as
//! the reference provider uses `prose-v1`. The digest functions take strings
//! (the unit text, or the class and the semantic stream that `code` builds),
//! so this module needs no tree-sitter. The string type is
//! [`crate::model::Fingerprint`].
