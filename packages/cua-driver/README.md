# CUA Driver

CUA Driver provides local computer-control tools through MCP. This Windie
package contains the pinned TryCua Rust release and its macOS application
bundle, so Windie does not run the upstream shell installer or depend on a
system-wide `cua-driver` command.

The current package release is the universal macOS build. Windows and Linux
marketplace variants should be published as platform-specific package
artifacts before this plugin is advertised for those systems.

## Permissions

Windie starts the bundled CuaDriver application through MCP. macOS may ask the
user to grant Accessibility, Screen Recording, or Automation permissions.
Windie does not silently grant, revoke, or bypass those operating-system
permissions. After granting them, repair the provider to rediscover its tools.

Uninstall removes the Windie-owned package and runtime. It does not remove
user permissions or alter unrelated CUA installations.
