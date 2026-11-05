//! Server-authoritative identity, pairing, roles, revocation and audit.
//!
//! This module has no transport, UI or platform vault. Every read and write
//! request must pass [`SyncAuthority::authorize`] before touching synced data.

use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::SyncError;
use crate::crypto::{
    constant_time_eq, fingerprint, hex_encode, normalize_pairing_code, owner_proof_message,
    pairing_proof_message, public_key_bytes, sha256_hex, verify,
};
use crate::model::{
    AuditEntry, DeviceRecord, DeviceStatus, MemberRecord, MemberRole, MemberStatus, PairingInvite,
    SignedRequest,
};

pub(crate) const APPLICATION_ID: i64 = 1_347_112_258;
const REQUEST_WINDOW_SECONDS: i64 = 300;
const NONCE_RETENTION_SECONDS: i64 = 86_400;
const MAX_DISPLAY_NAME_CHARS: usize = 80;
const PAIRING_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const PAIRING_CODE_CHARS: usize = 12;

type PairingInviteRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

pub struct SyncAuthority {
    pub(crate) connection: Connection,
    pub(crate) blob_root: Option<PathBuf>,
}

impl SyncAuthority {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SyncError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                SyncError::UnsupportedDatabase(format!("cannot create directory: {error}"))
            })?;
        }
        let connection = Connection::open(path)?;
        Self::initialize(connection, true, None)
    }

    pub fn open_with_blobs(
        path: impl AsRef<Path>,
        blob_root: impl AsRef<Path>,
    ) -> Result<Self, SyncError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                SyncError::UnsupportedDatabase(format!("cannot create directory: {error}"))
            })?;
        }
        std::fs::create_dir_all(blob_root.as_ref())?;
        let blob_root = blob_root.as_ref().canonicalize()?;
        if !blob_root.is_dir() {
            return Err(SyncError::InvalidBlob(
                "blob root is not a directory".into(),
            ));
        }
        let connection = Connection::open(path)?;
        Self::initialize(connection, true, Some(blob_root))
    }

    pub fn open_in_memory() -> Result<Self, SyncError> {
        Self::initialize(Connection::open_in_memory()?, false, None)
    }

    pub fn open_in_memory_with_blobs(blob_root: impl AsRef<Path>) -> Result<Self, SyncError> {
        std::fs::create_dir_all(blob_root.as_ref())?;
        let blob_root = blob_root.as_ref().canonicalize()?;
        Self::initialize(Connection::open_in_memory()?, false, Some(blob_root))
    }

    fn initialize(
        mut connection: Connection,
        durable_file: bool,
        blob_root: Option<PathBuf>,
    ) -> Result<Self, SyncError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        match crate::compatibility::classify_authority_database(application, version, count == 0) {
            crate::compatibility::SchemaDecision::Create => {
                let tx = connection.transaction()?;
                tx.execute_batch(include_str!("schema.sql"))?;
                tx.commit()?;
            }
            crate::compatibility::SchemaDecision::Migrate { .. }
            | crate::compatibility::SchemaDecision::Current => {}
            crate::compatibility::SchemaDecision::Reject(reason) => {
                return Err(SyncError::UnsupportedDatabase(format!(
                    "{}: application {} version {}",
                    reason.code(),
                    application,
                    version
                )));
            }
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 1 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0002_oplog.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 2 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0003_conflicts.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 3 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0004_trash.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 4 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0005_blobs.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 5 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0006_keys.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 6 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0007_resume.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 7 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0008_member_roles.sql"))?;
            tx.commit()?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 8 {
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("migrations/0009_member_identity.sql"))?;
            tx.commit()?;
        }
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(SyncError::UnsupportedDatabase(format!(
                "quick_check reported {integrity}"
            )));
        }
        if durable_file {
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "FULL")?;
        }
        Ok(Self {
            connection,
            blob_root,
        })
    }

    /// First-device bootstrap. Must be called behind an authenticated account
    /// creation flow once transport exists; the authority itself only allows
    /// this while the registry has never contained a device.
    pub fn enroll_owner(
        &mut self,
        public_key: &[u8],
        display_name: &str,
        proof: &[u8],
    ) -> Result<DeviceRecord, SyncError> {
        self.enroll_owner_at(public_key, display_name, proof, Utc::now())
    }

    pub fn enroll_owner_at(
        &mut self,
        public_key: &[u8],
        display_name: &str,
        proof: &[u8],
        now: DateTime<Utc>,
    ) -> Result<DeviceRecord, SyncError> {
        validate_display_name(display_name)?;
        if public_key.len() != crate::crypto::PUBLIC_KEY_LEN {
            return Err(SyncError::InvalidPublicKey);
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM sync_devices", [], |r| r.get(0))?;
        if count != 0 {
            return Err(SyncError::OwnerAlreadyEnrolled);
        }
        let public_key_hex = hex_encode(public_key);
        verify(
            &public_key_hex,
            &owner_proof_message(&public_key_hex, display_name.trim()),
            proof,
        )?;
        let member_id = uuid::Uuid::new_v4().to_string();
        let device = new_device(
            public_key,
            display_name,
            member_id.clone(),
            display_name.to_owned(),
            MemberRole::Owner,
            now,
        )?;
        let member = new_member(member_id, display_name, MemberRole::Owner, now);
        let tx = self.connection.transaction()?;
        insert_member(&tx, &member, None)?;
        insert_device(&tx, &device)?;
        tx.execute(
            "UPDATE sync_members SET created_by=?1 WHERE member_id=?2",
            params![device.device_id, member.member_id],
        )?;
        audit_on(
            &tx,
            "owner_enrolled",
            Some(&device.device_id),
            Some(&device.device_id),
            json!({
                "display_name": device.display_name,
                "fingerprint": device.fingerprint,
                "member_id": member.member_id,
                "member_name": member.display_name
            }),
        )?;
        tx.commit()?;
        Ok(device)
    }

    pub fn create_pairing_invite(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
    ) -> Result<PairingInvite, SyncError> {
        self.create_pairing_invite_with_role(actor_device_id, ttl_seconds, MemberRole::Editor)
    }

    pub fn create_pairing_invite_at(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
        now: DateTime<Utc>,
    ) -> Result<PairingInvite, SyncError> {
        self.create_pairing_invite_with_role_at(
            actor_device_id,
            ttl_seconds,
            MemberRole::Editor,
            now,
        )
    }

    pub fn create_pairing_invite_with_role(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
        role: MemberRole,
    ) -> Result<PairingInvite, SyncError> {
        self.create_pairing_invite_with_role_at(actor_device_id, ttl_seconds, role, Utc::now())
    }

    pub fn create_pairing_invite_with_role_at(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
        role: MemberRole,
        now: DateTime<Utc>,
    ) -> Result<PairingInvite, SyncError> {
        self.create_pairing_invite_target_at(
            actor_device_id,
            ttl_seconds,
            Some(role),
            None,
            None,
            now,
        )
    }

    pub fn create_member_pairing_invite(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
        member_name: &str,
        role: MemberRole,
    ) -> Result<PairingInvite, SyncError> {
        self.create_pairing_invite_target_at(
            actor_device_id,
            ttl_seconds,
            Some(role),
            None,
            Some(member_name),
            Utc::now(),
        )
    }

    pub fn create_device_pairing_invite(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
        member_id: &str,
    ) -> Result<PairingInvite, SyncError> {
        self.create_pairing_invite_target_at(
            actor_device_id,
            ttl_seconds,
            None,
            Some(member_id),
            None,
            Utc::now(),
        )
    }

    fn create_pairing_invite_target_at(
        &mut self,
        actor_device_id: &str,
        ttl_seconds: i64,
        role: Option<MemberRole>,
        target_member_id: Option<&str>,
        member_name: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<PairingInvite, SyncError> {
        if !(1..=3_600).contains(&ttl_seconds) {
            return Err(SyncError::InvalidLifetime);
        }
        let actor = self.device(actor_device_id)?;
        require_owner(&actor)?;
        let (role, target_member_id, member_name) = if let Some(member_id) = target_member_id {
            let member = self.member(member_id)?;
            if member.status != crate::MemberStatus::Active {
                return Err(SyncError::MemberRemoved);
            }
            (
                member.role,
                Some(member.member_id),
                Some(member.display_name),
            )
        } else {
            let role = role.ok_or_else(|| SyncError::InvalidRole("missing".into()))?;
            if let Some(name) = member_name {
                validate_display_name(name)?;
            }
            (role, None, member_name.map(|name| name.trim().to_owned()))
        };
        let code = generate_pairing_code()?;
        let invite_id = uuid::Uuid::new_v4().to_string();
        let created_at = format_time(now);
        let expires_at = format_time(now + chrono::Duration::seconds(ttl_seconds));
        let code_hash = pairing_code_hash(&invite_id, &code);
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM sync_pairing_invites WHERE consumed_at IS NULL AND expires_at<?1",
            [&created_at],
        )?;
        tx.execute(
            "INSERT INTO sync_pairing_invites(invite_id,code_hash,created_by,created_at,expires_at,invited_role,invited_member_id,invited_member_name) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                invite_id,
                code_hash,
                actor_device_id,
                created_at,
                expires_at,
                role.as_str(),
                target_member_id,
                member_name
            ],
        )?;
        audit_on(
            &tx,
            "pairing_invite_created",
            Some(actor_device_id),
            None,
            json!({
                "invite_id": invite_id,
                "expires_at": expires_at,
                "role": role.as_str(),
                "member_id": target_member_id,
                "member_name": member_name
            }),
        )?;
        tx.commit()?;
        Ok(PairingInvite {
            invite_id,
            code: format_pairing_code(&code),
            created_by: actor_device_id.to_owned(),
            member_id: target_member_id,
            member_name,
            role,
            created_at,
            expires_at,
        })
    }
    pub fn complete_pairing(
        &mut self,
        invite_id: &str,
        code: &str,
        public_key: &[u8],
        display_name: &str,
        proof: &[u8],
    ) -> Result<DeviceRecord, SyncError> {
        self.complete_pairing_at(invite_id, code, public_key, display_name, proof, Utc::now())
    }

    pub fn complete_pairing_at(
        &mut self,
        invite_id: &str,
        code: &str,
        public_key: &[u8],
        display_name: &str,
        proof: &[u8],
        now: DateTime<Utc>,
    ) -> Result<DeviceRecord, SyncError> {
        validate_display_name(display_name)?;
        if public_key.len() != crate::crypto::PUBLIC_KEY_LEN {
            return Err(SyncError::InvalidPublicKey);
        }
        let normalized = normalize_pairing_code(code);
        if normalized.len() != PAIRING_CODE_CHARS
            || !normalized
                .bytes()
                .all(|byte| PAIRING_ALPHABET.contains(&byte))
        {
            return Err(SyncError::InvalidPairingCode);
        }
        let row: Option<PairingInviteRow> = self
            .connection
            .query_row(
                "SELECT code_hash,created_by,expires_at,consumed_at,invited_role,invited_member_id,invited_member_name FROM sync_pairing_invites WHERE invite_id=?1",
                [invite_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        let (
            code_hash,
            created_by,
            expires_at,
            consumed_at,
            invited_role,
            invited_member_id,
            invited_member_name,
        ) = row.ok_or(SyncError::InviteNotFound)?;
        if consumed_at.is_some() {
            return Err(SyncError::InviteConsumed);
        }
        let expires_at = parse_time(&expires_at)?;
        if now >= expires_at {
            return Err(SyncError::InviteExpired);
        }
        if !constant_time_eq(
            code_hash.as_bytes(),
            pairing_code_hash(invite_id, &normalized).as_bytes(),
        ) {
            return Err(SyncError::InvalidPairingCode);
        }
        require_active(&self.device(&created_by)?)?;
        let public_key_hex = hex_encode(public_key);
        verify(
            &public_key_hex,
            &pairing_proof_message(invite_id, &normalized, &public_key_hex),
            proof,
        )?;
        let duplicate: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_devices WHERE public_key=?1)",
            [&public_key_hex],
            |r| r.get(0),
        )?;
        if duplicate {
            return Err(SyncError::DuplicateDevice);
        }
        let invited_role = MemberRole::parse(&invited_role)?;
        let (role, member_id, member_name, create_member) = if let Some(member_id) =
            invited_member_id
        {
            let member = self.member(&member_id)?;
            if member.status != crate::MemberStatus::Active {
                return Err(SyncError::MemberRemoved);
            }
            (member.role, member.member_id, member.display_name, false)
        } else {
            let member_name = invited_member_name.unwrap_or_else(|| display_name.trim().to_owned());
            validate_display_name(&member_name)?;
            (
                invited_role,
                uuid::Uuid::new_v4().to_string(),
                member_name,
                true,
            )
        };
        let device = new_device(
            public_key,
            display_name,
            member_id.clone(),
            member_name.clone(),
            role,
            now,
        )?;
        let tx = self.connection.transaction()?;
        if create_member {
            let member = new_member(member_id.clone(), &member_name, role, now);
            insert_member(&tx, &member, Some(&created_by))?;
        }
        insert_device(&tx, &device)?;
        let changed = tx.execute(
            "UPDATE sync_pairing_invites SET consumed_at=?1,consumed_by=?2 WHERE invite_id=?3 AND consumed_at IS NULL",
            params![format_time(now), device.device_id, invite_id],
        )?;
        if changed != 1 {
            return Err(SyncError::InviteConsumed);
        }
        audit_on(
            &tx,
            "device_paired",
            Some(&created_by),
            Some(&device.device_id),
            json!({
                "invite_id": invite_id,
                "display_name": device.display_name,
                "fingerprint": device.fingerprint,
                "role": role.as_str(),
                "member_id": member_id,
                "member_name": member_name
            }),
        )?;
        tx.commit()?;
        Ok(device)
    }

    pub fn revoke_device(
        &mut self,
        actor_device_id: &str,
        target_device_id: &str,
        reason: &str,
    ) -> Result<DeviceRecord, SyncError> {
        self.revoke_device_at(actor_device_id, target_device_id, reason, Utc::now())
    }

    pub fn revoke_device_at(
        &mut self,
        actor_device_id: &str,
        target_device_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<DeviceRecord, SyncError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > 240 {
            return Err(SyncError::InvalidRevocationReason);
        }
        let actor = self.device(actor_device_id)?;
        require_owner(&actor)?;
        let target = self.device(target_device_id)?;
        if target.status == DeviceStatus::Revoked {
            return Err(SyncError::DeviceRevoked);
        }
        let active: i64 = self.connection.query_row(
            "SELECT count(*) FROM sync_devices WHERE status='active'",
            [],
            |r| r.get(0),
        )?;
        if active <= 1 {
            audit_on(
                &self.connection,
                "revoke_refused_last_active",
                Some(actor_device_id),
                Some(target_device_id),
                json!({"reason": reason}),
            )?;
            return Err(SyncError::LastActiveDevice);
        }
        if target.role == MemberRole::Owner {
            let owners: i64 = self.connection.query_row(
                "SELECT count(*) FROM sync_devices d JOIN sync_members m ON m.member_id=d.member_id WHERE d.status='active' AND m.status='active' AND m.role='owner'",
                [],
                |r| r.get(0),
            )?;
            if owners <= 1 {
                audit_on(
                    &self.connection,
                    "revoke_refused_last_owner",
                    Some(actor_device_id),
                    Some(target_device_id),
                    json!({"reason": reason}),
                )?;
                return Err(SyncError::LastOwner);
            }
        }
        let revoked_at = format_time(now);
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE sync_devices SET status='revoked',revoked_at=?1,revoked_by=?2,revoked_reason=?3 WHERE device_id=?4 AND status='active'",
            params![revoked_at, actor_device_id, reason, target_device_id],
        )?;
        tx.execute(
            "DELETE FROM sync_key_envelopes WHERE device_id=?1",
            [target_device_id],
        )?;
        tx.execute(
            "DELETE FROM sync_device_encryption_keys WHERE device_id=?1",
            [target_device_id],
        )?;
        audit_on(
            &tx,
            "device_revoked",
            Some(actor_device_id),
            Some(target_device_id),
            json!({"reason": reason, "revoked_at": revoked_at, "key_material_deleted": true}),
        )?;
        tx.commit()?;
        self.device(target_device_id)
    }

    pub fn set_device_role(
        &mut self,
        actor_device_id: &str,
        target_device_id: &str,
        role: MemberRole,
    ) -> Result<DeviceRecord, SyncError> {
        let target = self.device(target_device_id)?;
        self.set_member_role(actor_device_id, &target.member_id, role)?;
        self.device(target_device_id)
    }

    pub fn set_member_role(
        &mut self,
        actor_device_id: &str,
        member_id: &str,
        role: MemberRole,
    ) -> Result<MemberRecord, SyncError> {
        let actor = self.device(actor_device_id)?;
        require_owner(&actor)?;
        let target = self.member(member_id)?;
        if target.status != crate::MemberStatus::Active {
            return Err(SyncError::MemberRemoved);
        }
        if target.role == role {
            return Ok(target);
        }
        if target.role == MemberRole::Owner && role != MemberRole::Owner {
            let other_owners: i64 = self.connection.query_row(
                "SELECT count(*) FROM sync_devices d JOIN sync_members m ON m.member_id=d.member_id WHERE d.status='active' AND m.status='active' AND m.role='owner' AND m.member_id<>?1",
                [member_id],
                |r| r.get(0),
            )?;
            if other_owners == 0 {
                return Err(SyncError::LastOwner);
            }
        }
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE sync_members SET role=?1 WHERE member_id=?2 AND status='active'",
            params![role.as_str(), member_id],
        )?;
        tx.execute(
            "UPDATE sync_devices SET role=?1 WHERE member_id=?2",
            params![role.as_str(), member_id],
        )?;
        audit_on(
            &tx,
            "member_role_changed",
            Some(actor_device_id),
            None,
            json!({
                "member_id": member_id,
                "from": target.role.as_str(),
                "to": role.as_str()
            }),
        )?;
        tx.commit()?;
        self.member(member_id)
    }

    pub fn remove_member(
        &mut self,
        actor_device_id: &str,
        member_id: &str,
        reason: &str,
    ) -> Result<MemberRecord, SyncError> {
        self.remove_member_at(actor_device_id, member_id, reason, Utc::now())
    }

    pub fn remove_member_at(
        &mut self,
        actor_device_id: &str,
        member_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<MemberRecord, SyncError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > 240 {
            return Err(SyncError::InvalidRevocationReason);
        }
        let actor = self.device(actor_device_id)?;
        require_owner(&actor)?;
        let target = self.member(member_id)?;
        if target.status != crate::MemberStatus::Active {
            return Err(SyncError::MemberRemoved);
        }
        if target.role == MemberRole::Owner {
            let other_owners: i64 = self.connection.query_row(
                "SELECT count(*) FROM sync_devices d JOIN sync_members m ON m.member_id=d.member_id WHERE d.status='active' AND m.status='active' AND m.role='owner' AND m.member_id<>?1",
                [member_id],
                |r| r.get(0),
            )?;
            if other_owners == 0 {
                return Err(SyncError::LastOwner);
            }
        }
        let removed_at = format_time(now);
        let device_ids = self
            .connection
            .prepare("SELECT device_id FROM sync_devices WHERE member_id=?1 AND status='active'")?
            .query_map([member_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE sync_members SET status='removed',removed_at=?1,removed_by=?2,removed_reason=?3 WHERE member_id=?4 AND status='active'",
            params![removed_at, actor_device_id, reason, member_id],
        )?;
        tx.execute(
            "UPDATE sync_devices SET status='revoked',revoked_at=?1,revoked_by=?2,revoked_reason=?3 WHERE member_id=?4 AND status='active'",
            params![removed_at, actor_device_id, reason, member_id],
        )?;
        tx.execute(
            "DELETE FROM sync_key_envelopes WHERE device_id IN (SELECT device_id FROM sync_devices WHERE member_id=?1)",
            [member_id],
        )?;
        tx.execute(
            "DELETE FROM sync_device_encryption_keys WHERE device_id IN (SELECT device_id FROM sync_devices WHERE member_id=?1)",
            [member_id],
        )?;
        audit_on(
            &tx,
            "member_removed",
            Some(actor_device_id),
            None,
            json!({
                "member_id": member_id,
                "display_name": target.display_name,
                "reason": reason,
                "device_ids": device_ids,
                "key_material_deleted": true
            }),
        )?;
        tx.commit()?;
        self.member(member_id)
    }
    /// Single gate for synced reads and writes. The caller must verify the
    /// returned record before releasing plaintext or accepting an op.
    pub fn authorize(
        &mut self,
        request: &SignedRequest,
        now_unix: i64,
    ) -> Result<DeviceRecord, SyncError> {
        self.verify_device_request(request, now_unix)?;
        self.consume_request_nonce(request, now_unix)?;
        self.touch_device(&request.device_id, now_unix)?;
        self.device(&request.device_id)
    }

    /// Status, signature and time window only. Used by idempotent op submission
    /// before nonce consumption, so a retry of an already-applied op succeeds.
    pub(crate) fn verify_device_request(
        &self,
        request: &SignedRequest,
        now_unix: i64,
    ) -> Result<DeviceRecord, SyncError> {
        self.verify_device_request_inner(request, now_unix, true)
    }

    /// Device-local metadata such as encryption-key registration and reading
    /// progress is writable by viewers too. Content and membership writes still
    /// go through `verify_device_request` and its role gate.
    pub(crate) fn verify_device_request_without_role_check(
        &self,
        request: &SignedRequest,
        now_unix: i64,
    ) -> Result<DeviceRecord, SyncError> {
        self.verify_device_request_inner(request, now_unix, false)
    }

    fn verify_device_request_inner(
        &self,
        request: &SignedRequest,
        now_unix: i64,
        enforce_write_role: bool,
    ) -> Result<DeviceRecord, SyncError> {
        if !valid_nonce(&request.nonce) {
            return Err(SyncError::InvalidNonce);
        }
        let device = self.device(&request.device_id)?;
        if device.status == DeviceStatus::Revoked {
            audit_on(
                &self.connection,
                "access_denied_revoked",
                Some(&request.device_id),
                Some(&request.device_id),
                json!({"access": request.access.as_str(), "nonce": request.nonce}),
            )?;
            return Err(SyncError::DeviceRevoked);
        }
        let message = crate::crypto::access_message(
            &request.device_id,
            request.access,
            &request.nonce,
            request.issued_at,
            &request.request_hash,
        );
        if verify(&device.public_key, &message, &request.signature).is_err() {
            audit_on(
                &self.connection,
                "access_denied_invalid_signature",
                Some(&request.device_id),
                Some(&request.device_id),
                json!({"access": request.access.as_str()}),
            )?;
            return Err(SyncError::InvalidSignature);
        }
        if (now_unix - request.issued_at).abs() > REQUEST_WINDOW_SECONDS {
            return Err(SyncError::StaleRequest);
        }
        if enforce_write_role
            && request.access == crate::SyncAccess::Write
            && !device.role.can_write()
        {
            audit_on(
                &self.connection,
                "access_denied_role",
                Some(&request.device_id),
                Some(&request.device_id),
                json!({"access": request.access.as_str(), "role": device.role.as_str()}),
            )?;
            return Err(SyncError::InsufficientRole);
        }
        Ok(device)
    }

    pub(crate) fn consume_request_nonce(
        &self,
        request: &SignedRequest,
        now_unix: i64,
    ) -> Result<(), SyncError> {
        self.connection.execute(
            "DELETE FROM sync_request_nonces WHERE seen_at<?1",
            [now_unix - NONCE_RETENTION_SECONDS],
        )?;
        let inserted = self.connection.execute(
            "INSERT INTO sync_request_nonces(device_id,nonce,seen_at) VALUES(?1,?2,?3)",
            params![request.device_id, request.nonce, now_unix],
        );
        if let Err(rusqlite::Error::SqliteFailure(error, _)) = &inserted
            && error.code == rusqlite::ErrorCode::ConstraintViolation
        {
            audit_on(
                &self.connection,
                "access_denied_replayed",
                Some(&request.device_id),
                Some(&request.device_id),
                json!({"access": request.access.as_str(), "nonce": request.nonce}),
            )?;
            return Err(SyncError::ReplayedRequest);
        }
        inserted?;
        Ok(())
    }

    pub(crate) fn touch_device(&self, device_id: &str, now_unix: i64) -> Result<(), SyncError> {
        self.connection.execute(
            "UPDATE sync_devices SET last_seen_at=?1 WHERE device_id=?2",
            params![format_seconds(now_unix)?, device_id],
        )?;
        Ok(())
    }

    /// Advisory server clock for client skew correction. It is intentionally
    /// unsigned: a client only needs it to align its own request timestamp.
    pub fn server_time_unix(&self) -> i64 {
        Utc::now().timestamp()
    }
    pub fn schema_version(&self) -> Result<i64, SyncError> {
        self.connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(SyncError::from)
    }
    pub fn device(&self, device_id: &str) -> Result<DeviceRecord, SyncError> {
        self.connection
            .query_row(
                "SELECT d.device_id,d.display_name,d.public_key,d.fingerprint,d.status,d.paired_at,d.last_seen_at,d.revoked_at,d.revoked_by,d.revoked_reason,m.role,d.member_id,m.display_name FROM sync_devices d JOIN sync_members m ON m.member_id=d.member_id WHERE d.device_id=?1",
                [device_id],
                device_from_row,
            )
            .optional()?
            .ok_or(SyncError::DeviceNotFound)
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceRecord>, SyncError> {
        let mut statement = self.connection.prepare(
            "SELECT d.device_id,d.display_name,d.public_key,d.fingerprint,d.status,d.paired_at,d.last_seen_at,d.revoked_at,d.revoked_by,d.revoked_reason,m.role,d.member_id,m.display_name FROM sync_devices d JOIN sync_members m ON m.member_id=d.member_id ORDER BY CASE d.status WHEN 'active' THEN 0 ELSE 1 END,d.paired_at,d.device_id",
        )?;
        let rows = statement
            .query_map([], device_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn member(&self, member_id: &str) -> Result<MemberRecord, SyncError> {
        self.connection
            .query_row(
                "SELECT m.member_id,m.display_name,m.role,m.status,m.created_at,m.removed_at,m.removed_by,m.removed_reason,(SELECT count(*) FROM sync_devices d WHERE d.member_id=m.member_id AND d.status='active') FROM sync_members m WHERE m.member_id=?1",
                [member_id],
                member_from_row,
            )
            .optional()?
            .ok_or(SyncError::MemberNotFound)
    }

    pub fn list_members(&self) -> Result<Vec<MemberRecord>, SyncError> {
        let mut statement = self.connection.prepare(
            "SELECT m.member_id,m.display_name,m.role,m.status,m.created_at,m.removed_at,m.removed_by,m.removed_reason,(SELECT count(*) FROM sync_devices d WHERE d.member_id=m.member_id AND d.status='active') FROM sync_members m ORDER BY CASE m.status WHEN 'active' THEN 0 ELSE 1 END,m.created_at,m.member_id",
        )?;
        let rows = statement
            .query_map([], member_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn audit_log(&self, limit: usize) -> Result<Vec<AuditEntry>, SyncError> {
        let limit = limit.clamp(1, 500) as i64;
        let mut statement = self.connection.prepare(
            "SELECT seq,at,action,actor_device_id,target_device_id,detail_json FROM sync_device_audit ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit], |row| {
                let detail: String = row.get(5)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    detail,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(seq, at, action, actor, target, detail)| {
                Ok(AuditEntry {
                    seq,
                    at,
                    action,
                    actor_device_id: actor,
                    target_device_id: target,
                    detail: serde_json::from_str(&detail)?,
                })
            })
            .collect()
    }
}

fn new_device(
    public_key: &[u8],
    display_name: &str,
    member_id: String,
    member_name: String,
    role: MemberRole,
    now: DateTime<Utc>,
) -> Result<DeviceRecord, SyncError> {
    let public_key_hex = hex_encode(public_key);
    Ok(DeviceRecord {
        device_id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.trim().to_owned(),
        public_key: public_key_hex.clone(),
        fingerprint: fingerprint(public_key),
        member_id,
        member_name,
        role,
        status: DeviceStatus::Active,
        paired_at: format_time(now),
        last_seen_at: None,
        revoked_at: None,
        revoked_by: None,
        revoked_reason: None,
    })
}

fn insert_device(connection: &Connection, device: &DeviceRecord) -> Result<(), SyncError> {
    connection.execute(
        "INSERT INTO sync_devices(device_id,display_name,public_key,fingerprint,status,paired_at,role,member_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            device.device_id,
            device.display_name,
            device.public_key,
            device.fingerprint,
            device.status.as_str(),
            device.paired_at,
            device.role.as_str(),
            device.member_id
        ],
    )?;
    Ok(())
}

fn device_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRecord> {
    let status: String = row.get(4)?;
    let role: String = row.get(10)?;
    Ok(DeviceRecord {
        device_id: row.get(0)?,
        display_name: row.get(1)?,
        public_key: row.get(2)?,
        fingerprint: row.get(3)?,
        member_id: row.get(11)?,
        member_name: row.get(12)?,
        role: MemberRole::parse(&role).map_err(|_| {
            rusqlite::Error::InvalidColumnType(10, role, rusqlite::types::Type::Text)
        })?,
        status: DeviceStatus::parse(&status).map_err(|_| {
            rusqlite::Error::InvalidColumnType(4, status, rusqlite::types::Type::Text)
        })?,
        paired_at: row.get(5)?,
        last_seen_at: row.get(6)?,
        revoked_at: row.get(7)?,
        revoked_by: row.get(8)?,
        revoked_reason: row.get(9)?,
    })
}

fn new_member(
    member_id: String,
    display_name: &str,
    role: MemberRole,
    now: DateTime<Utc>,
) -> MemberRecord {
    MemberRecord {
        member_id,
        display_name: display_name.trim().to_owned(),
        role,
        status: MemberStatus::Active,
        created_at: format_time(now),
        removed_at: None,
        removed_by: None,
        removed_reason: None,
        device_count: 0,
    }
}

fn insert_member(
    connection: &Connection,
    member: &MemberRecord,
    created_by: Option<&str>,
) -> Result<(), SyncError> {
    connection.execute(
        "INSERT INTO sync_members(member_id,display_name,role,status,created_at,created_by) VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            member.member_id,
            member.display_name,
            member.role.as_str(),
            member.status.as_str(),
            member.created_at,
            created_by
        ],
    )?;
    Ok(())
}

fn member_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemberRecord> {
    let role: String = row.get(2)?;
    let status: String = row.get(3)?;
    Ok(MemberRecord {
        member_id: row.get(0)?,
        display_name: row.get(1)?,
        role: MemberRole::parse(&role).map_err(|_| {
            rusqlite::Error::InvalidColumnType(2, role, rusqlite::types::Type::Text)
        })?,
        status: MemberStatus::parse(&status).map_err(|_| {
            rusqlite::Error::InvalidColumnType(3, status, rusqlite::types::Type::Text)
        })?,
        created_at: row.get(4)?,
        removed_at: row.get(5)?,
        removed_by: row.get(6)?,
        removed_reason: row.get(7)?,
        device_count: row.get(8)?,
    })
}
fn require_active(device: &DeviceRecord) -> Result<(), SyncError> {
    match device.status {
        DeviceStatus::Active => Ok(()),
        DeviceStatus::Revoked => Err(SyncError::DeviceRevoked),
    }
}

pub(crate) fn require_owner(device: &DeviceRecord) -> Result<(), SyncError> {
    require_active(device)?;
    if !device.role.can_administer() {
        return Err(SyncError::InsufficientRole);
    }
    Ok(())
}

fn validate_display_name(display_name: &str) -> Result<(), SyncError> {
    let trimmed = display_name.trim();
    let count = trimmed.chars().count();
    if count == 0 || count > MAX_DISPLAY_NAME_CHARS || trimmed.chars().any(char::is_control) {
        return Err(SyncError::InvalidDisplayName);
    }
    Ok(())
}

fn valid_nonce(nonce: &str) -> bool {
    (16..=128).contains(&nonce.len())
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn generate_pairing_code() -> Result<String, SyncError> {
    let mut bytes = [0u8; PAIRING_CODE_CHARS];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SyncError::RandomUnavailable)?;
    Ok(bytes
        .iter()
        .map(|byte| PAIRING_ALPHABET[*byte as usize % PAIRING_ALPHABET.len()] as char)
        .collect())
}

fn pairing_code_hash(invite_id: &str, normalized_code: &str) -> String {
    sha256_hex(format!("spanseek-sync-invite-v1\u{0}{invite_id}\u{0}{normalized_code}").as_bytes())
}

fn format_pairing_code(code: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = code;
    while rest.len() > 4 {
        parts.push(&rest[..4]);
        rest = &rest[4..];
    }
    parts.push(rest);
    parts.join("-")
}

fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn format_seconds(unix: i64) -> Result<String, SyncError> {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(format_time)
        .ok_or_else(|| SyncError::UnsupportedDatabase(format!("invalid unix time {unix}")))
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, SyncError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| SyncError::UnsupportedDatabase(format!("invalid timestamp {value}")))
}

pub(crate) fn audit_on(
    connection: &Connection,
    action: &str,
    actor: Option<&str>,
    target: Option<&str>,
    detail: serde_json::Value,
) -> Result<(), SyncError> {
    connection.execute(
        "INSERT INTO sync_device_audit(at,action,actor_device_id,target_device_id,detail_json) VALUES(?1,?2,?3,?4,?5)",
        params![format_time(Utc::now()), action, actor, target, serde_json::to_string(&detail)?],
    )?;
    Ok(())
}

/// Verifies that a public key string is a valid Ed25519 key without registering it.
pub fn validate_public_key(public_key_hex: &str) -> Result<(), SyncError> {
    public_key_bytes(public_key_hex).map(|_| ())
}
