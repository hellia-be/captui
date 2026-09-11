//! captui core: pure helpers (source args, recorder argv, output naming) kept
//! IO-free so they are unit-testable in CI, which has no Wayland session and no
//! PipeWire.

pub mod recorder;
pub mod sources;
