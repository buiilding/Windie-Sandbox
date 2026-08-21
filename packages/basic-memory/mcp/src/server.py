"""MCPB entry point marker for the uv-managed Basic Memory server.

Windie launches the package's declared ``basic-memory mcp`` console command
through uv. The file is present because MCPB requires an entry point for every
server package; the command declaration remains the authoritative launch path.
"""
