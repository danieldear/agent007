pub mod budget;
pub mod compact;
pub mod context;
pub mod error;
pub mod hash;
pub mod paths;
pub mod repo_brain;
pub mod repo_filter;
pub mod repo_graph;
pub mod repo_index;
pub mod repo_readiness;
pub mod tool_executor;
pub mod tree_sitter_support;
pub mod types;
pub use tool_executor::ToolExecutor;
pub mod agent;
pub mod dispatcher;
pub mod events;
pub mod orchestrator;
pub mod persona;
pub mod run_store;
pub mod task;
pub mod worker;

pub use budget::{estimate_tokens, BudgetEstimate, CompactLevel, TokenBudget};
pub use compact::{compact_command_output, CompactOutput};
pub use context::{
    ContextBundle, ContextCompiler, ContextFile, ContextMemoryNote, ContextPromptManifest,
    ContextPromptSection,
};
pub use dispatcher::{Dispatcher, LocalDispatcher};
pub use error::CoreError;
pub use events::AgentEvent;
pub use hash::stable_context_hash;
pub use persona::{NoOpPersonaProvider, PersonaProvider, PersonaSpec};
pub use repo_brain::{RepoBrain, RepoBrainBuilder};
pub use repo_graph::{
    build_and_save_graph, build_and_save_index, callees_for_symbol, callers_for_symbol,
    context_bundle_for_query, default_graph_path_for_root, dep_path_between_symbols,
    dirty_paths_path_for_root, doc_links_for_symbol, evidence_refs_for_text,
    freshen_graph_if_needed, graph_stale_paths, graph_status, impact_radius_for_symbol,
    is_repo_graph_trackable_path, load_graph, load_or_build_graph, load_repo_graph_dirty_paths,
    mark_repo_graph_dirty_paths, refresh_graph_for_paths, repo_graph_trackable_files,
    resolve_graph_path, symbol_lookup, usage_graph_for_symbol, RepoGraph, RepoGraphBuilder,
    RepoGraphCounts, RepoGraphDirtySet, RepoGraphEdge, RepoGraphEdgeKind, RepoGraphFreshenReport,
    RepoGraphFreshnessState, RepoGraphNeighborhood, RepoGraphNode, RepoGraphNodeKind,
    RepoGraphPathResult, RepoGraphPathStep, RepoGraphQueryContext, RepoGraphStatus,
};
pub use repo_index::{
    build_and_save_index_for_graph, context_bundle_for_query_index, default_index_path_for_root,
    evidence_refs_for_text_index, freshen_index_if_needed, index_is_stale,
    index_path_for_graph_path, index_status, load_or_build_index, open_index_for_graph_path,
    save_index, RepoIndex, RepoIndexFreshenReport, RepoIndexStatus,
};

pub use repo_readiness::{
    detect_repo_intelligence_readiness, ensure_repo_graph_ready_for_task,
    ensure_repo_graph_ready_for_trigger, load_repo_intelligence_readiness, readiness_path_for_root,
    task_requests_repo_graph, write_repo_intelligence_readiness, InstallRecommendation,
    LanguageKind, LanguageReadiness, LspServerReadiness, RepoGraphEnsureAction,
    RepoGraphEnsureResult, RepoIntelligenceOptions, RepoIntelligenceReadiness,
    RepoIntelligenceState, TreeSitterReadiness,
};
pub use run_store::{
    AgentMessage, AgentMessageKind, RunCostMode, RunDetail, RunLogEntry, RunMetadata, RunScorecard,
    RunStatus, RunStore, RunTokenSummary, TOKEN_PRICE_PER_TOKEN_USD,
};
pub use task::{Task, TaskQueue, TaskResult};
pub use tree_sitter_support::{
    language_is_supported as tree_sitter_language_is_supported, TreeSitterSupportSummary,
};
pub use types::{AgentId, MemoryRef, PromptRef, PromptStore};
