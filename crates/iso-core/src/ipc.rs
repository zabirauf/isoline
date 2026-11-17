//! IPC payloads. The UI calls only the commands listed in CONTRACTS.md and receives only
//! these types. TypeScript bindings are generated into `app/ui/src/ipc/bindings` by
//! `cargo test -p iso-core export_bindings`.
//!
//! Field names are camelCase on the wire.

use serde::{Deserialize, Serialize};

use crate::{Confidence, FileId, ImportKind, Language, SymbolId, SymbolKind, Tier};

#[cfg(feature = "ts")]
use ts_rs::TS;

macro_rules! ipc {
    ($($item:item)*) => {
        $(
            #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
            #[cfg_attr(feature = "ts", derive(TS))]
            #[cfg_attr(feature = "ts", ts(export))]
            #[serde(rename_all = "camelCase")]
            $item
        )*
    };
}

ipc! {
    pub struct TierCounts {
        pub full: u32,
        pub outline: u32,
        pub text: u32,
    }

    pub struct LangCount {
        pub lang: Language,
        pub label: String,
        pub files: u32,
        pub lines: u64,
    }

    /// Returned by `open_repo`. Indexing continues in the background.
    pub struct RepoSummary {
        pub root: String,
        pub name: String,
        pub files: u32,
        pub lines: u64,
        pub tiers: TierCounts,
        pub languages: Vec<LangCount>,
        /// Files whose facts are current.
        pub indexed: u32,
        /// Root of the Merkle tree at the last completed index.
        pub root_hash: Option<String>,
        pub ai_enabled: bool,
    }

    /// Event `index_progress`.
    pub struct IndexProgress {
        pub phase: IndexPhase,
        pub done: u32,
        pub total: u32,
        pub current: Option<String>,
        pub elapsed_ms: u64,
    }

    pub struct TreeNode {
        pub name: String,
        /// Repo-relative path with `/` separators; "" for the root.
        pub path: String,
        pub is_dir: bool,
        pub lang: Option<Language>,
        pub tier: Option<Tier>,
        pub module: Option<String>,
        /// CSS color for the module, light theme. The UI adjusts for dark.
        pub color: Option<String>,
        pub status: FileStatus,
        pub lines: u32,
        pub children: Vec<TreeNode>,
    }

    /// Returned by `get_file_view`: everything the reading list and the dependency map need.
    pub struct FileView {
        pub path: String,
        pub name: String,
        pub lang: Language,
        pub lang_label: String,
        pub monaco_lang: String,
        pub tier: Tier,
        pub lines: u32,
        pub content_hash: String,
        /// Number of layers (max level + 1); 0 when there are no functions.
        pub layers: u32,
        pub functions: Vec<FnCard>,
        pub types: Vec<TypeDecl>,
        pub modules: Vec<ImportModule>,
        /// Line range covering the import block, when imports are contiguous at the top.
        pub imports_range: Option<LineRange>,
        /// Outline for tier-2 files, and for symbols that are not cards in tier 1.
        pub outline: Vec<OutlineItem>,
        /// Flow units available in this file (top-level functions, methods, controller methods).
        pub flow_units: Vec<FlowUnitRef>,
        /// Files in the repo that import this one.
        pub importers: Vec<FileRef>,
        /// Model-written overview of the file, if cached.
        pub overview: Option<AiText>,
        pub has_errors: bool,
        pub module: Option<ModuleRef>,
    }

    pub struct LineRange {
        pub start: u32,
        pub end: u32,
    }

    pub struct FileRef {
        pub path: String,
        pub line: Option<u32>,
    }

    pub struct ModuleRef {
        pub id: String,
        pub name: String,
        pub color: String,
    }

    /// One function card.
    pub struct FnCard {
        pub id: SymbolId,
        /// Display name: `run`, or `MyClass.run` for methods.
        pub name: String,
        pub qualified_name: String,
        pub kind: SymbolKind,
        pub start: u32,
        pub end: u32,
        pub exported: bool,
        pub is_async: bool,
        pub signature: String,
        pub doc: Option<String>,
        /// Longest call path to a leaf within this file. Entry points have the highest level.
        pub level: u32,
        /// Other cards in this file this one calls (by `name`).
        pub calls: Vec<String>,
        pub called_by: Vec<String>,
        /// Imported value names used in the body.
        pub uses: Vec<String>,
        /// Imported names used only in type positions.
        pub type_uses: Vec<String>,
        /// Callers in other files (heuristic or better).
        pub external_callers: u32,
        pub summary: Option<Summary>,
        /// Flow unit id for this card (same as `name` for top-level functions).
        pub unit: Option<String>,
    }

    /// Model output for one symbol, always shown with an AI marker.
    pub struct Summary {
        /// One sentence that starts with a verb.
        pub desc: String,
        /// Three to seven algorithm steps.
        pub algo: Vec<String>,
        pub notes: Vec<Note>,
        pub model: String,
        /// False when the text came from a doc comment rather than a model.
        pub ai: bool,
    }

    pub struct Note {
        pub kind: NoteKind,
        pub text: String,
    }

    pub struct AiText {
        pub text: String,
        pub model: String,
    }

    pub struct TypeDecl {
        pub id: SymbolId,
        pub name: String,
        pub kind: SymbolKind,
        pub start: u32,
        pub end: u32,
        pub exported: bool,
        pub doc: Option<String>,
        pub summary: Option<String>,
    }

    /// An imported module with the symbols this file takes from it.
    pub struct ImportModule {
        /// Specifier as written.
        pub id: String,
        pub kind: ImportKind,
        /// Repo-relative path when the specifier resolves inside the repo.
        pub resolved: Option<String>,
        /// Package or standard-library import (outside the repo).
        pub external: bool,
        pub symbols: Vec<ImportSym>,
        pub line: u32,
    }

    pub struct ImportSym {
        pub name: String,
        pub type_only: bool,
        /// A member reached through a parameter rather than imported directly.
        pub member: bool,
    }

    pub struct OutlineItem {
        pub name: String,
        pub kind: SymbolKind,
        pub start: u32,
        pub end: u32,
        pub depth: u32,
    }

    pub struct FlowUnitRef {
        pub id: String,
        pub owner: String,
        pub method: Option<String>,
        pub start: u32,
        pub end: u32,
    }

    /// Returned by `get_source`.
    pub struct SourceSlice {
        pub path: String,
        pub start: u32,
        pub end: u32,
        pub text: String,
        pub monaco_lang: String,
    }

    pub struct SymRef {
        pub id: SymbolId,
        pub name: String,
        pub path: String,
        pub line: u32,
        pub confidence: Confidence,
        /// Hops from the queried symbol (1 = direct).
        pub depth: u32,
    }

    /// Returned by `get_impact`.
    pub struct Impact {
        pub symbol: SymRef,
        pub dependents: Vec<SymRef>,
        pub dependencies: Vec<SymRef>,
        pub files_affected: u32,
    }

    pub struct ModuleInfo {
        pub id: String,
        pub name: String,
        pub role: Option<String>,
        pub color: String,
        pub source: ModuleSource,
        /// Directory prefix the module is anchored to.
        pub dir: String,
        pub files: u32,
        pub lines: u64,
    }

    pub struct ModuleEdge {
        pub from: String,
        pub to: String,
        /// Number of file-level import edges between the two modules.
        pub weight: u32,
    }

    /// Returned by `get_module_map`.
    pub struct ModuleMap {
        pub modules: Vec<ModuleInfo>,
        pub edges: Vec<ModuleEdge>,
        pub cycles: Vec<Vec<String>>,
    }

    pub struct SearchHit {
        pub kind: SearchKind,
        pub name: String,
        pub path: String,
        pub line: Option<u32>,
        pub symbol_kind: Option<SymbolKind>,
        pub score: f32,
    }

    pub struct AiStatus {
        pub sidecar: SidecarState,
        pub signed_in: bool,
        pub user: Option<String>,
        pub models: Vec<AiModel>,
        pub enabled_for_repo: bool,
        pub mock: bool,
        /// Device-flow code to show while signing in.
        pub device_code: Option<DeviceCode>,
        pub message: Option<String>,
    }

    pub struct DeviceCode {
        pub user_code: String,
        pub verification_uri: String,
    }

    pub struct AiModel {
        pub id: String,
        pub name: String,
        /// USD per million input tokens.
        pub input_per_m: f64,
        pub output_per_m: f64,
    }

    /// Returned by `ai_estimate`.
    pub struct AiEstimate {
        pub scope: AiScope,
        pub symbols: u32,
        pub cached: u32,
        pub requests: u32,
        pub tokens_in: u64,
        pub tokens_out: u64,
        pub cost_usd: f64,
        pub model: String,
    }

    pub struct AiRunHandle {
        pub job_id: u64,
        pub estimate: AiEstimate,
    }

    /// Event `ai_progress`.
    pub struct AiProgress {
        pub job_id: u64,
        pub done: u32,
        pub total: u32,
        pub tokens_in: u64,
        pub tokens_out: u64,
        pub cost_usd: f64,
        pub finished: bool,
        pub error: Option<String>,
    }

    /// Returned by `get_jobs`; event `job_progress`.
    pub struct JobsState {
        pub queued: u32,
        pub running: u32,
        pub done: u32,
        pub failed: u32,
        pub cache_hits: u64,
        pub cache_misses: u64,
        pub hit_rate: f64,
        pub pending_files: Vec<String>,
    }

    pub struct FileIdWire {
        pub id: FileId,
    }
}

macro_rules! ipc_enum {
    ($($item:item)*) => {
        $(
            #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
            #[cfg_attr(feature = "ts", derive(TS))]
            #[cfg_attr(feature = "ts", ts(export))]
            #[serde(rename_all = "lowercase")]
            $item
        )*
    };
}

ipc_enum! {
    pub enum IndexPhase { Walk, Parse, Resolve, Graph, Done }
    pub enum FileStatus { Indexed, Pending, Failed, Skipped }
    pub enum NoteKind { Info, Warn }
    pub enum ModuleSource { Cluster, Ai, User }
    pub enum SearchKind { File, Symbol }
    pub enum SidecarState { Off, Starting, Ready, Failed }
    pub enum AiScope { File, Module, Repo }
}

#[cfg(all(test, feature = "ts"))]
mod tests {
    // ts-rs exports bindings for every `#[ts(export)]` type when tests run.
    #[test]
    fn export_bindings_marker() {}
}
