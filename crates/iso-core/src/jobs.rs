//! Background job kinds and priorities. Lower number runs first.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Parse and extract one file.
    Parse,
    /// Re-resolve imports of one file.
    Resolve,
    /// Recompute graph aggregates.
    Graph,
    /// Run a precise indexer (SCIP).
    Precise,
    /// AI leaf pass for one file.
    AiLeaf,
    /// AI cluster pass for one module.
    AiCluster,
    /// AI repo pass.
    AiRepo,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Parse => "parse",
            JobKind::Resolve => "resolve",
            JobKind::Graph => "graph",
            JobKind::Precise => "precise",
            JobKind::AiLeaf => "ai_leaf",
            JobKind::AiCluster => "ai_cluster",
            JobKind::AiRepo => "ai_repo",
        }
    }
    pub fn parse(s: &str) -> Option<JobKind> {
        Some(match s {
            "parse" => JobKind::Parse,
            "resolve" => JobKind::Resolve,
            "graph" => JobKind::Graph,
            "precise" => JobKind::Precise,
            "ai_leaf" => JobKind::AiLeaf,
            "ai_cluster" => JobKind::AiCluster,
            "ai_repo" => JobKind::AiRepo,
            _ => return None,
        })
    }
}

/// Priority bands, in the order the plan gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    VisibleFile = 0,
    Neighbors = 1,
    ExpandedOrRecent = 2,
    Repo = 3,
    Precise = 4,
    Ai = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}
