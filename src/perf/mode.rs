//! Benchmark mode, category, and option types.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Benchmark mode selected by the CLI.
pub enum BenchmarkMode {
    Local,
    Conversation,
}

impl BenchmarkMode {
    /// Returns the mode label printed in benchmark output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Conversation => "conversation",
        }
    }

    /// Marks benchmark modes that may send a paid provider request.
    pub fn may_call_provider(self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Local benchmark category selected by `windie bench` flags.
pub enum BenchmarkCategory {
    Persistence,
    Conversation,
    Runtime,
    Tools,
    Mutations,
    Mcp,
}

impl BenchmarkCategory {
    /// Returns every local benchmark category in stable output order.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Persistence,
            Self::Conversation,
            Self::Runtime,
            Self::Tools,
            Self::Mutations,
            Self::Mcp,
        ]
    }
}

/// Optional controls for benchmark execution and output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkOptions {
    pub runs: usize,
    pub json: bool,
    pub categories: Vec<BenchmarkCategory>,
}

impl Default for BenchmarkOptions {
    /// Defaults to one human-readable run to preserve the simple benchmark
    /// behavior.
    fn default() -> Self {
        Self {
            runs: 1,
            json: false,
            categories: BenchmarkCategory::all(),
        }
    }
}
