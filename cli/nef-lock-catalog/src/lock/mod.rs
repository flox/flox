//! Locking engine — imperative shell (network + I/O).
//!
//! Houses the lockfile model ([build_lock]), source handling
//! ([flakeref]), and the attr-path tree builder ([tree]), moved out of
//! the crate root to establish the functional-core (`scan/`) vs
//! imperative-shell (`lock/`) split. The lookup + transform engine
//! builds into this module in ECO-93.

pub(crate) mod build_lock;
pub(crate) mod flakeref;
pub(crate) mod tree;
