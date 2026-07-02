use std::io::{self, Write};

use anyhow::{Context, Result};

pub struct TerminalOutput;

impl TerminalOutput {
    pub fn start_assistant_message(&self) {
        println!();
    }

    pub fn assistant_delta(&self, text: &str) -> Result<()> {
        print!("{text}");
        io::stdout()
            .flush()
            .context("failed to flush assistant output")
    }

    pub fn end_assistant_message(&self) {
        println!("\n");
    }

    pub fn newline(&self) {
        println!();
    }
}
