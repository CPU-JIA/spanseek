//! Version policy shared by the sync authority and its compatibility tests.

/// Earliest authority database that this build can migrate in place.
pub const MIN_SUPPORTED_AUTHORITY_SCHEMA: i64 = 1;
/// Current authority database schema. Keep in lockstep with `0009_member_identity.sql`.
pub const CURRENT_AUTHORITY_SCHEMA: i64 = 9;
/// Wire protocol is the signed request + object-op contract from G02–G10.
/// Version 2 adds roles and owner-only administration. Version 3 adds human
/// member identity, member-targeted device invites and member administration.
pub const SYNC_PROTOCOL_VERSION: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaDecision {
    /// Empty file: initialize a new authority.
    Create,
    /// Older compatible authority: migrate before serving requests.
    Migrate { from: i64 },
    /// Current authority: serve directly.
    Current,
    /// Unknown/future authority: never open or modify it.
    Reject(RejectReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    ForeignApplication,
    FutureSchema,
    InvalidVersion,
}

impl RejectReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::ForeignApplication => "foreign_application",
            Self::FutureSchema => "future_schema",
            Self::InvalidVersion => "invalid_version",
        }
    }
}

/// Classifies a SQLite identity without writing to it. `empty` is true only
/// when application_id/user_version are both zero and no user tables exist.
pub fn classify_authority_database(
    application_id: i64,
    user_version: i64,
    empty: bool,
) -> SchemaDecision {
    if application_id == 0 && user_version == 0 && empty {
        return SchemaDecision::Create;
    }
    if application_id != crate::authority::APPLICATION_ID {
        return SchemaDecision::Reject(RejectReason::ForeignApplication);
    }
    if user_version > CURRENT_AUTHORITY_SCHEMA {
        return SchemaDecision::Reject(RejectReason::FutureSchema);
    }
    if !(MIN_SUPPORTED_AUTHORITY_SCHEMA..=CURRENT_AUTHORITY_SCHEMA).contains(&user_version) {
        return SchemaDecision::Reject(RejectReason::InvalidVersion);
    }
    if user_version == CURRENT_AUTHORITY_SCHEMA {
        SchemaDecision::Current
    } else {
        SchemaDecision::Migrate { from: user_version }
    }
}
