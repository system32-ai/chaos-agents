use sqlx::any::AnyPool;

use crate::config::DbTargetConfig;

pub async fn create_pool(config: &DbTargetConfig) -> anyhow::Result<AnyPool> {
    sqlx::any::install_default_drivers();
    let pool = AnyPool::connect(&config.connection_url).await?;
    Ok(pool)
}
