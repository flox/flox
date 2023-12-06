use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use flox_types::version::Version;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use thiserror::Error;

use super::{FetchError, Floxmeta, TransactionCommitError, TransactionEnterError};
use crate::models::root::transaction::{GitAccess, GitSandBox};
use crate::providers::git::{GitCommandError, GitProvider};

pub(super) const FLOX_MAIN_BRANCH: &str = "floxmain";
pub(super) const FLOX_USER_META_FILE: &str = "floxUserMeta.json";

#[serde_as]
#[derive(Deserialize, Serialize)]
pub struct UserMeta {
    /// User provided channels
    /// TODO: transition to runix flakeRefs
    #[serde_as(as = "Option<BTreeMap<_, DisplayFromStr>>")]
    pub channels: Option<BTreeMap<String, String>>,
    #[serde(rename = "floxClientUUID")]
    pub client_uuid: uuid::Uuid,
    #[serde(rename = "floxMetricsConsent")]
    pub metrics_consent: Option<u8>,
    pub version: Version<1>,
}

impl<'flox, A: GitAccess> Floxmeta<'flox, A> {
    /// load and parse `floxUserMeta.json` file from floxmeta repo
    ///
    /// note: fetches updates from upstream (todo: is this a ui decision?)
    pub fn user_meta(&self) -> Result<UserMeta, GetUserMetaError> {
        let user_meta_str = self
            .git()
            .show(&format!("{FLOX_MAIN_BRANCH}:{FLOX_USER_META_FILE}"))
            .map_err(GetUserMetaError::Show)?;
        let user_meta = serde_json::from_str(&user_meta_str.to_string_lossy())?;
        Ok(user_meta)
    }
}

impl<'flox> Floxmeta<'flox, GitSandBox> {
    /// write `floxUserMeta.json` file to floxmeta repo
    ///
    /// This is in a sandbox, where checkouts and adding files is allowed.
    /// It is assumed the correct branch is checked out before this function is called.
    pub fn set_user_meta(&self, user_meta: &UserMeta) -> Result<(), SetUserMetaError> {
        let mut file = File::create(self.git().workdir().unwrap().join(FLOX_USER_META_FILE))
            .map_err(SetUserMetaError::OpenUserMetaFile)?;

        serde_json::to_writer_pretty(&mut file, user_meta)?;

        self.git()
            .add(&[Path::new(FLOX_USER_META_FILE)])
            .map_err(SetUserMetaError::Add)?;

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum TransactionError<Inner> {
    #[error(transparent)]
    Enter(TransactionEnterError),
    #[error(transparent)]
    Inner(#[from] Inner),
    #[error(transparent)]
    Setup(anyhow::Error),
    #[error(transparent)]
    Commit(TransactionCommitError),
}

#[derive(Error, Debug)]
pub enum GetUserMetaError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("Could not access 'userFloxMeta.json': {0}")]
    Show(GitCommandError),
    #[error("Could not parse 'userFloxMeta.json': {0}")]
    Deserialize(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum SetUserMetaError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("Could not checkout '{FLOX_MAIN_BRANCH}' branch: {0}")]
    Checkout(GitCommandError),
    #[error("Could not open or create '{FLOX_USER_META_FILE}' file: {0}")]
    OpenUserMetaFile(std::io::Error),
    #[error("Could not serialize 'userFloxMeta.json': {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("Could not add '{FLOX_USER_META_FILE}': {0}")]
    Add(GitCommandError),
}
