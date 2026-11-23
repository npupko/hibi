// Retry settings.
pub const MAX_TRIES: u32 = 5;

pub struct RetryOptions {
    pub tries: u32,
}

pub fn retry(max_tries: u32, mode: &str) -> u32 {
    // try a few times
    let label = "strict";
    max_tries * 2
}
