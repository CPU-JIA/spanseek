use crate::{
    AppError, AppPreferences, AppService, PreferencesState, RecentObject, Result, SessionLayout,
    SessionState, WorkspaceEntry, WorkspaceIdentity, path_from_text, uuid, workspace_availability,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;

impl AppService {
    pub fn preferences(&self) -> Result<PreferencesState> {
        let (version, json) = self.connection.query_row(
            "SELECT state_version,preferences_json FROM app_preferences WHERE singleton=1",
            [],
            |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)),
        )?;
        let preferences: AppPreferences = serde_json::from_str(&json)?;
        validate_preferences(&preferences)?;
        Ok(PreferencesState {
            state_version: version,
            preferences,
        })
    }

    pub fn update_preferences(
        &mut self,
        base_state_version: u32,
        preferences: AppPreferences,
    ) -> Result<PreferencesState> {
        validate_preferences(&preferences)?;
        let version = base_state_version
            .checked_add(1)
            .ok_or(AppError::InvalidInput("state version overflow"))?;
        let changed=self.connection.execute("UPDATE app_preferences SET state_version=?1,preferences_json=?2 WHERE singleton=1 AND state_version=?3",params![version,serde_json::to_string(&preferences)?,base_state_version])?;
        if changed != 1 {
            return Err(AppError::StateConflict);
        }
        Ok(PreferencesState {
            state_version: version,
            preferences,
        })
    }

    pub fn workspaces(&self) -> Result<Vec<WorkspaceEntry>> {
        let mut statement=self.connection.prepare("SELECT local_instance_id,workspace_id,canonical_path,storage_identity,last_opened_at FROM workspace_catalog WHERE show_in_recents=1 ORDER BY last_opened_at DESC,local_instance_id LIMIT 100")?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(instance, workspace, path, expected, last_opened_at)| {
                let availability = workspace_availability(&path_from_text(&path), &expected);
                WorkspaceEntry {
                    identity: WorkspaceIdentity {
                        local_instance_id: instance,
                        workspace_id: workspace,
                    },
                    path,
                    last_opened_at,
                    availability,
                }
            })
            .collect())
    }

    pub fn forget_workspace(&mut self, identity: &WorkspaceIdentity) -> Result<()> {
        self.known_identity(identity)?;
        self.connection.execute("UPDATE workspace_catalog SET show_in_recents=0 WHERE local_instance_id=?1 AND workspace_id=?2",params![identity.local_instance_id,identity.workspace_id])?;
        Ok(())
    }

    pub fn session(&self, identity: &WorkspaceIdentity) -> Result<SessionState> {
        let mut session = self.read_session(identity)?;
        let ids = session
            .layout
            .panes
            .iter()
            .flat_map(|pane| pane.tabs.iter().map(|tab| tab.object_id.clone()))
            .collect::<Vec<_>>();
        remove_deleted_tabs(
            &mut session.layout,
            &self.active_deleted_ids(identity, &ids)?,
        );
        Ok(session)
    }

    fn read_session(&self, identity: &WorkspaceIdentity) -> Result<SessionState> {
        self.known_identity(identity)?;
        let row=self.connection.query_row("SELECT state_version,session_json FROM local_sessions WHERE local_instance_id=?1 AND workspace_id=?2",params![identity.local_instance_id,identity.workspace_id],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,String>(1)?))).optional()?;
        let (state_version, layout) = match row {
            Some((version, json)) => (version, serde_json::from_str(&json)?),
            None => (0, SessionLayout::default()),
        };
        validate_session(&layout)?;
        Ok(SessionState {
            identity: identity.clone(),
            state_version,
            layout,
        })
    }

    pub fn save_session(
        &mut self,
        identity: &WorkspaceIdentity,
        base_state_version: u32,
        mut layout: SessionLayout,
    ) -> Result<SessionState> {
        self.known_identity(identity)?;
        let deleted = self.active_deleted_ids(
            identity,
            &layout
                .panes
                .iter()
                .flat_map(|pane| pane.tabs.iter().map(|tab| tab.object_id.clone()))
                .collect::<Vec<_>>(),
        )?;
        remove_deleted_tabs(&mut layout, &deleted);
        validate_session(&layout)?;
        let version = base_state_version
            .checked_add(1)
            .ok_or(AppError::InvalidInput("state version overflow"))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current:Option<u32>=tx.query_row("SELECT state_version FROM local_sessions WHERE local_instance_id=?1 AND workspace_id=?2",params![identity.local_instance_id,identity.workspace_id],|r|r.get(0)).optional()?;
        if current.unwrap_or(0) != base_state_version {
            return Err(AppError::StateConflict);
        }
        tx.execute("INSERT INTO local_sessions(local_instance_id,workspace_id,state_version,session_json) VALUES(?1,?2,?3,?4) ON CONFLICT(local_instance_id,workspace_id) DO UPDATE SET state_version=excluded.state_version,session_json=excluded.session_json",params![identity.local_instance_id,identity.workspace_id,version,serde_json::to_string(&layout)?])?;
        tx.commit()?;
        Ok(SessionState {
            identity: identity.clone(),
            state_version: version,
            layout,
        })
    }

    pub fn recent_objects(&self, identity: &WorkspaceIdentity) -> Result<Vec<RecentObject>> {
        self.known_identity(identity)?;
        let mut objects=self.connection.prepare("SELECT object_id,last_opened_at FROM recent_objects WHERE local_instance_id=?1 AND workspace_id=?2 ORDER BY last_opened_at DESC,object_id LIMIT 100")?.query_map(params![identity.local_instance_id,identity.workspace_id],|r|Ok(RecentObject{object_id:r.get(0)?,last_opened_at:r.get(1)?}))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let deleted = self.active_deleted_ids(
            identity,
            &objects
                .iter()
                .map(|o| o.object_id.clone())
                .collect::<Vec<_>>(),
        )?;
        objects.retain(|o| !deleted.contains(&o.object_id));
        Ok(objects)
    }

    pub fn delete_trash(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        request: pkms_core::DeleteTrash,
    ) -> Result<pkms_core::TrashCommit> {
        let result = self
            .library_mut(identity)?
            .delete_trash(command_id, request)?;
        let deleted: Vec<_> = result
            .deleted_objects
            .iter()
            .map(|o| o.object_id.clone())
            .collect();
        // Separate app storage cannot join the knowledge transaction. Its cleanup
        // is idempotent and runs again after a lost reply/restart on core replay.
        let session = self.read_session(identity)?;
        let mut layout = session.layout;
        let changed = remove_deleted_tabs(&mut layout, &deleted);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM recent_objects WHERE local_instance_id=?1 AND workspace_id=?2 AND object_id IN (SELECT value FROM json_each(?3))",
            params![identity.local_instance_id,identity.workspace_id,serde_json::to_string(&deleted)?])?;
        if changed {
            let version = session
                .state_version
                .checked_add(1)
                .ok_or(AppError::InvalidInput("state version overflow"))?;
            if tx.execute("UPDATE local_sessions SET state_version=?3,session_json=?4 WHERE local_instance_id=?1 AND workspace_id=?2 AND state_version=?5",
                params![identity.local_instance_id,identity.workspace_id,version,serde_json::to_string(&layout)?,session.state_version])? != 1 {
                return Err(AppError::StateConflict);
            }
        }
        tx.commit()?;
        Ok(result)
    }

    fn active_deleted_ids(
        &self,
        identity: &WorkspaceIdentity,
        ids: &[String],
    ) -> Result<Vec<String>> {
        // Background layout saves for a known inactive workspace retain their
        // old contract. Reopening it sanitizes against that library's tombstones.
        if self
            .active_workspace()
            .is_some_and(|entry| entry.identity == *identity)
        {
            Ok(self.library(identity)?.permanently_deleted_ids(ids)?)
        } else {
            Ok(Vec::new())
        }
    }

    pub fn touch_recent_object(
        &mut self,
        identity: &WorkspaceIdentity,
        object_id: &str,
    ) -> Result<()> {
        uuid(object_id)?;
        if !self.library(identity)?.contains_object(object_id)? {
            return Err(AppError::NotFound);
        }
        self.connection.execute("INSERT INTO recent_objects(local_instance_id,workspace_id,object_id,last_opened_at) VALUES(?1,?2,?3,?4) ON CONFLICT(local_instance_id,workspace_id,object_id) DO UPDATE SET last_opened_at=excluded.last_opened_at",params![identity.local_instance_id,identity.workspace_id,object_id,Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub(crate) fn known_identity(&self, identity: &WorkspaceIdentity) -> Result<()> {
        uuid(&identity.local_instance_id)?;
        uuid(&identity.workspace_id)?;
        let exists:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM workspace_catalog WHERE local_instance_id=?1 AND workspace_id=?2)",params![identity.local_instance_id,identity.workspace_id],|r|r.get(0))?;
        if !exists {
            return Err(AppError::InstanceMismatch);
        }
        Ok(())
    }
}

fn remove_deleted_tabs(layout: &mut SessionLayout, deleted: &[String]) -> bool {
    let deleted: BTreeSet<_> = deleted.iter().map(String::as_str).collect();
    let mut changed = false;
    for pane in &mut layout.panes {
        let before = pane.tabs.len();
        pane.tabs
            .retain(|tab| !deleted.contains(tab.object_id.as_str()));
        pane.scroll_offsets
            .retain(|p| !deleted.contains(p.object_id.as_str()));
        if pane
            .active_object_id
            .as_ref()
            .is_some_and(|id| deleted.contains(id.as_str()))
        {
            pane.active_object_id = pane.tabs.first().map(|tab| tab.object_id.clone());
        }
        changed |= before != pane.tabs.len();
    }
    changed
}

fn validate_preferences(preferences: &AppPreferences) -> Result<()> {
    if !(12..=24).contains(&preferences.ui_font_size)
        || !(14..=32).contains(&preferences.editor_font_size)
        // Matches the zoom levels the workspace offers, so a value the user
        // reaches with Ctrl+wheel can always be stored.
        || !(50..=300).contains(&preferences.zoom_percent)
    {
        return Err(AppError::InvalidInput(
            "display settings exceed supported range",
        ));
    }
    let allowed = [
        "Ctrl+N",
        "Ctrl+Shift+N",
        "Ctrl+K",
        "Ctrl+P",
        "Ctrl+Shift+K",
        "Alt+N",
        "Alt+K",
    ];
    if !allowed.contains(&preferences.new_note_shortcut.as_str())
        || !allowed.contains(&preferences.search_shortcut.as_str())
        || preferences.new_note_shortcut == preferences.search_shortcut
    {
        return Err(AppError::InvalidInput("unsupported or duplicate shortcut"));
    }
    validate_workspace_registry(&preferences.object_type_registry, "object type")?;
    validate_workspace_registry(&preferences.note_template_registry, "note template")?;
    Ok(())
}

fn validate_workspace_registry(value: &str, label: &str) -> Result<()> {
    if value.len() > 65_536 {
        return Err(AppError::InvalidInput(if label == "object type" {
            "object type registry is too large"
        } else {
            "note template registry is too large"
        }));
    }
    let registry: serde_json::Value = serde_json::from_str(value).map_err(|_| {
        AppError::InvalidInput(if label == "object type" {
            "object type registry must be valid JSON"
        } else {
            "note template registry must be valid JSON"
        })
    })?;
    if registry.get("schema").and_then(serde_json::Value::as_u64) != Some(1)
        || !registry
            .get("workspaces")
            .is_some_and(serde_json::Value::is_object)
    {
        return Err(AppError::InvalidInput(if label == "object type" {
            "object type registry must use schema 1 and a workspaces object"
        } else {
            "note template registry must use schema 1 and a workspaces object"
        }));
    }
    Ok(())
}
fn validate_session(layout: &SessionLayout) -> Result<()> {
    if layout
        .sidebar_width
        .is_some_and(|width| !(160..=400).contains(&width))
        || layout
            .note_list_width
            .is_some_and(|width| !(200..=600).contains(&width))
    {
        return Err(AppError::InvalidInput("column width exceeds bounds"));
    }
    if let Some(window) = &layout.window
        && (!(640..=16000).contains(&window.width)
            || !(480..=16000).contains(&window.height)
            || window.left.unsigned_abs() > 100000
            || window.top.unsigned_abs() > 100000)
    {
        return Err(AppError::InvalidInput("window geometry exceeds bounds"));
    }
    if layout.panes.is_empty()
        || layout.panes.len() > 2
        || usize::from(layout.active_pane) >= layout.panes.len()
    {
        return Err(AppError::InvalidInput(
            "layout must contain one or two panes",
        ));
    }
    for pane in &layout.panes {
        if pane.tabs.len() > 50 {
            return Err(AppError::InvalidInput("too many open tabs"));
        }
        let mut ids = BTreeSet::new();
        for tab in &pane.tabs {
            uuid(&tab.object_id)?;
            if !ids.insert(&tab.object_id) {
                return Err(AppError::InvalidInput("duplicate tab in one pane"));
            }
        }
        let mut scroll_ids = BTreeSet::new();
        for position in &pane.scroll_offsets {
            if !ids.contains(&position.object_id)
                || !scroll_ids.insert(&position.object_id)
                || position.x > 10_000_000
                || position.y > 10_000_000
            {
                return Err(AppError::InvalidInput("invalid tab scroll position"));
            }
        }
        if let Some(active) = &pane.active_object_id
            && !ids.contains(active)
        {
            return Err(AppError::InvalidInput("active tab is absent"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_registry_preferences_are_bounded_and_versioned() {
        let mut preferences = AppPreferences::default();
        assert!(validate_preferences(&preferences).is_ok());

        preferences.object_type_registry = "not json".into();
        assert!(validate_preferences(&preferences).is_err());
        preferences.object_type_registry = r#"{"schema":1,"workspaces":{}}"#.into();

        preferences.note_template_registry = "not json".into();
        assert!(validate_preferences(&preferences).is_err());
        preferences.note_template_registry = r#"{"schema":2,"workspaces":{}}"#.into();
        assert!(validate_preferences(&preferences).is_err());
        preferences.note_template_registry = r#"{"schema":1,"workspaces":[]}"#.into();
        assert!(validate_preferences(&preferences).is_err());

        preferences.object_type_registry = r#"{"schema":2,"workspaces":{}}"#.into();
        preferences.note_template_registry = r#"{"schema":1,"workspaces":{}}"#.into();
        assert!(validate_preferences(&preferences).is_err());
        preferences.object_type_registry = r#"{"schema":1,"workspaces":[]}"#.into();
        assert!(validate_preferences(&preferences).is_err());
        preferences.object_type_registry = format!(
            r#"{{"schema":1,"workspaces":{{"workspace":"{}"}}}}"#,
            "x".repeat(65_536)
        );
        assert!(validate_preferences(&preferences).is_err());
        preferences.object_type_registry = r#"{"schema":1,"workspaces":{}}"#.into();
        preferences.note_template_registry = format!(
            r#"{{"schema":1,"workspaces":{{"workspace":"{}"}}}}"#,
            "x".repeat(65_536)
        );
        assert!(validate_preferences(&preferences).is_err());
    }

    #[test]
    fn legacy_preferences_without_registry_receive_an_empty_versioned_store() {
        let legacy = serde_json::json!({
            "language": "zh-CN",
            "theme": "system",
            "ui_font_size": 14,
            "editor_font_size": 17,
            "zoom_percent": 100,
            "new_note_shortcut": "Ctrl+N",
            "search_shortcut": "Ctrl+K"
        });
        let preferences: AppPreferences = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            preferences.object_type_registry,
            r#"{"schema":1,"workspaces":{}}"#
        );
        assert_eq!(
            preferences.note_template_registry,
            r#"{"schema":1,"workspaces":{}}"#
        );
        assert!(validate_preferences(&preferences).is_ok());
    }
}
