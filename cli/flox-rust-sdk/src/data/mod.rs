mod canonical_path;
mod version;

pub use canonical_path::{CanonicalPath, CanonicalizeError};
pub use version::Version;
pub type System = String;
