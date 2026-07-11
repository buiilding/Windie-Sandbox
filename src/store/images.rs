//! Persisted image asset loading, insertion, and cleanup.

use super::*;

impl Store {
    /// Loads one image asset only when it is referenced by the conversation.
    ///
    /// Conversation APIs use this as the binary transfer boundary for image
    /// parts. The `message_parts` link keeps ownership scoped to messages in
    /// the requested conversation, so clients cannot fetch an arbitrary asset by
    /// guessing an image ID from another conversation.
    pub fn load_conversation_image_asset(
        &self,
        conversation_id: &ConversationId,
        image_asset_id: &ImageAssetId,
    ) -> Result<ImagePart> {
        self.ensure_conversation_exists(conversation_id)?;

        self.connection
            .query_row(
                "
                SELECT image_assets.id, image_assets.mime_type, image_assets.bytes
                FROM image_assets
                WHERE image_assets.id = ?2
                  AND EXISTS (
                      SELECT 1
                      FROM message_parts
                      JOIN messages ON messages.id = message_parts.message_id
                      WHERE messages.conversation_id = ?1
                        AND message_parts.image_asset_id = image_assets.id
                  )
                ",
                params![conversation_id.as_str(), image_asset_id.as_str()],
                |row| {
                    Ok(ImagePart {
                        asset_id: ImageAssetId::new(row.get::<_, String>(0)?),
                        mime_type: row.get(1)?,
                        bytes: row.get(2)?,
                    })
                },
            )
            .optional()
            .context("failed to load conversation image asset")?
            .ok_or_else(|| {
                error::not_found(format!(
                    "image asset does not exist in conversation: {image_asset_id}"
                ))
            })
    }
}

/// Copies image bytes into image asset storage inside an existing transaction.
pub(super) fn insert_image_asset_in_transaction(
    transaction: &Transaction<'_>,
    mime_type: &str,
    bytes: &[u8],
    now: i64,
) -> Result<ImageAssetId> {
    let asset_id = ImageAssetId::new(Uuid::new_v4().to_string());
    let sha256 = sha256_hex(bytes);

    transaction
        .execute(
            "
            INSERT INTO image_assets (id, bytes, mime_type, sha256, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ",
            params![asset_id.as_str(), bytes, mime_type, sha256, now],
        )
        .context("failed to save image asset")?;

    Ok(asset_id)
}

/// Links one image asset to an ordered message part.
pub(super) fn insert_image_part_in_transaction(
    transaction: &Transaction<'_>,
    message_id: &MessageId,
    position: usize,
    image_asset_id: &ImageAssetId,
) -> Result<()> {
    transaction
        .execute(
            "
            INSERT INTO message_parts (id, message_id, position, kind, text, image_asset_id)
            VALUES (?1, ?2, ?3, 'image', NULL, ?4)
            ",
            params![
                Uuid::new_v4().to_string(),
                message_id.as_str(),
                position as i64,
                image_asset_id.as_str()
            ],
        )
        .context("failed to save image message part")?;

    Ok(())
}
/// Removes image assets no remaining message part references.
pub(super) fn delete_orphan_image_assets_in_transaction(
    transaction: &Transaction<'_>,
) -> Result<()> {
    transaction
        .execute(
            "
            DELETE FROM image_assets
            WHERE id NOT IN (
                SELECT image_asset_id
                FROM message_parts
                WHERE image_asset_id IS NOT NULL
            )
            ",
            [],
        )
        .context("failed to delete orphan image assets")?;

    Ok(())
}

/// Returns lowercase hex SHA-256 text for stored asset identity metadata.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}
