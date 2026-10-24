use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub type Db = PgPool;

/// Connections to hold open. A single-user tool serves one person's browsing plus one refresh loop, so the pool exists to bound concurrency rather than to provide it. Each PostgreSQL backend costs several megabytes of server memory, which is the reason this is small and configurable: on a 1 GiB host the difference between 4 and 8 backends is real.
const DEFAULT_MAX_CONNECTIONS: u32 = 4;

fn max_connections() -> u32 {
    std::env::var("DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_CONNECTIONS)
}

/// Connect with a bounded pool. A small pool surfaces a leaked connection immediately instead of hiding it.
pub async fn connect(database_url: &str) -> Result<Db, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections())
        .acquire_timeout(Duration::from_secs(10))
        // An idle connection still occupies a backend on the server, so the pool gives them back rather than holding every one it ever opened.
        .idle_timeout(Duration::from_secs(300))
        .connect(database_url)
        .await
}

/// Apply every pending migration. SQLx runs each migration in a transaction and records it, so a repeat call on an up-to-date database is a no-op.
pub async fn migrate(pool: &Db) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_size_is_configurable_and_never_zero() {
        // Serialized through one test because the setting is process-wide.
        std::env::set_var("DATABASE_MAX_CONNECTIONS", "3");
        assert_eq!(max_connections(), 3);
        std::env::set_var("DATABASE_MAX_CONNECTIONS", "0");
        assert_eq!(
            max_connections(),
            DEFAULT_MAX_CONNECTIONS,
            "zero is not a pool"
        );
        std::env::set_var("DATABASE_MAX_CONNECTIONS", "not a number");
        assert_eq!(max_connections(), DEFAULT_MAX_CONNECTIONS);
        std::env::remove_var("DATABASE_MAX_CONNECTIONS");
        assert_eq!(max_connections(), DEFAULT_MAX_CONNECTIONS);
    }
}
