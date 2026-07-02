use anyhow::{Context, Result};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

pub struct TerminalInput {
    editor: DefaultEditor,
}

impl TerminalInput {
    pub fn new() -> Result<Self> {
        Ok(Self {
            editor: DefaultEditor::new().context("failed to create terminal input")?,
        })
    }

    pub fn read(&mut self, prompt: &str) -> Result<Option<String>> {
        match self.editor.readline(prompt) {
            Ok(input) => {
                if !input.trim().is_empty() {
                    let _ = self.editor.add_history_entry(input.as_str());
                }

                Ok(Some(input))
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => Ok(None),
            Err(error) => Err(error).context("failed to read terminal input"),
        }
    }
}
