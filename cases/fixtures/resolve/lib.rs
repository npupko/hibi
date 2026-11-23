// Retry policy.
pub const MAX: u32 = 5;

/// Doc for retry.
pub fn retry(n: u32) -> u32 {
    let limit = MAX; // cap here
    n.min(limit)
}

pub fn twice() {
    // step(1); first
    step(1);
    step(1);
}

pub struct Cache;

impl Cache {
    pub fn get(&self) -> u32 { 1 }
    pub fn put(&self) -> u32 { 1 }
}
