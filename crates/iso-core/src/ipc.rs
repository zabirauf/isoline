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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub lines: u64,
    }

    /// Returned by `open_repo`. Indexing continues in the background.
    pub struct RepoSummary {
        pub root: String,
        pub name: String,
        pub files: u32,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
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
        /// Definition targets for names used in this file (see `SymbolTarget`).
        pub targets: Vec<SymbolTarget>,
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
        /// Parameters and return type parsed from the declaration.
        pub sig: SigInfo,
    }

    /// A declaration's parameter list and return type, as written.
    pub struct SigInfo {
        pub params: Vec<ParamInfo>,
        /// Return type text, whitespace collapsed (`Promise<Result>`, `-> io::Result<()>` gives `io::Result<()>`).
        pub returns: Option<String>,
    }

    pub struct ParamInfo {
        /// Name as written: `harness`, `self`, `{ a, b }`, `*args`.
        pub name: String,
        /// Type annotation text, whitespace collapsed, if any.
        pub type_text: Option<String>,
        /// Default value text, if any.
        pub default: Option<String>,
        /// Rest or variadic parameter (`...xs`, `*args`, `**kw`).
        pub rest: bool,
        /// Optional parameter (`x?: T` or has a default).
        pub optional: bool,
    }

    /// Where a name used in this file is defined: drives type colors, hover definitions and
    /// click-to-jump. Every identifier that appears in a card signature, every import binding
    /// and every top-level symbol of the file has one.
    pub struct SymbolTarget {
        pub name: String,
        pub origin: SymbolOrigin,
        pub kind: Option<SymbolKind>,
        /// Repo-relative file of the definition; `None` for external and builtin names.
        pub path: Option<String>,
        pub start: u32,
        pub end: u32,
        /// Module specifier for imported and external names.
        pub module: Option<String>,
        pub doc: Option<String>,
        pub signature: Option<String>,
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
        /// One-sentence description of the unit (model output or doc comment), when known.
        pub summary: Option<String>,
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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub tokens_in: u64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub tokens_out: u64,
        pub cost_usd: f64,
        pub model: String,
    }

    pub struct AiRunHandle {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub job_id: u64,
        pub estimate: AiEstimate,
    }

    /// Event `ai_progress`.
    pub struct AiProgress {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub job_id: u64,
        pub done: u32,
        pub total: u32,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub tokens_in: u64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub cache_hits: u64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub cache_misses: u64,
        pub hit_rate: f64,
        pub pending_files: Vec<String>,
    }

    pub struct FileIdWire {
        pub id: FileId,
    }

    // ---------- git ----------

    /// Returned by `git_status`. `isRepo` is false when the folder is not in a git work tree.
    pub struct GitStatus {
        pub is_repo: bool,
        /// Current branch; `None` when HEAD is detached.
        pub branch: Option<String>,
        /// Short commit id of HEAD; `None` before the first commit.
        pub head: Option<String>,
        pub upstream: Option<String>,
        pub ahead: u32,
        pub behind: u32,
        /// Tracked files with changes, staged or not.
        pub changed: u32,
        pub untracked: u32,
        /// `merge`, `rebase`, `cherry-pick`, `revert` or `bisect` while one is in progress.
        pub operation: Option<String>,
        /// A remote points at github.com, so pull requests can be listed through `gh`.
        pub github: bool,
    }

    pub struct GitBranch {
        /// `main`, or `origin/main` for a remote branch.
        pub name: String,
        pub remote: bool,
        pub current: bool,
        /// Short commit id of the tip.
        pub sha: String,
        pub subject: String,
        pub author: String,
        /// Committer date of the tip, unix seconds.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub date: i64,
        pub upstream: Option<String>,
    }

    pub struct GitCommit {
        pub sha: String,
        pub short: String,
        pub subject: String,
        pub author: String,
        /// Committer date, unix seconds.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub date: i64,
        /// Branch and tag names that point at this commit.
        pub refs: Vec<String>,
    }

    pub struct GitTag {
        pub name: String,
        pub sha: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub date: i64,
    }

    /// Returned by `git_refs`: what the branch menu and the compare pickers offer.
    pub struct GitRefs {
        /// Local branches first, then remote ones; most recent commit first within each.
        pub branches: Vec<GitBranch>,
        pub tags: Vec<GitTag>,
        /// Recent commits on HEAD.
        pub commits: Vec<GitCommit>,
    }

    pub struct PullRequest {
        pub number: u32,
        pub title: String,
        pub author: String,
        pub head_ref: String,
        pub base_ref: String,
        pub draft: bool,
        pub url: String,
        /// Last update, unix seconds.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub updated: i64,
        pub additions: u32,
        pub deletions: u32,
        pub changed_files: u32,
    }

    /// Returned by `git_prs`. `available` is false when `gh` is missing or not signed in;
    /// `message` then says what to do.
    pub struct PullRequests {
        pub available: bool,
        pub message: Option<String>,
        pub items: Vec<PullRequest>,
    }

    // ---------- compare ----------

    /// One side of a comparison, resolved.
    pub struct CompareRev {
        /// What was asked for: `main`, `a1b2c3d`, `pr:12`, `worktree`.
        pub spec: String,
        /// Full commit id; `worktree` for uncommitted changes.
        pub sha: String,
        /// Display text: `main`, `#12 · fix-login`, `Working tree`.
        pub label: String,
        pub subject: Option<String>,
    }

    pub struct CompareTotals {
        pub files: u32,
        pub added: u32,
        pub modified: u32,
        pub deleted: u32,
        pub renamed: u32,
        pub additions: u32,
        pub deletions: u32,
        pub symbols_added: u32,
        pub symbols_deleted: u32,
        pub symbols_modified: u32,
        pub signatures_changed: u32,
        pub imports_added: u32,
        pub imports_deleted: u32,
    }

    /// Returned by `compare`: what changed between two revisions, in the map's terms.
    pub struct Comparison {
        pub base: CompareRev,
        pub head: CompareRev,
        /// The base side is this merge base of the two (what a pull request shows).
        pub merge_base: Option<String>,
        pub pr: Option<PullRequest>,
        pub files: Vec<FileChange>,
        pub totals: CompareTotals,
        /// Modules with changed files, most changed lines first.
        pub modules: Vec<ModuleChange>,
        /// Module dependencies whose file-level import count changed.
        pub module_edges: Vec<ModuleEdgeChange>,
        /// Deleted or re-signatured functions that unchanged files still call.
        pub impact: Vec<ImpactItem>,
        /// Commits on the head side that the base side lacks, newest first (at most 200).
        pub commits: Vec<GitCommit>,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub elapsed_ms: u64,
        /// Too many changed files: symbols were compared for the first ones only.
        pub truncated: bool,
    }

    pub struct FileChange {
        pub path: String,
        /// Path on the base side, for renames.
        pub old_path: Option<String>,
        pub change: ChangeKind,
        pub lang: Language,
        pub tier: Tier,
        pub additions: u32,
        pub deletions: u32,
        pub binary: bool,
        pub module: Option<ModuleRef>,
        /// Functions, classes and types added, deleted or modified, in file order.
        pub symbols: Vec<SymbolChange>,
        /// Imports added and deleted, with what they resolve to.
        pub imports: Vec<ImportChange>,
    }

    pub struct SymbolChange {
        /// Qualified name: `run`, `MyClass.run`.
        pub name: String,
        pub kind: SymbolKind,
        /// `added`, `deleted` or `modified`.
        pub change: ChangeKind,
        pub exported: bool,
        pub signature_before: Option<String>,
        pub signature_after: Option<String>,
        /// Lines on the base side (absent for added symbols).
        pub before: Option<LineRange>,
        /// Lines on the head side (absent for deleted symbols).
        pub after: Option<LineRange>,
        /// Callees that appear in, or vanish from, the body.
        pub calls_added: Vec<String>,
        pub calls_deleted: Vec<String>,
        /// Call sites in other files, from the index.
        pub callers: u32,
    }

    pub struct ImportChange {
        /// `added` or `deleted`.
        pub change: ChangeKind,
        pub specifier: String,
        /// Repo file it resolves to.
        pub file: Option<String>,
        /// Package it resolves to.
        pub external: Option<String>,
    }

    pub struct ModuleChange {
        pub id: String,
        pub name: String,
        pub color: String,
        pub dir: String,
        pub files: u32,
        pub additions: u32,
        pub deletions: u32,
        /// Symbols added, deleted or modified in the module.
        pub symbols: u32,
    }

    pub struct ModuleEdgeChange {
        pub from: String,
        pub to: String,
        /// File-level imports from `from` to `to` on each side.
        pub before: u32,
        pub after: u32,
    }

    pub struct ImpactItem {
        pub symbol: String,
        pub path: String,
        pub change: ChangeKind,
        pub signature_before: Option<String>,
        pub signature_after: Option<String>,
        /// Call sites in files the comparison did not change (the first 50 files).
        pub callers: Vec<FileRef>,
        /// Unchanged files that call it, including the ones not listed.
        pub caller_files: u32,
        /// The symbol is not in the index (the head side is checked out), so `callers` are the
        /// files that import it from its file rather than call edges.
        pub by_import: bool,
    }

    /// Returned by `compare_file`: both versions of one file, for the diff view.
    pub struct FileDiffText {
        pub path: String,
        pub old_path: Option<String>,
        pub before: Option<String>,
        pub after: Option<String>,
        pub monaco_lang: String,
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
    pub enum SymbolOrigin { Local, Imported, External, Builtin }
    pub enum ChangeKind { Added, Modified, Deleted, Renamed }
}

#[cfg(all(test, feature = "ts"))]
mod tests {
    // ts-rs exports bindings for every `#[ts(export)]` type when tests run.
    #[test]
    fn export_bindings_marker() {}
}
