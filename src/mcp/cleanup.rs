//! Typed cleanup actions for code-approved provider runtimes.
//!
//! Cleanup actions are runtime behavior, not serialized provider metadata. The
//! provider definition owns the action, while the provider operation owns the
//! lifecycle ordering around it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Local cleanup owned by one approved provider definition.
pub(crate) enum ProviderCleanup {
    /// The provider has no Windie-owned local runtime or registration.
    None,
    /// Run CUA Driver's platform-specific official uninstaller.
    CuaDriver,
    /// Remove exact directories beneath Windie's data root.
    WindieDirectories(&'static [&'static str]),
    /// Remove Basic Memory's Windie project and its Windie-owned cache.
    BasicMemory,
}
