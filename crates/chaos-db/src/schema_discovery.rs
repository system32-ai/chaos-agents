use chaos_core::discovery::{ColumnInfo, DbResource};
use sqlx::any::AnyPool;
use sqlx::Row;

/// Introspect database schema using information_schema (works for both Pg and MySQL).
pub async fn discover_schema(pool: &AnyPool) -> anyhow::Result<Vec<DbResource>> {
    let tables = sqlx::query(
        r#"
        SELECT table_schema, table_name
        FROM information_schema.tables
        WHERE table_schema NOT IN ('information_schema', 'pg_catalog', 'mysql', 'performance_schema', 'sys')
          AND table_type = 'BASE TABLE'
        ORDER BY table_schema, table_name
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut resources = Vec::new();

    for table_row in &tables {
        let schema: String = table_row.get("table_schema");
        let table_name: String = table_row.get("table_name");

        let columns = sqlx::query(
            r#"
            SELECT
                c.column_name,
                c.data_type,
                c.is_nullable,
                CASE WHEN tc.constraint_type = 'PRIMARY KEY' THEN 'YES' ELSE 'NO' END as is_pk
            FROM information_schema.columns c
            LEFT JOIN information_schema.key_column_usage kcu
                ON c.table_schema = kcu.table_schema
                AND c.table_name = kcu.table_name
                AND c.column_name = kcu.column_name
            LEFT JOIN information_schema.table_constraints tc
                ON kcu.constraint_name = tc.constraint_name
                AND kcu.table_schema = tc.table_schema
                AND tc.constraint_type = 'PRIMARY KEY'
            WHERE c.table_schema = $1 AND c.table_name = $2
            ORDER BY c.ordinal_position
            "#,
        )
        .bind(&schema)
        .bind(&table_name)
        .fetch_all(pool)
        .await?;

        let column_infos: Vec<ColumnInfo> = columns
            .iter()
            .map(|col| ColumnInfo {
                name: col.get("column_name"),
                data_type: col.get("data_type"),
                is_nullable: col.get::<String, _>("is_nullable") == "YES",
                is_primary_key: col.get::<String, _>("is_pk") == "YES",
            })
            .collect();

        resources.push(DbResource {
            table_name: table_name.clone(),
            schema: schema.clone(),
            columns: column_infos,
            row_count_estimate: 0,
        });
    }

    Ok(resources)
}
