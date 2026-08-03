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

    /// Reports whether this mode performs a paid provider request.
    ///
    /// Both supported modes are deterministic and provider-free. External
    /// provider/inference measurements are intentionally separate integration
    /// work rather than part of `windie bench`.
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
    Serialization,
    Runtime,
    Sessions,
    Tools,
    Mutations,
    Mcp,
    Api,
    Lifecycle,
}

impl BenchmarkCategory {
    /// Returns every local benchmark category in stable output order.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Persistence,
            Self::Conversation,
            Self::Serialization,
            Self::Runtime,
            Self::Sessions,
            Self::Tools,
            Self::Mutations,
            Self::Mcp,
            Self::Api,
            Self::Lifecycle,
        ]
    }

    /// Returns the deterministic provider-free categories used by default.
    pub fn deterministic() -> Vec<Self> {
        vec![
            Self::Persistence,
            Self::Conversation,
            Self::Serialization,
            Self::Runtime,
            Self::Sessions,
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
            categories: BenchmarkCategory::deterministic(),
        }
    }
}
