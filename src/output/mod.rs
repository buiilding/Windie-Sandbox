//! Terminal output boundary.
//!
//! This module owns CLI printing for assistant streams and command output.
//! Other modules should pass display data here instead of formatting terminal
//! output themselves.

mod formatting;
mod terminal;

pub(crate) use formatting::{
    ConversationListReport, available_tool_lines, conversation_lines, encode_query_value,
    format_duration, help_lines, invalid_usage_lines, message_lines, model_lines,
    performance_comparison_lines, performance_report_lines, print_lines, text_preview, tree_lines,
};
pub(crate) use terminal::{RuntimeOutput, TerminalOutput};

#[cfg(test)]
use crate::conversation::Message;
#[cfg(test)]
use crate::llm::ModelInfo;
#[cfg(test)]
use crate::operation::InspectionReport;
#[cfg(test)]
use crate::store::ConversationInfo;

#[cfg(test)]
mod tests;
