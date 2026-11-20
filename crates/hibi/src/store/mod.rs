//! The `Store` trait, `Ledger`, `Batch` and `StoreWarning` (spec section 2.4).
//!
//! Contract for every backend:
//!
//! - `load` returns each record once, with claims, marks and retirements each
//!   sorted by id ascending (byte order).
//! - `append` is idempotent per record id. Either it returns `Ok` and every
//!   record of the batch is durable, or it returns `Err`. After a crash the
//!   store MUST be readable and MUST NOT contain a mark or retirement whose
//!   claim is absent.
//! - `append` MUST verify that each record's id equals the id derived from
//!   its body (section 3.5) and fail with `internal` otherwise
//!   ([`Batch::verify_ids`]).
//! - `append` of a mark or retirement whose claim is neither in the store nor
//!   in the same batch fails with `internal` and writes nothing
//!   ([`Batch::verify_claims`]).
//! - The engine never deletes or rewrites a record through this trait.

#[cfg(feature = "git-store")]
pub mod git_text;

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::model::{Claim, ClaimId, Mark, Retirement};

/// A record store.
pub trait Store {
    /// Every record currently in the store.
    fn load(&self) -> Result<Ledger, Error>;
    /// Add records in one transaction. A record whose id already exists is skipped.
    fn append(&self, batch: &Batch) -> Result<(), Error>;
    /// Opaque text that changes whenever the stored record set changes.
    fn snapshot_id(&self) -> Result<String, Error>;
}

/// Every record of a store, each list sorted by id ascending.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    pub claims: Vec<Claim>,
    pub marks: Vec<Mark>,
    pub retirements: Vec<Retirement>,
    pub warnings: Vec<StoreWarning>,
}

/// Records to add in one `append`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub claims: Vec<Claim>,
    pub marks: Vec<Mark>,
    pub retirements: Vec<Retirement>,
}

/// A store warning (section 12.3), shown in the `warnings` of the JSON
/// envelope (section 15.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoreWarning {
    /// `orphan_record` or `unknown_replacement` (section 12.3).
    pub code: String,
    pub message: String,
}

/// Warning code: a mark or retirement names a claim that is not in the store.
pub const ORPHAN_RECORD: &str = "orphan_record";
/// Warning code: `replacedBy` names a claim that is not in the store.
pub const UNKNOWN_REPLACEMENT: &str = "unknown_replacement";

impl StoreWarning {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        StoreWarning {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl Ledger {
    /// The claim with this full id.
    pub fn claim(&self, id: &str) -> Option<&Claim> {
        self.claims
            .binary_search_by(|c| c.id.as_str().cmp(id))
            .ok()
            .map(|i| &self.claims[i])
    }

    /// The marks of a claim, in id order.
    pub fn marks_of<'a>(&'a self, claim: &'a ClaimId) -> impl Iterator<Item = &'a Mark> + 'a {
        self.marks.iter().filter(move |m| &m.claim == claim)
    }

    /// The retirements of a claim, in id order.
    pub fn retirements_of<'a>(
        &'a self,
        claim: &'a ClaimId,
    ) -> impl Iterator<Item = &'a Retirement> + 'a {
        self.retirements.iter().filter(move |r| &r.retires == claim)
    }

    /// True when the claim has at least one retirement.
    pub fn is_retired(&self, claim: &ClaimId) -> bool {
        self.retirements_of(claim).next().is_some()
    }
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty() && self.marks.is_empty() && self.retirements.is_empty()
    }

    /// Every record's id equals the id derived from its body (section 3.5).
    /// Failure is `internal`.
    pub fn verify_ids(&self) -> Result<(), Error> {
        let mismatch = |id: &str, derived: &str| {
            Error::internal(format!(
                "record {id} does not carry its derived id {derived}"
            ))
        };
        for c in &self.claims {
            let d = c.derive_id()?;
            if d != c.id {
                return Err(mismatch(c.id.as_str(), d.as_str()));
            }
        }
        for m in &self.marks {
            let d = m.derive_id()?;
            if d != m.id {
                return Err(mismatch(m.id.as_str(), d.as_str()));
            }
        }
        for r in &self.retirements {
            let d = r.derive_id()?;
            if d != r.id {
                return Err(mismatch(r.id.as_str(), d.as_str()));
            }
        }
        Ok(())
    }

    /// Every mark and retirement names a claim that is in `stored` or in
    /// this batch. Failure is `internal`.
    pub fn verify_claims<'a>(
        &'a self,
        stored: impl IntoIterator<Item = &'a ClaimId>,
    ) -> Result<(), Error> {
        let known: BTreeSet<&str> = stored
            .into_iter()
            .map(ClaimId::as_str)
            .chain(self.claims.iter().map(|c| c.id.as_str()))
            .collect();
        for m in &self.marks {
            if !known.contains(m.claim.as_str()) {
                return Err(Error::internal(format!(
                    "mark {} names claim {}, which is neither stored nor in the batch",
                    m.id, m.claim
                )));
            }
        }
        for r in &self.retirements {
            if !known.contains(r.retires.as_str()) {
                return Err(Error::internal(format!(
                    "retirement {} names claim {}, which is neither stored nor in the batch",
                    r.id, r.retires
                )));
            }
        }
        Ok(())
    }
}
