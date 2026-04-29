//! Workspace records, grants, registry persistence, and workspace utilities.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceRecord {
    pub(crate) id: WorkspaceId,
    pub(crate) kind: WorkspaceKind,
    pub(crate) root: PathBuf,
    pub(crate) aliases: Vec<String>,
    pub(crate) vcs: Option<String>,
    pub(crate) trusted: bool,
    pub(crate) worktree_root: Option<String>,
    pub(crate) artifact_root: Option<String>,
}

impl WorkspaceRecord {
    pub(crate) fn event_payload(self) -> WorkspaceRecordPayload {
        WorkspaceRecordPayload {
            workspace_id: self.id,
            kind: self.kind,
            root: self.root.display().to_string(),
            aliases: self.aliases,
            vcs: self.vcs,
            trusted: self.trusted,
            worktree_root: self.worktree_root,
            artifact_root: self.artifact_root,
        }
    }

    pub(crate) fn into_store(self) -> WorkspaceMetadata {
        WorkspaceMetadata {
            id: self.id.to_string(),
            kind: store_workspace_kind(self.kind),
            root: self.root,
            vcs: store_workspace_vcs(self.vcs.as_deref()),
            owner: None,
            trusted: self.trusted,
            worktree_root: self.worktree_root.map(PathBuf::from),
            artifact_root: self.artifact_root.map(PathBuf::from),
            checkpoint_policy: CheckpointPolicy::Disabled,
            aliases: if self.aliases.is_empty() {
                Vec::new()
            } else {
                vec![WorkspaceAlias {
                    workspace_id: self.id.to_string(),
                    aliases: self.aliases,
                }]
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceGrantRecord {
    pub(crate) grant_id: WorkspaceGrantId,
    pub(crate) agent_id: Option<AgentId>,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) root: PathBuf,
    pub(crate) access: Vec<WorkspaceAccess>,
    pub(crate) created_at: Timestamp,
    pub(crate) expires_at: Option<Timestamp>,
    pub(crate) source: String,
}

impl WorkspaceGrantRecord {
    pub(crate) fn event_payload(self) -> WorkspaceGrantPayload {
        WorkspaceGrantPayload {
            grant_id: self.grant_id,
            agent_id: self.agent_id,
            workspace_id: self.workspace_id,
            root: self.root.display().to_string(),
            access: self.access,
            expires_at: self.expires_at,
            source: self.source,
        }
    }

    pub(crate) fn into_store(self, profile_id: &str) -> StoreWorkspaceGrantRecord {
        StoreWorkspaceGrantRecord {
            grant_id: self.grant_id.to_string(),
            profile_id: profile_id.to_owned(),
            agent_id: self.agent_id,
            workspace_id: self.workspace_id.to_string(),
            root: self.root,
            access: self
                .access
                .into_iter()
                .map(store_workspace_access)
                .collect(),
            created_at: self.created_at,
            expires_at: self.expires_at,
            source: store_grant_source(&self.source),
            reason: None,
        }
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.expires_at
            .as_ref()
            .and_then(|expires_at| DateTime::parse_from_rfc3339(expires_at.as_str()).ok())
            .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedWorkspace {
    pub(crate) root: PathBuf,
}

pub(crate) fn load_workspace_registry(
    profile_home: &ProfileHome,
) -> HashMap<WorkspaceId, WorkspaceRecord> {
    profile_home
        .workspace_registry()
        .load()
        .unwrap_or_default()
        .workspace
        .into_iter()
        .filter_map(workspace_record_from_store)
        .map(|record| (record.id.clone(), record))
        .collect()
}

pub(crate) fn save_workspace_registry(
    profile_home: &ProfileHome,
    workspaces: &HashMap<WorkspaceId, WorkspaceRecord>,
) -> Result<(), cadis_store::StoreError> {
    let mut records = workspaces.values().cloned().collect::<Vec<_>>();
    records.sort_by(|left, right| left.id.cmp(&right.id));
    profile_home.workspace_registry().save(&WorkspaceRegistry {
        workspace: records
            .into_iter()
            .map(WorkspaceRecord::into_store)
            .collect(),
    })
}

pub(crate) fn load_workspace_grants(
    profile_home: &ProfileHome,
    workspaces: &HashMap<WorkspaceId, WorkspaceRecord>,
) -> HashMap<WorkspaceGrantId, WorkspaceGrantRecord> {
    profile_home
        .workspace_grants()
        .load()
        .map(|recovery| recovery.records)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|record| workspace_grant_record_from_store(record, workspaces))
        .map(|record| (record.grant_id.clone(), record))
        .collect()
}

pub(crate) fn workspace_record_from_store(record: WorkspaceMetadata) -> Option<WorkspaceRecord> {
    let root = record.root.canonicalize().ok()?;
    let aliases = record
        .aliases
        .into_iter()
        .filter(|alias| alias.workspace_id == record.id)
        .flat_map(|alias| alias.aliases)
        .collect::<Vec<_>>();
    Some(WorkspaceRecord {
        id: WorkspaceId::from(record.id),
        kind: protocol_workspace_kind(record.kind),
        root,
        aliases: normalize_aliases(aliases),
        vcs: match record.vcs {
            WorkspaceVcs::Git => Some("git".to_owned()),
            WorkspaceVcs::None => None,
        },
        trusted: record.trusted,
        worktree_root: record
            .worktree_root
            .map(|path| path.display().to_string())
            .filter(|value| !value.trim().is_empty()),
        artifact_root: record
            .artifact_root
            .map(|path| path.display().to_string())
            .filter(|value| !value.trim().is_empty()),
    })
}

pub(crate) fn workspace_grant_record_from_store(
    record: StoreWorkspaceGrantRecord,
    workspaces: &HashMap<WorkspaceId, WorkspaceRecord>,
) -> Option<WorkspaceGrantRecord> {
    let workspace_id = WorkspaceId::from(record.workspace_id);
    if !workspaces.contains_key(&workspace_id) {
        return None;
    }
    Some(WorkspaceGrantRecord {
        grant_id: WorkspaceGrantId::from(record.grant_id),
        agent_id: record.agent_id,
        workspace_id,
        root: record.root.canonicalize().unwrap_or(record.root),
        access: normalize_workspace_access(
            record
                .access
                .into_iter()
                .map(protocol_workspace_access)
                .collect(),
        ),
        created_at: record.created_at,
        expires_at: record.expires_at,
        source: protocol_grant_source(record.source).to_owned(),
    })
}

pub(crate) fn next_workspace_grant_counter(
    grants: &HashMap<WorkspaceGrantId, WorkspaceGrantRecord>,
) -> u64 {
    grants
        .keys()
        .filter_map(|grant_id| grant_id.as_str().strip_prefix("grant_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

pub(crate) fn normalize_workspace_access(access: Vec<WorkspaceAccess>) -> Vec<WorkspaceAccess> {
    let mut normalized = if access.is_empty() {
        vec![WorkspaceAccess::Read]
    } else {
        access
    };
    normalized.sort_by_key(|access| match access {
        WorkspaceAccess::Read => 0,
        WorkspaceAccess::Write => 1,
        WorkspaceAccess::Exec => 2,
        WorkspaceAccess::Admin => 3,
    });
    normalized.dedup();
    normalized
}

pub(crate) fn workspace_access_allows(
    granted: &[WorkspaceAccess],
    required: WorkspaceAccess,
) -> bool {
    granted.contains(&WorkspaceAccess::Admin)
        || granted.contains(&required)
        || (required == WorkspaceAccess::Read && granted.contains(&WorkspaceAccess::Write))
}

pub(crate) fn workspace_grant_matches_agent(
    grant_agent_id: Option<&AgentId>,
    request_agent_id: Option<&AgentId>,
) -> bool {
    grant_agent_id.is_none() || grant_agent_id == request_agent_id
}

pub(crate) fn store_workspace_kind(kind: WorkspaceKind) -> StoreWorkspaceKind {
    match kind {
        WorkspaceKind::Project => StoreWorkspaceKind::Project,
        WorkspaceKind::Documents => StoreWorkspaceKind::Documents,
        WorkspaceKind::Sandbox => StoreWorkspaceKind::Sandbox,
        WorkspaceKind::Worktree => StoreWorkspaceKind::Worktree,
    }
}

pub(crate) fn protocol_workspace_kind(kind: StoreWorkspaceKind) -> WorkspaceKind {
    match kind {
        StoreWorkspaceKind::Project => WorkspaceKind::Project,
        StoreWorkspaceKind::Documents => WorkspaceKind::Documents,
        StoreWorkspaceKind::Sandbox => WorkspaceKind::Sandbox,
        StoreWorkspaceKind::Worktree => WorkspaceKind::Worktree,
    }
}

pub(crate) fn store_workspace_access(access: WorkspaceAccess) -> StoreWorkspaceAccess {
    match access {
        WorkspaceAccess::Read => StoreWorkspaceAccess::Read,
        WorkspaceAccess::Write => StoreWorkspaceAccess::Write,
        WorkspaceAccess::Exec => StoreWorkspaceAccess::Exec,
        WorkspaceAccess::Admin => StoreWorkspaceAccess::Admin,
    }
}

pub(crate) fn protocol_workspace_access(access: StoreWorkspaceAccess) -> WorkspaceAccess {
    match access {
        StoreWorkspaceAccess::Read => WorkspaceAccess::Read,
        StoreWorkspaceAccess::Write => WorkspaceAccess::Write,
        StoreWorkspaceAccess::Exec => WorkspaceAccess::Exec,
        StoreWorkspaceAccess::Admin => WorkspaceAccess::Admin,
    }
}

pub(crate) fn store_workspace_vcs(vcs: Option<&str>) -> WorkspaceVcs {
    match vcs.unwrap_or_default().trim().to_lowercase().as_str() {
        "git" => WorkspaceVcs::Git,
        _ => WorkspaceVcs::None,
    }
}

pub(crate) fn store_grant_source(source: &str) -> StoreGrantSource {
    match source.trim().to_lowercase().as_str() {
        "route" => StoreGrantSource::Route,
        "policy" => StoreGrantSource::Policy,
        "worker_spawn" | "worker-spawn" => StoreGrantSource::WorkerSpawn,
        _ => StoreGrantSource::User,
    }
}

pub(crate) fn protocol_grant_source(source: StoreGrantSource) -> &'static str {
    match source {
        StoreGrantSource::Route => "route",
        StoreGrantSource::User => "user",
        StoreGrantSource::Policy => "policy",
        StoreGrantSource::WorkerSpawn => "worker_spawn",
    }
}

pub(crate) fn normalize_aliases(aliases: Vec<String>) -> Vec<String> {
    let mut aliases = aliases
        .into_iter()
        .map(|alias| alias.trim().to_owned())
        .filter(|alias| !alias.is_empty())
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

fn canonical_home_dir() -> Option<PathBuf> {
    let mut candidates = vec![
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    if let (Some(home_drive), Some(home_path)) =
        (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH"))
    {
        let mut joined = PathBuf::from(home_drive);
        joined.push(PathBuf::from(home_path));
        candidates.push(joined);
    }

    candidates
        .into_iter()
        .find_map(|candidate| candidate.canonicalize().ok())
}

fn protected_system_paths() -> Vec<PathBuf> {
    let mut protected = vec![
        PathBuf::from("/etc"),
        PathBuf::from("/dev"),
        PathBuf::from("/proc"),
        PathBuf::from("/sys"),
        PathBuf::from("/run"),
    ];

    if cfg!(windows) {
        let drive = std::env::var_os("SystemDrive")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:"));
        for segment in ["Windows", "Program Files", "Program Files (x86)", "ProgramData"] {
            let mut path = drive.clone();
            path.push(segment);
            protected.push(path);
        }
    }

    protected
        .into_iter()
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect()
}

fn is_protected_system_path(path: &Path) -> bool {
    protected_system_paths()
        .iter()
        .any(|denied| path == denied || path.starts_with(denied))
}

pub(crate) fn canonical_workspace_root(root: &str) -> Result<PathBuf, std::io::Error> {
    let path = if let Some(rest) = root.strip_prefix("~/").or_else(|| root.strip_prefix("~\\")) {
        canonical_home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(root))
    } else {
        PathBuf::from(root)
    };
    path.canonicalize()
}

pub(crate) fn validate_workspace_root(root: &Path, cadis_home: &Path) -> Result<(), ErrorPayload> {
    if root.parent().is_none() {
        return Err(tool_error(
            "workspace_root_too_broad",
            "workspace root cannot be the filesystem root",
            false,
        ));
    }

    if is_protected_system_path(root) {
        return Err(tool_error(
            "workspace_root_denied",
            format!(
                "workspace root {} is a protected system path",
                root.display()
            ),
            false,
        ));
    }

    if let Some(home) = canonical_home_dir() {
        if root == home || home.starts_with(root) {
            return Err(tool_error(
                "workspace_root_too_broad",
                "workspace root cannot be the home directory or an ancestor of it",
                false,
            ));
        }

        for denied in [".ssh", ".aws", ".gnupg", ".config/gh", ".cadis"] {
            let denied = home.join(denied);
            if root.starts_with(&denied) {
                return Err(tool_error(
                    "workspace_root_denied",
                    format!(
                        "workspace root {} is a protected secret path",
                        root.display()
                    ),
                    false,
                ));
            }
        }
    }

    if let Ok(cadis_home) = cadis_home.canonicalize() {
        if root == cadis_home || root.starts_with(&cadis_home) || cadis_home.starts_with(root) {
            return Err(tool_error(
                "workspace_root_denied",
                "workspace root cannot be CADIS_HOME or an ancestor/child of it",
                false,
            ));
        }
    }

    Ok(())
}

pub(crate) fn validate_shell_cwd(cwd: &Path, cadis_home: &Path) -> Result<(), ErrorPayload> {
    if cwd.parent().is_none() {
        return Err(tool_error(
            "shell_cwd_denied",
            "shell.run cwd cannot be the filesystem root",
            false,
        ));
    }

    if is_protected_system_path(cwd) {
        return Err(tool_error(
            "shell_cwd_denied",
            format!("shell.run cwd {} is a protected system path", cwd.display()),
            false,
        ));
    }

    if let Some(home) = canonical_home_dir() {
        if cwd == home || home.starts_with(cwd) {
            return Err(tool_error(
                "shell_cwd_denied",
                "shell.run cwd cannot be the home directory or an ancestor of it",
                false,
            ));
        }

        for denied in [".ssh", ".aws", ".gnupg", ".config/gh", ".cadis"] {
            let denied = home.join(denied);
            if cwd.starts_with(&denied) {
                return Err(tool_error(
                    "shell_cwd_denied",
                    format!("shell.run cwd {} is a protected secret path", cwd.display()),
                    false,
                ));
            }
        }
    }

    if let Ok(cadis_home) = cadis_home.canonicalize() {
        if cwd == cadis_home || cwd.starts_with(&cadis_home) || cadis_home.starts_with(cwd) {
            return Err(tool_error(
                "shell_cwd_denied",
                "shell.run cwd cannot be CADIS_HOME or an ancestor/child of it",
                false,
            ));
        }
    }

    Ok(())
}

pub(crate) fn root_check(name: &str, root: &Path) -> WorkspaceDoctorCheck {
    if root.is_dir() {
        WorkspaceDoctorCheck {
            name: name.to_owned(),
            status: "ok".to_owned(),
            message: format!("{} exists", root.display()),
        }
    } else {
        WorkspaceDoctorCheck {
            name: name.to_owned(),
            status: "error".to_owned(),
            message: format!("{} is not a directory", root.display()),
        }
    }
}

pub(crate) fn project_workspace_metadata_checks(
    workspace: &WorkspaceRecord,
) -> Vec<WorkspaceDoctorCheck> {
    if workspace.kind != WorkspaceKind::Project {
        return Vec::new();
    }

    let mut checks = project_workspace_metadata_checks_for_root(&workspace.root);
    checks.extend(project_worker_worktree_checks_for_root(&workspace.root));
    let Some(metadata) = ProjectWorkspaceStore::new(&workspace.root)
        .load()
        .ok()
        .flatten()
    else {
        return checks;
    };

    if metadata.workspace_id != workspace.id.to_string() {
        checks.push(WorkspaceDoctorCheck {
            name: "workspace.metadata.id".to_owned(),
            status: "error".to_owned(),
            message: format!(
                ".cadis/workspace.toml workspace_id '{}' does not match registry id '{}'",
                metadata.workspace_id, workspace.id
            ),
        });
    }

    let metadata_kind = protocol_workspace_kind(metadata.kind);
    if metadata_kind != workspace.kind {
        checks.push(WorkspaceDoctorCheck {
            name: "workspace.metadata.kind".to_owned(),
            status: "warn".to_owned(),
            message: format!(
                ".cadis/workspace.toml kind {:?} differs from registry kind {:?}",
                metadata_kind, workspace.kind
            ),
        });
    }

    for (name, path) in [
        ("workspace.metadata.worktree_root", metadata.worktree_root),
        ("workspace.metadata.artifact_root", metadata.artifact_root),
        ("workspace.metadata.media_root", metadata.media_root),
    ] {
        if path.is_absolute() {
            checks.push(WorkspaceDoctorCheck {
                name: name.to_owned(),
                status: "warn".to_owned(),
                message: format!("{} should be project-relative", path.display()),
            });
        }
    }

    checks
}

pub(crate) fn project_workspace_metadata_checks_for_root(root: &Path) -> Vec<WorkspaceDoctorCheck> {
    let store = ProjectWorkspaceStore::new(root);
    match store.load() {
        Ok(Some(_)) => vec![WorkspaceDoctorCheck {
            name: "workspace.metadata".to_owned(),
            status: "ok".to_owned(),
            message: format!("{} exists", store.workspace_toml_path().display()),
        }],
        Ok(None) => vec![WorkspaceDoctorCheck {
            name: "workspace.metadata".to_owned(),
            status: "warn".to_owned(),
            message: format!("{} is missing", store.workspace_toml_path().display()),
        }],
        Err(error) => vec![WorkspaceDoctorCheck {
            name: "workspace.metadata".to_owned(),
            status: "error".to_owned(),
            message: format!(
                "could not read {}: {error}",
                store.workspace_toml_path().display()
            ),
        }],
    }
}

pub(crate) fn project_worker_worktree_checks_for_root(root: &Path) -> Vec<WorkspaceDoctorCheck> {
    let store = ProjectWorkspaceStore::new(root);
    match store.worker_worktree_diagnostics() {
        Ok(diagnostics) => diagnostics
            .into_iter()
            .map(project_worktree_diagnostic_check)
            .collect(),
        Err(error) => vec![WorkspaceDoctorCheck {
            name: "workspace.worktrees.metadata".to_owned(),
            status: "error".to_owned(),
            message: format!("could not inspect worker worktree metadata: {error}"),
        }],
    }
}

pub(crate) fn project_worktree_diagnostic_check(
    diagnostic: ProjectWorktreeDiagnostic,
) -> WorkspaceDoctorCheck {
    WorkspaceDoctorCheck {
        name: diagnostic.name,
        status: diagnostic.status,
        message: diagnostic.message,
    }
}
