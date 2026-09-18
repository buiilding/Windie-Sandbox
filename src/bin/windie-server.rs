//! Entrypoint for the authenticated PostgreSQL-backed Windie hosted server.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let config = windie::hosted::HostedConfig::from_environment()?;
    let store = windie::hosted::HostedStore::connect(&config.database_url).await?;
    store.migrate().await?;
    windie::hosted::serve(config, store).await
}
