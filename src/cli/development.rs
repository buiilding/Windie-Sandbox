//! Parser for repository development, release, marketplace, and benchmark commands.

use crate::perf::{BenchmarkCategory, BenchmarkOptions};

use super::{
    BenchmarkCommand, Command, ConversationId, DevCommand, DevComponent, MarketplaceCommand,
    ReleaseCommand,
};

/// Parses the `windie dev` command group.
pub(super) fn parse_dev_command(args: &[String]) -> Command {
    match args {
        [action] if action == "up" => Command::Dev(DevCommand::Up),
        [action, component] if action == "run" => match component.as_str() {
            "gateway" => Command::Dev(DevCommand::Run {
                component: DevComponent::Gateway,
            }),
            "api" => Command::Dev(DevCommand::Run {
                component: DevComponent::Api,
            }),
            "inspector" => Command::Dev(DevCommand::Run {
                component: DevComponent::Inspector,
            }),
            _ => Command::Invalid,
        },
        [action] if action == "status" => Command::Dev(DevCommand::Status),
        [action] if action == "down" => Command::Dev(DevCommand::Down),
        _ => Command::Invalid,
    }
}

/// Parses the `windie release` command group.
pub(super) fn parse_release_command(args: &[String]) -> Command {
    match args {
        [action] if action == "build" => Command::Release(ReleaseCommand::Build),
        [action] if action == "install" => Command::Release(ReleaseCommand::Install),
        [action] if action == "verify" => Command::Release(ReleaseCommand::Verify),
        _ => Command::Invalid,
    }
}

/// Parses the `windie marketplace` command group.
pub(super) fn parse_marketplace_command(args: &[String]) -> Command {
    match args {
        [action] if action == "build" => Command::Marketplace(MarketplaceCommand::Build),
        [action] if action == "serve" => Command::Marketplace(MarketplaceCommand::Serve),
        [action] if action == "publish" => Command::Marketplace(MarketplaceCommand::Publish),
        _ => Command::Invalid,
    }
}

/// Parses a benchmark run and its optional conversation selector.
pub(super) fn parse_benchmark_command(args: &[String]) -> Command {
    let (conversation_id, option_args) = match args.first() {
        Some(value) if !value.starts_with('-') => (Some(ConversationId::new(value)), &args[1..]),
        _ => (None, args),
    };
    match parse_benchmark_options(option_args) {
        Some(options) => Command::Benchmark(BenchmarkCommand::Run {
            conversation_id,
            options,
        }),
        None => Command::Invalid,
    }
}

/// Parses `windie compare baseline` benchmark options.
pub(super) fn parse_compare_baseline_command(args: &[String]) -> Command {
    match parse_benchmark_options(args) {
        Some(options) => Command::Benchmark(BenchmarkCommand::CompareBaseline { options }),
        None => Command::Invalid,
    }
}

/// Parses `windie update baseline` benchmark options.
pub(super) fn parse_update_baseline_command(args: &[String]) -> Command {
    match parse_benchmark_options(args) {
        Some(options) => Command::Benchmark(BenchmarkCommand::UpdateBaseline { options }),
        None => Command::Invalid,
    }
}

/// Parses benchmark selection, repetition, and JSON-output flags.
fn parse_benchmark_options(args: &[String]) -> Option<BenchmarkOptions> {
    let mut options = BenchmarkOptions::default();
    let mut categories = Vec::new();
    let mut all = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => options.json = true,
            "--all" => all = true,
            "--runs" => {
                index += 1;
                options.runs = args.get(index)?.parse().ok()?;
                if options.runs == 0 {
                    return None;
                }
            }
            flag => categories.push(match flag {
                "--persistence" => BenchmarkCategory::Persistence,
                "--conversation" => BenchmarkCategory::Conversation,
                "--serialization" => BenchmarkCategory::Serialization,
                "--runtime" => BenchmarkCategory::Runtime,
                "--sessions" => BenchmarkCategory::Sessions,
                "--tools" => BenchmarkCategory::Tools,
                "--mutations" => BenchmarkCategory::Mutations,
                "--mcp" => BenchmarkCategory::Mcp,
                "--api" => BenchmarkCategory::Api,
                "--lifecycle" => BenchmarkCategory::Lifecycle,
                _ => return None,
            }),
        }
        index += 1;
    }
    if all {
        options.categories = BenchmarkCategory::all();
    } else if !categories.is_empty() {
        options.categories = BenchmarkCategory::all()
            .into_iter()
            .filter(|category| categories.contains(category))
            .collect();
    }
    Some(options)
}
