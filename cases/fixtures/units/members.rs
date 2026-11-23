#[derive(Debug)]
pub struct Point {
    pub x: i32,
    #[serde(rename = "why")]
    y: i32,
}

pub struct Pair(u8, u8);

union Bits { i: u32, f: f32 }

#[repr(u8)]
enum Mode {
    Fast,
    #[default]
    Slow = 2,
    Custom { level: u8 },
}

extern "C" {
    fn abs(x: i32) -> i32;
    static ERRNO: i32;
}

extern "C" {
    fn labs(x: i64) -> i64;
}

trait Store {
    const LIMIT: usize;
    type Item;
    fn get(&self) -> Self::Item;
}

impl dyn Store {
    const EMPTY: u8 = 0;
}

impl<'a> Store for &'a Point<u8> {
    type Item = u8;
    fn get(&self) -> u8 { 0 }
}

#[cfg(test)]
use std::fmt;
