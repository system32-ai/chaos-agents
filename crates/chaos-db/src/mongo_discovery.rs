use chaos_core::discovery::MongoResource;
use mongodb::Client;

/// Discover MongoDB databases and collections.
pub async fn discover_mongo(
    client: &Client,
    filter_databases: &[String],
) -> anyhow::Result<Vec<MongoResource>> {
    let mut resources = Vec::new();

    let db_names = client.list_database_names().await?;

    for db_name in &db_names {
        // Skip system databases
        if matches!(db_name.as_str(), "admin" | "local" | "config") {
            continue;
        }

        // If filter is set, skip databases not in the list
        if !filter_databases.is_empty() && !filter_databases.contains(db_name) {
            continue;
        }

        let db = client.database(db_name);
        let collection_names = db.list_collection_names().await?;

        for coll_name in &collection_names {
            // Skip system collections
            if coll_name.starts_with("system.") {
                continue;
            }

            let coll = db.collection::<mongodb::bson::Document>(coll_name);
            let doc_count = coll.estimated_document_count().await.unwrap_or(0);

            resources.push(MongoResource {
                database: db_name.clone(),
                collection: coll_name.clone(),
                document_count: doc_count,
            });
        }
    }

    Ok(resources)
}
