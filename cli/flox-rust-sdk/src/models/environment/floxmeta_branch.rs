use thiserror::Error;

use crate::models::floxmeta::FloxMeta;

#[derive(Debug)]
pub struct FloxmetaBranch {
    floxmeta: FloxMeta,
    branch: String,
}

#[derive(Debug, Error)]
pub enum FloxmetaBranchError {}

#[cfg(test)]
mod tests {
    use super::*;
}
