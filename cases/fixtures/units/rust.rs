//! Crate docs are comments, not units.
#![allow(dead_code)]

use std::fmt::{self, Display};

/// A cache.
#[derive(Debug)]
pub struct Cache {
    size: usize,
}

pub struct Wrap<T>(T);
union Bits { i: u32, f: f32 }
enum Mode { Fast, Slow }

pub trait Store {
    type Item;
    const LIMIT: usize;
    fn get(&self) -> usize;
    fn put(&mut self) {}
}

impl Cache {
    pub fn new() -> Self {
        Cache { size: 0 }
    }
}

impl Cache {
    const EMPTY: usize = 0;
}

impl Display for Cache {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "caché")
    }
}

impl<T> Wrap<T> {
    fn inner(self) -> T {
        self.0
    }
}

mod util {
    pub fn helper() -> u8 { 1 }
    pub static NAME: &str = "util";
}

mod external;

type Alias = Vec<u8>;
const MAX: u32 = 10;

macro_rules! square {
    ($x:expr) => { $x * $x };
}

fn main() {}
