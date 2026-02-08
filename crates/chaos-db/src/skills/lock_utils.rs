use chaos_core::error::{ChaosError, ChaosResult};
use sqlx::any::Any;
use sqlx::pool::PoolConnection;
use sqlx::{AnyPool, Row};

use crate::config::DbType;

const VALID_TABLE_LOCK_MODES: &[&str] = &[
    "ACCESS SHARE",
    "ROW SHARE",
    "ROW EXCLUSIVE",
    "SHARE UPDATE EXCLUSIVE",
    "SHARE",
    "SHARE ROW EXCLUSIVE",
    "EXCLUSIVE",
    "ACCESS EXCLUSIVE",
];

const VALID_ROW_LOCK_TYPES: &[&str] = &[
    "FOR UPDATE",
    "FOR NO KEY UPDATE",
    "FOR SHARE",
    "FOR KEY SHARE",
];

pub fn validate_lock_mode(mode: &str) -> ChaosResult<()> {
    let upper = mode.to_uppercase();
    if !VALID_TABLE_LOCK_MODES.contains(&upper.as_str()) {
        return Err(ChaosError::Config(format!(
            "Invalid lock mode '{}'. Valid modes: {:?}",
            mode, VALID_TABLE_LOCK_MODES
        )));
    }
    Ok(())
}

pub fn validate_row_lock_type(lock_type: &str) -> ChaosResult<()> {
    let upper = lock_type.to_uppercase();
    if !VALID_ROW_LOCK_TYPES.contains(&upper.as_str()) {
        return Err(ChaosError::Config(format!(
            "Invalid row lock type '{}'. Valid types: {:?}",
            lock_type, VALID_ROW_LOCK_TYPES
        )));
    }
    Ok(())
}

pub async fn discover_user_tables(pool: &AnyPool) -> ChaosResult<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT table_schema, table_name FROM information_schema.tables \
         WHERE table_schema NOT IN ('information_schema', 'pg_catalog', 'mysql', 'performance_schema', 'sys', 'crdb_internal') \
         AND table_type = 'BASE TABLE' LIMIT 5",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ChaosError::Discovery(format!("Failed to list tables: {e}")))?;

    Ok(rows
        .iter()
        .map(|r| {
            let schema: String = r.get("table_schema");
            let table: String = r.get("table_name");
            (schema, table)
        })
        .collect())
}

pub async fn get_backend_pid(
    conn: &mut PoolConnection<Any>,
    db_type: DbType,
) -> ChaosResult<i32> {
    match db_type {
        DbType::Postgres | DbType::CockroachDb | DbType::YugabyteDb => {
            let row = sqlx::query("SELECT pg_backend_pid()")
                .fetch_one(&mut **conn)
                .await
                .map_err(|e| {
                    ChaosError::Other(anyhow::anyhow!("Failed to get backend PID: {e}"))
                })?;
            Ok(row.get::<i32, _>(0))
        }
        DbType::Mysql => {
            let row = sqlx::query("SELECT CONNECTION_ID()")
                .fetch_one(&mut **conn)
                .await
                .map_err(|e| {
                    ChaosError::Other(anyhow::anyhow!("Failed to get connection ID: {e}"))
                })?;
            let id: i64 = row.get(0);
            Ok(id as i32)
        }
        DbType::MongoDB => Err(ChaosError::Config(
            "Lock skills not supported for MongoDB".into(),
        )),
    }
}

pub async fn terminate_backend(pool: &AnyPool, pid: i32, db_type_str: &str) -> ChaosResult<()> {
    let db_lower = db_type_str.to_lowercase();

    if db_lower.contains("mysql") {
        let kill_query = format!("KILL {}", pid);
        sqlx::query(&kill_query).execute(pool).await.map_err(|e| {
            ChaosError::Other(anyhow::anyhow!(
                "Failed to KILL MySQL connection {}: {e}",
                pid
            ))
        })?;
    } else {
        // PostgreSQL, CockroachDB, YugabyteDB all support pg_terminate_backend
        let result = sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .fetch_one(pool)
            .await
            .map_err(|e| {
                ChaosError::Other(anyhow::anyhow!(
                    "Failed to terminate backend PID {}: {e}",
                    pid
                ))
            })?;

        let terminated: bool = result.try_get::<bool, _>(0).unwrap_or(false);
        if !terminated {
            tracing::warn!(
                pid,
                "pg_terminate_backend returned false -- backend may already be gone"
            );
        }
    }

    Ok(())
}

pub async fn find_pk_column(
    conn: &mut PoolConnection<Any>,
    schema: &str,
    table: &str,
) -> Option<String> {
    let pk_row = sqlx::query(
        "SELECT c.column_name FROM information_schema.columns c \
         JOIN information_schema.key_column_usage kcu \
           ON c.table_schema = kcu.table_schema AND c.table_name = kcu.table_name AND c.column_name = kcu.column_name \
         JOIN information_schema.table_constraints tc \
           ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema \
         WHERE tc.constraint_type = 'PRIMARY KEY' AND c.table_schema = $1 AND c.table_name = $2 \
         LIMIT 1",
    )
    .bind(schema)
    .bind(table)
    .fetch_optional(&mut **conn)
    .await
    .ok()
    .flatten();

    pk_row.map(|row| row.get("column_name"))
}
