//! Plugin inspection API handlers.
//!
//! These handlers expose installed skill Markdown only when a client opens a
//! plugin's detail view. The provider list remains compact and does not carry
//! the full contents of every skill bundle.

use super::*;

pub(super) async fn get_plugin_skills(
    Path(plugin_id): Path<String>,
) -> ApiResult<operation::PluginSkillsResponse> {
    let plugin_id = crate::plugins::PluginId::new(plugin_id);
    Ok(Json(operation::read_plugin_skills(&plugin_id)?))
}
