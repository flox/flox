use std::rc::Rc;

use tempfile::TempDir;

use crate::providers::git::GitProvider;

#[derive(Debug)]
pub struct ReadOnly<Git: GitProvider> {
    git: Rc<Git>,
}

impl<Git: GitProvider> ReadOnly<Git> {
    pub fn new(git: Git) -> Self {
        Self { git: Rc::new(git) }
    }

    pub fn to_sandbox_in(self, tempdir: TempDir, git: Git) -> GitSandBox<Git> {
        GitSandBox {
            original: self.git,
            sandboxed: git,
            _tempdir: tempdir,
        }
    }
}

#[derive(Debug)]
pub struct GitSandBox<Git: GitProvider> {
    sandboxed: Git,
    original: Rc<Git>,
    _tempdir: TempDir,
}

impl<Git: GitProvider> GitSandBox<Git> {
    /// cleans up sandbox
    ///
    /// since we use TempDir, the tempdir will be removed as it gos out of scope
    pub fn abort(self) -> ReadOnly<Git> {
        ReadOnly { git: self.original }
    }
}

pub trait GitAccess<Git: GitProvider> {
    fn git(&self) -> &Git;
    fn read_only(&self) -> ReadOnly<Git>;
}

impl<Git: GitProvider> GitAccess<Git> for ReadOnly<Git> {
    fn git(&self) -> &Git {
        &self.git
    }

    fn read_only(&self) -> ReadOnly<Git> {
        ReadOnly {
            git: self.git.to_owned(),
        }
    }
}

impl<Git: GitProvider> GitAccess<Git> for GitSandBox<Git> {
    fn git(&self) -> &Git {
        &self.sandboxed
    }

    fn read_only(&self) -> ReadOnly<Git> {
        ReadOnly {
            git: self.original.clone(),
        }
    }
}
