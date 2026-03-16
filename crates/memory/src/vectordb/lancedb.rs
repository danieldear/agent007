use async_trait::async_trait;
use std::sync::Arc;
use arrow_array::{
    FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray,
    Float32Array, Array,
};
use arrow_schema::{DataType, Field, Schema};
use lancedb::{connect, Table};
use lancedb::query::{ExecutableQuery, QueryBase};
use crate::error::MemoryError;
use crate::vectordb::{SearchResult, VectorDB};

pub struct LanceDBStore {
    table: Arc<tokio::sync::RwLock<Table>>,
    dims: usize,
}

impl LanceDBStore {
    pub async fn new(db_path: &str, table_name: &str, dims: usize) -> Result<Self, MemoryError> {
        let conn = connect(db_path).execute().await
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dims as i32,
                ),
                false,
            ),
            Field::new("payload", DataType::Utf8, false),
        ]));

        // Try to open existing table, create if not found
        let table = match conn.open_table(table_name).execute().await {
            Ok(t) => t,
            Err(_) => {
                // Create empty table with schema
                conn.create_empty_table(table_name, schema)
                    .execute()
                    .await
                    .map_err(|e| MemoryError::VectorDb(e.to_string()))?
            }
        };

        Ok(Self {
            table: Arc::new(tokio::sync::RwLock::new(table)),
            dims,
        })
    }
}

#[async_trait]
impl VectorDB for LanceDBStore {
    async fn upsert(&self, id: &str, vector: Vec<f32>, payload: serde_json::Value)
        -> Result<(), MemoryError>
    {
        let table = self.table.write().await;

        // Delete existing row with matching id
        let _ = table.delete(&format!("id = '{}'", id)).await;

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.dims as i32,
                ),
                false,
            ),
            Field::new("payload", DataType::Utf8, false),
        ]));

        let id_array = Arc::new(StringArray::from(vec![id]));
        let float_values = Arc::new(Float32Array::from(vector));
        let vector_array = Arc::new(
            FixedSizeListArray::new(
                Arc::new(Field::new("item", DataType::Float32, true)),
                self.dims as i32,
                float_values,
                None,
            )
        );
        let payload_str = serde_json::to_string(&payload).map_err(MemoryError::Json)?;
        let payload_array = Arc::new(StringArray::from(vec![payload_str.as_str()]));

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![id_array, vector_array, payload_array],
        ).map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        let reader = RecordBatchIterator::new(
            vec![Ok(batch)].into_iter(),
            schema,
        );
        table.add(reader).execute().await
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        Ok(())
    }

    async fn search(&self, query: Vec<f32>, limit: usize)
        -> Result<Vec<SearchResult>, MemoryError>
    {
        let table = self.table.read().await;

        let results = table
            .vector_search(query).map_err(|e| MemoryError::VectorDb(e.to_string()))?
            .limit(limit)
            .execute()
            .await
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        use futures::TryStreamExt;
        let batches: Vec<RecordBatch> = results.try_collect().await
            .map_err(|e| MemoryError::VectorDb(e.to_string()))?;

        let mut search_results = Vec::new();
        for batch in &batches {
            let id_col = batch.column_by_name("id")
                .ok_or_else(|| MemoryError::VectorDb("missing 'id' column".to_string()))?;
            let payload_col = batch.column_by_name("payload")
                .ok_or_else(|| MemoryError::VectorDb("missing 'payload' column".to_string()))?;
            let distance_col = batch.column_by_name("_distance")
                .ok_or_else(|| MemoryError::VectorDb("missing '_distance' column".to_string()))?;

            let ids = id_col.as_any().downcast_ref::<StringArray>()
                .ok_or_else(|| MemoryError::VectorDb("id column wrong type".to_string()))?;
            let payloads = payload_col.as_any().downcast_ref::<StringArray>()
                .ok_or_else(|| MemoryError::VectorDb("payload column wrong type".to_string()))?;
            let distances = distance_col.as_any().downcast_ref::<Float32Array>()
                .ok_or_else(|| MemoryError::VectorDb("_distance column wrong type".to_string()))?;

            for i in 0..batch.num_rows() {
                let id = ids.value(i).to_string();
                let payload: serde_json::Value = serde_json::from_str(payloads.value(i))
                    .map_err(MemoryError::Json)?;
                let distance = distances.value(i);
                let score = 1.0 / (1.0 + distance);
                search_results.push(SearchResult { id, score, payload });
            }
        }

        Ok(search_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectordb::VectorDB;
    use tempfile::TempDir;

    #[tokio::test]
    async fn lancedb_upsert_and_search() {
        let dir = TempDir::new().unwrap();
        let store = LanceDBStore::new(
            dir.path().to_str().unwrap(),
            "test",
            4,
        ).await.unwrap();

        store.upsert("a", vec![1.0, 0.0, 0.0, 0.0], serde_json::json!({"doc": "alpha"})).await.unwrap();
        store.upsert("b", vec![0.0, 1.0, 0.0, 0.0], serde_json::json!({"doc": "beta"})).await.unwrap();

        let results = store.search(vec![1.0, 0.0, 0.0, 0.0], 1).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "a");
    }
}
