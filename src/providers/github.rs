
use octocrab::Octocrab;
use anyhow::{Result, anyhow};

#[tokio::test]
async fn test_github() -> Result<()> {

    dotenv::dotenv().ok();
    let token = std::env::var(crate::environment::GITHUB_TOKEN).expect("GITHUB_TOKEN env variable is required. Add it to the .env file.");

    let octocrab = Octocrab::builder().personal_token(token.to_string()).build()?;
    
    let my_repos = octocrab
        .current()
        .list_repos_for_authenticated_user()
        .type_("owner")
        .sort("updated")
        .per_page(100)
        .send()
        .await?;

    for repo in my_repos {
        println!("{}", repo.name);
    }

    return Ok(())
}