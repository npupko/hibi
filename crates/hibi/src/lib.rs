//! hibi records that a statement was checked against evidence, and later
//! reports what changed since the check and which units in a declared scope
//! have no claim (spec section 1).
//!
//! With every feature off the crate still provides `model`, `canon`,
//! `locate`, `text`, `duration`, `engine` and the `Store` and `Provider`
//! traits (spec section 2.2), so an embedder can supply its own store and
//! providers.

pub mod build_info;
pub mod canon;
pub mod config;
pub mod duration;
pub mod engine;
pub mod error;
pub mod fingerprint;
pub mod git;
pub mod locate;
pub mod model;
pub mod provider;
pub mod store;
pub mod text;

#[cfg(feature = "cache")]
pub mod cache;
#[cfg(feature = "file")]
pub mod code;
#[cfg(feature = "judge")]
pub mod judge;
#[cfg(feature = "file")]
pub mod prose;

pub use error::{Code, Error};
