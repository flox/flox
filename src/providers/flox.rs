struct FloxPackageProvider {
    store_directory: String
}


use crate::models::*;
use async_trait::async_trait;
use anyhow::{anyhow, Result, Ok};
use tokio::process::Command;


/// A package provider that uses flox to maange Packages / Environments.
/// 

impl FloxPackageProvider {
    fn default() -> Self {
        FloxPackageProvider { store_directory: "/nix/store".to_string() }
    }
}
#[async_trait]
impl PackageProvider for FloxPackageProvider {
    async fn create(&self, package_name: &str) -> Result<CreateResult> {
    
        let mut process = Command::new("flox")
            .arg("create")
            .arg(package_name)
            .output();
        
        // {
        //     Ok(_) => Ok(CreateResult::new("Package created")),
        //     Err(err) => Err(anyhow!("Error thrown trying to create a message: {}", err)),
        // }
        let output = process.await?;

        Ok(CreateResult::new(std::str::from_utf8(&output.stdout)?))
    }

    async fn install(&self) -> Result<InstallResult> {
        Ok(InstallResult{})
    }

    async fn search(&self, query: &str) -> Result<SearchResult> {
        Ok(SearchResult::new(Vec::new()))
    }

    async fn shell(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use crate::models::PackageProvider;

    use super::FloxPackageProvider;

    #[tokio::test]
    async fn test_create() -> Result<()> {
        let pp = FloxPackageProvider::default();

        pp.create("test-package-1").await?;

        Ok(())
    }
}