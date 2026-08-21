# Basic Memory

Basic Memory gives Windie a local-first knowledge base backed by Markdown files
and an MCP server.

The package keeps the server runtime, Python dependencies, uv cache, and
generated configuration under Windie ownership. User notes remain in:

```text
~/.windie/memory/
```

Uninstalling the plugin removes the installed server and runtime files but does
not delete those user-owned memory files.

Basic Memory is published by Basic Machines:

https://github.com/basicmachines-co/basic-memory
