//! Conversation message persistence facade.

use super::*;

mod codecs;
mod insert;
mod load;
mod mutate;

#[derive(Clone, Copy)]
pub(super) enum InsertSelection<'a> {
    Always,
    IfCurrent(Option<&'a MessageId>),
}

pub(super) use codecs::encode_message_metadata;
pub(super) use insert::{insert_unsaved_message_parts_in_transaction, select_inserted_message};
