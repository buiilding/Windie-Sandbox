//! Account identity resolved from a verified Supabase user.

/// Server-side account identity attached to every authenticated request.
///
/// The database ID is internal to Windie. Clients never choose it and every
/// hosted-store query scopes data to it.
#[derive(Debug, Clone)]
pub(crate) struct HostedAccount {
    pub(crate) id: String,
    pub(crate) auth_subject: String,
}
