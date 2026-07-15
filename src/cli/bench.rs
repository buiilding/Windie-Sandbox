//! Benchmark and baseline command parsing.

use super::*;

/// Parses benchmark commands and their optional output controls.
///
/// `--runs` repeats local measurements so users can compare median/p95 values
/// across code changes. `--json` writes a persistent artifact to stdout.
pub(super) fn parse_bench_command(args: &[String]) -> Command {
    let Some(options) = parse_benchmark_options(args) else {
        return Command::Invalid;
    };

    Command::Bench {
        mode: BenchmarkMode::Local,
        conversation_id: None,
        options,
    }
}

/// Parses optional benchmark flags after the mode/conversation selector.
fn parse_benchmark_options(args: &[String]) -> Option<BenchmarkOptions> {
    let mut options = BenchmarkOptions::default();
    let mut categories = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--json") => {
                options.json = true;
                index += 1;
            }
            Some("--runs") => {
                let runs = args.get(index + 1)?.parse::<usize>().ok()?;
                if runs == 0 {
                    return None;
                }

                options.runs = runs;
                index += 2;
            }
            Some("--persistence") => {
                categories.push(BenchmarkCategory::Persistence);
                index += 1;
            }
            Some("--conversation") => {
                categories.push(BenchmarkCategory::Conversation);
                index += 1;
            }
            Some("--runtime") => {
                categories.push(BenchmarkCategory::Runtime);
                index += 1;
            }
            Some("--tools") => {
                categories.push(BenchmarkCategory::Tools);
                index += 1;
            }
            Some("--mutations") => {
                categories.push(BenchmarkCategory::Mutations);
                index += 1;
            }
            Some("--mcp") => {
                categories.push(BenchmarkCategory::Mcp);
                index += 1;
            }
            _ => return None,
        }
    }
    if !categories.is_empty() {
        options.categories = BenchmarkCategory::all()
            .into_iter()
            .filter(|category| categories.contains(category))
            .collect();
    }

    Some(options)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Baseline command selected by a top-level benchmark baseline action.
pub(super) enum BaselineCommand {
    Compare,
    Update,
}

/// Parses `windie compare baseline` and `windie update baseline`.
pub(super) fn parse_baseline_command(args: &[String], command: BaselineCommand) -> Command {
    let Some(options) = parse_benchmark_options(args) else {
        return Command::Invalid;
    };

    match command {
        BaselineCommand::Compare => Command::CompareBaseline { options },
        BaselineCommand::Update => Command::UpdateBaseline { options },
    }
}
