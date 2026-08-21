//! Windie CLI entrypoint.
//!
//! The binary only reads the typed command and hands it to the CLI adapter
//! boundary. Command orchestration belongs in `cli::adapter` modules so this
//! file remains a startup wiring point.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    windie::cli::run(windie::cli::read()).await
}
