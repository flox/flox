use std::rc::Rc;

use tempfile::TempDir;

use crate::providers::git::GitCommandProvider as Git;

#[derive(Debug)]
pub struct ReadOnly {
    git: Rc<Git>,
}

impl ReadOnly {
    pub fn new(git: Git) -> Self {
        Self { git: Rc::new(git) }
    }

    pub fn to_sandbox_in(self, tempdir: TempDir, git: Git) -> GitSandBox {
        GitSandBox {
            original: self.git,
            sandboxed: git,
            _tempdir: tempdir,
        }
    }
}

#[derive(Debug)]
pub struct GitSandBox {
    sandboxed: Git,
    original: Rc<Git>,
    _tempdir: TempDir,
}

impl GitSandBox {
    /// cleans up sandbox
    ///
    /// since we use TempDir, the tempdir will be removed as it gos out of scope
    pub fn abort(self) -> ReadOnly {
        ReadOnly { git: self.original }
    }
}

pub trait GitAccess {
    fn git(&self) -> &Git;
    fn read_only(&self) -> ReadOnly;
}

impl GitAccess for ReadOnly {
    fn git(&self) -> &Git {
        &self.git
    }

    fn read_only(&self) -> ReadOnly {
        ReadOnly {
            git: self.git.to_owned(),
        }
    }
}

impl GitAccess for GitSandBox {
    fn git(&self) -> &Git {
        &self.sandboxed
    }

    fn read_only(&self) -> ReadOnly {
        ReadOnly {
            git: self.original.clone(),
        }
    }
}
