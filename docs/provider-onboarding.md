# Provider onboarding

## OpenRouter API key flow

Windie does not fetch OpenRouter API keys from the global environment or from
Windie's `.env` file. The user supplies the key interactively during
`windie onboard`.

The flow is:

1. Windie asks Bifrost for the complete provider catalog:

   ```http
   GET http://localhost:8080/api/providers/catalog
   ```

2. The user selects OpenRouter from the catalog.

3. If OpenRouter is not configured, Windie creates the provider in Bifrost:

   ```http
   POST /api/providers
   {"provider":"openrouter"}
   ```

4. Windie prompts for the API key using hidden terminal input. The key is held
   in memory and is not written to Windie's `.env` file.

5. Windie submits the key to Bifrost's managed-key endpoint:

   ```http
   POST /api/providers/openrouter/keys
   {
     "name": "windie-openrouter-1",
     "value": "user-entered-key",
     "models": ["*"],
     "blacklisted_models": [],
     "weight": 1.0,
     "enabled": true
   }
   ```

6. Bifrost validates the key, stores it in its provider configuration store,
   and refreshes model discovery.

When Windie launches the local Bifrost binary, Bifrost's persistent data lives
under:

```text
~/.windie/bifrost/data
```

The API key is therefore owned and persisted by Bifrost. Windie's
`~/.windie/.env` remains reserved for MCP and other Windie-local extension
secrets.

## Environment boundary

When Windie launches Bifrost, it clears the inherited process environment. It
passes only values explicitly loaded from `~/.windie/.env`, excluding known LLM
provider keys. This prevents OpenRouter or other model-provider keys from being
silently inherited from a shell or global `.env` file.

Relevant implementation files:

- `src/cli/onboard.rs` — hidden terminal input
- `src/operation/onboarding.rs` — onboarding workflow
- `src/llm/management.rs` — Bifrost management client
- `src/gateway.rs` — owned Bifrost process and environment boundary
- `bifrost/transports/bifrost-http/handlers/provider_keys.go` — Bifrost key persistence path
