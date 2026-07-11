//! SQLite codecs for persisted messages.

use super::super::*;
use super::mutate::MessageTreeRow;

impl FromSql for Role {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            role => Err(FromSqlError::Other(
                format!("unknown message role: {role}").into(),
            )),
        }
    }
}

pub(super) fn read_message_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    let metadata_json = row.get::<_, Option<String>>(4)?;

    Ok(Message {
        id: Some(MessageId::new(row.get::<_, String>(0)?)),
        parent_message_id: row.get::<_, Option<String>>(1)?.map(MessageId::new),
        role: row.get(2)?,
        content: row.get(3)?,
        parts: Vec::new(),
        metadata: decode_message_metadata(metadata_json)?,
    })
}

/// Converts one SQLite message row into a lightweight tree mutation row.
pub(super) fn read_message_tree_row(row: &Row<'_>) -> rusqlite::Result<MessageTreeRow> {
    let metadata_json = row.get::<_, Option<String>>(3)?;

    Ok(MessageTreeRow {
        id: MessageId::new(row.get::<_, String>(0)?),
        parent_message_id: row.get::<_, Option<String>>(1)?.map(MessageId::new),
        role: row.get(2)?,
        metadata: decode_message_metadata(metadata_json)?,
    })
}

pub(super) fn read_message_part_row(row: &Row<'_>) -> rusqlite::Result<(String, MessagePart)> {
    let message_id = row.get::<_, String>(0)?;
    let kind = row.get::<_, String>(1)?;
    let part = match kind.as_str() {
        "text" => MessagePart::Text(row.get::<_, String>(2)?),
        "image" => MessagePart::Image(ImagePart {
            asset_id: ImageAssetId::new(row.get::<_, String>(3)?),
            mime_type: row.get(4)?,
            bytes: row.get(5)?,
        }),
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                Type::Text,
                format!("unknown message part kind: {kind}").into(),
            ));
        }
    };

    Ok((message_id, part))
}

/// Serializes typed message metadata for SQLite storage.
pub(in crate::store) fn encode_message_metadata(
    metadata: Option<&MessageMetadata>,
) -> Result<Option<String>> {
    metadata
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize message metadata")
}

/// Decodes SQLite JSON metadata into the typed runtime metadata model.
fn decode_message_metadata(metadata: Option<String>) -> rusqlite::Result<Option<MessageMetadata>> {
    metadata
        .map(|metadata| {
            serde_json::from_str(&metadata).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })
        })
        .transpose()
}
