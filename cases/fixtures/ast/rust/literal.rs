// Retry settings.
pub const MAX_ATTEMPTS: u32 = 50;

pub struct RetryOptions {
    pub attempts: u32,
}

pub fn retry(max_attempts: u32, mode: &str) -> u32 {
    // try a few times
    let label = "lenient";
    max_attempts * 2
}
