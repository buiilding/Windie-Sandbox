# Provider Configuration

Windie delegates model routing and model-provider configuration to Bifrost.
Windie and Bifrost share one canonical provider-secret file outside the source
checkout:

```text
~/.config/windie/providers.env
```

Create the file and restrict its permissions:

```bash
mkdir -p ~/.config/windie
touch ~/.config/windie/providers.env
chmod 600 ~/.config/windie/providers.env
```

Add one environment variable for each provider. For example:

```dotenv
OPENAI_API_KEY=...
ANTHROPIC_API_KEY=...
OPENROUTER_API_KEY=...
EXA_API_KEY=...
```

Do not commit this file. Windie reads named MCP credentials from it when
starting approved provider processes and passes the complete environment to
Bifrost. A process environment value overrides the matching file entry.

After adding or changing a secret, restart Windie and Bifrost so existing
provider processes and sessions receive it:

```bash
windie gateway stop
windie gateway start
```

Open the Bifrost dashboard at [http://localhost:8080](http://localhost:8080).
In **Models**, open the provider configuration and add each provider represented
in `providers.env`. Add a key for the provider and reference the corresponding
environment variable instead of entering the secret directly:

```text
OpenAI:     env.OPENAI_API_KEY
Anthropic:  env.ANTHROPIC_API_KEY
OpenRouter: env.OPENROUTER_API_KEY
```

Use the provider's plain name for the configuration name. Saving these provider
rows completes Bifrost configuration; the environment file alone does not
create providers or models.

`WINDIE_ENV_FILE` selects a different canonical file when needed. Without that
override, Windie checks `~/.config/windie/providers.env`, including during
development runs that isolate data and other configuration under `target/`.

Verify the models Bifrost exposes to Windie:

```bash
windie models
```
