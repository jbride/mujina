//! BM13xx HashThread implementation.
//!
//! This module re-exports from `crate::asic::bm13xx::thread` for backwards
//! compatibility. New code should import from `asic::bm13xx::thread` directly.

pub use crate::asic::bm13xx::thread::BM13xxThread;
