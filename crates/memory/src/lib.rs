pub mod error;
pub mod indexer;
pub mod retriever;
pub mod store;
pub mod vectordb;

pub use error::MemoryError;
pub use indexer::Indexer;
pub use retriever::Retriever;
pub use store::{IndexTask, MemoryStore, ScopedMemoryStore};
pub use vectordb::{SearchResult, VectorDB};

/// Attach a background vector-indexing task to `store`.
///
/// After this call every [`MemoryStore::write`] will enqueue the written content
/// for embedding and insertion into the vector store backing `indexer`.  The
/// background task runs until the last clone of `store` is dropped (which closes
/// the channel and causes the task to exit cleanly).
///
/// Returns the [`tokio::task::JoinHandle`] for the background task.  Callers may
/// drop the handle — the task will keep running as long as the store is alive.
/// Capacity of the background-indexer task queue.
/// Writes that would exceed this limit are dropped (with a debug log) rather
/// than growing the queue without bound and risking OOM during fast write bursts.
const INDEXER_CHANNEL_CAPACITY: usize = 1_024;

pub fn start_background_indexer(
    store: &std::sync::Arc<MemoryStore>,
    indexer: std::sync::Arc<Indexer>,
) -> tokio::task::JoinHandle<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<IndexTask>(INDEXER_CHANNEL_CAPACITY);
    if !store.set_index_channel(tx) {
        // Channel already set on this store — don't spawn a second consumer or
        // the existing receiver would starve and never see half the messages.
        tracing::warn!(
            "start_background_indexer: index channel already set on this MemoryStore; \
             skipping duplicate indexer spawn"
        );
        return tokio::spawn(async {});
    }
    tokio::spawn(async move {
        while let Some(task) = rx.recv().await {
            if let Err(e) = indexer.index_text(&task.doc_id, &task.content).await {
                tracing::warn!(
                    doc_id = %task.doc_id,
                    error = %e,
                    "background memory index failed"
                );
            }
        }
        tracing::debug!("background memory indexer exiting — channel closed");
    })
}
