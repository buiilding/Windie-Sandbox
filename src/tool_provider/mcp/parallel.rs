//! Parallel Search hosted MCP provider definition.
//!
//! Parallel exposes web search and URL extraction through a remote Streamable
//! HTTP MCP endpoint. Windie supports anonymous access by default and can add
//! an optional Bearer API key for higher rate limits.

use super::McpProviderDefinition;
use crate::mcp::{McpHttpAuthorization, McpHttpEndpoint, McpTransport};
use crate::tool_provider::{
    ProviderAuthentication, ProviderManifest, ProviderPermission, ProviderPlatform, ProviderScope,
    ProviderSecret,
};

const PARALLEL_MCP_URL: &str = "https://search.parallel.ai/mcp";
const PARALLEL_API_KEY_ENV: &str = "PARALLEL_API_KEY";

/// Returns the code-approved Parallel Search MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let transport = McpTransport::streamable_http(McpHttpEndpoint {
        url: PARALLEL_MCP_URL,
        authorization: McpHttpAuthorization::OptionalBearerEnv(PARALLEL_API_KEY_ENV),
    });

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_streamable_http(
            "parallel-search",
            "Parallel Search",
            "Search the live web and extract content from URLs through Parallel Search MCP.",
            PARALLEL_MCP_URL,
            ProviderPlatform::desktop(),
            vec![ProviderSecret::optional(
                PARALLEL_API_KEY_ENV,
                "Parallel API key for higher rate limits",
            )],
            vec![ProviderPermission::Network],
        )
        .with_author("Parallel")
        .with_metadata(
            ProviderScope::Cloud,
            ProviderAuthentication::OptionalApiKey,
            "web_data",
            &["web", "search", "research", "extraction"],
            Some("https://docs.parallel.ai/integrations/mcp/search-mcp"),
            &[
                "Parallel Search works anonymously for basic usage.",
                "Add PARALLEL_API_KEY for higher rate limits.",
            ],
        )
        .with_readme(include_str!("readmes/parallel-search.md")),
        provider_id: "parallel-search",
        schema_prefix: "parallel_search",
        display_name: "Parallel Search",
        transport,
        package_command: None,
        readiness_probe: None,
        setup: None,
    }
}
