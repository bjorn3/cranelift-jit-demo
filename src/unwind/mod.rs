//! Handling of everything related to debuginfo.

mod emit;
mod unwind;

pub(crate) use emit::DebugRelocName;
pub(crate) use unwind::UnwindContext;
