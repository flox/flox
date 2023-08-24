use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use anyhow::anyhow;
use flox_types::version::Version;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use thiserror::Error;

use super::{FetchError, Floxmeta, TransactionCommitError, TransactionEnterError};
use crate::models::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use crate::providers::git::GitProvider;

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

impl<'flox, Git: GitProvider, A: GitAccess<Git>> Floxmeta<'flox, Git, A> {
    /// load and parse `floxUserMeta.json` file from floxmeta repo
    ///
    /// note: fetches updates from upstream (todo: is this a ui decision?)
    pub async fn user_meta(&self) -> Result<UserMeta, GetUserMetaError<Git>> {
        let user_meta_str = self
            .git()
            .show(&format!("{FLOX_MAIN_BRANCH}:{FLOX_USER_META_FILE}"))
            .await
            .map_err(GetUserMetaError::Show)?;
        let user_meta = serde_json::from_str(&user_meta_str.to_string_lossy())?;
        Ok(user_meta)
    }
}

impl<'flox, Git: GitProvider> Floxmeta<'flox, Git, ReadOnly<Git>> {
    pub async fn set_user_meta(
        &self,
        user_meta: &UserMeta,
        message: &str,
    ) -> Result<(), TransactionError<Git, SetUserMetaError<Git>>> {
        let floxmeta_sandbox = self
            .clone()
            .enter_transaction()
            .await
            .map_err(TransactionError::Enter)?;

        floxmeta_sandbox
            .git()
            .checkout(FLOX_MAIN_BRANCH, false)
            .await
            .map_err(|e| TransactionError::Setup(anyhow!(e.to_string())))?;

        floxmeta_sandbox.set_user_meta(user_meta).await?;

        floxmeta_sandbox
            .commit_transaction(message)
            .await
            .map_err(TransactionError::Commit)?;

        Ok(())
    }
}

impl<'flox, Git: GitProvider> Floxmeta<'flox, Git, GitSandBox<Git>> {
    /// write `floxUserMeta.json` file to floxmeta repo
    ///
    /// This is in a sandbox, where checkouts and adding files is allowed.
    /// It is assumed the correct branch is checked out before this function is called.
    pub async fn set_user_meta(&self, user_meta: &UserMeta) -> Result<(), SetUserMetaError<Git>> {
        let mut file = File::create(self.git().workdir().unwrap().join(FLOX_USER_META_FILE))
            .map_err(SetUserMetaError::OpenUserMetaFile)?;

        serde_json::to_writer_pretty(&mut file, user_meta)?;

        self.git()
            .add(&[Path::new(FLOX_USER_META_FILE)])
            .await
            .map_err(SetUserMetaError::Add)?;

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum TransactionError<Git: GitProvider, Inner> {
    #[error(transparent)]
    Enter(TransactionEnterError<Git>),
    #[error(transparent)]
    Inner(#[from] Inner),
    #[error(transparent)]
    Setup(anyhow::Error),
    #[error(transparent)]
    Commit(TransactionCommitError<Git>),
}

#[derive(Error, Debug)]
pub enum GetUserMetaError<Git: GitProvider> {
    #[error(transparent)]
    Fetch(#[from] FetchError<Git>),
    #[error("Could not access 'userFloxMeta.json': {0}")]
    Show(Git::ShowError),
    #[error("Could not parse 'userFloxMeta.json': {0}")]
    Deserialize(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum SetUserMetaError<Git: GitProvider> {
    #[error(transparent)]
    Fetch(#[from] FetchError<Git>),
    #[error("Could not checkout '{FLOX_MAIN_BRANCH}' branch: {0}")]
    Checkout(Git::CheckoutError),
    #[error("Could not open or create '{FLOX_USER_META_FILE}' file: {0}")]
    OpenUserMetaFile(std::io::Error),
    #[error("Could not serialize 'userFloxMeta.json': {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("Could not add '{FLOX_USER_META_FILE}': {0}")]
    Add(Git::AddError),
}

#[cfg(feature = "impure-unit-tests")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::floxmeta::floxmeta_tests::flox_instance;
    use crate::models::floxmeta::FLOXMETA_DIR_NAME;
    use crate::models::root::transaction::ReadOnly;
    use crate::providers::git::GitCommandProvider;

    #[tokio::test]
    async fn user_meta() {
        let (flox, _tempdir_handle) = flox_instance();

        let meta_repo = flox.cache_dir.join(FLOXMETA_DIR_NAME).join("flox");
        tokio::fs::create_dir_all(&meta_repo).await.unwrap();

        let _git = <GitCommandProvider as GitProvider>::clone(
            "https://github.com/flox/floxmeta",
            &meta_repo,
            true,
        )
        .await
        .unwrap();

        let floxmeta = Floxmeta::<GitCommandProvider, ReadOnly<_>>::get_floxmeta(&flox, "flox")
            .await
            .expect("Should open floxmeta repo");

        let user_meta = floxmeta
            .user_meta()
            .await
            .expect("Should find floxUserMeta");

        let floxmeta = floxmeta
            .enter_transaction()
            .await
            .expect("Should enter transaction");
        floxmeta.git().checkout("floxmain", false).await.unwrap();
        floxmeta
            .set_user_meta(&UserMeta {
                channels: Some([].into()),
                ..user_meta
            })
            .await
            .expect("Should set usermeta");
        let floxmeta = floxmeta
            .commit_transaction("Write user meta")
            .await
            .expect("Should commit transaction");

        let user_meta = floxmeta
            .user_meta()
            .await
            .expect("Should find floxUserMeta");

        assert!(user_meta.channels.unwrap().is_empty());
    }
}
