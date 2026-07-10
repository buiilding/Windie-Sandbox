# Provider Configuration

Windie delegates model routing and provider configuration to Bifrost. Provider
secrets remain outside the source checkout in:

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
```

Do not commit this file. After adding or changing a secret, restart Bifrost so
the gateway process receives the new environment:

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

Verify the models Bifrost exposes to Windie:

```bash
windie models
```
