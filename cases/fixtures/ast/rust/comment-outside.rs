// Retry policy, reviewed.
pub const MAX_ATTEMPTS: u32 = 5;

pub struct RetryOptions {
    pub attempts: u32,
}

/// Retries with backoff.
pub fn retry(max_attempts: u32, mode: &str) -> u32 {
    // try a few times
    let label = "strict";
    max_attempts * 2
}
