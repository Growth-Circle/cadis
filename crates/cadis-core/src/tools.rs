//! Tool registry, definitions, execution types, and file-patch utilities.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolRegistry {
    pub(crate) definitions: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub(crate) fn new(definitions: Vec<ToolDefinition>) -> Result<Self, RuntimeError> {
        for (index, definition) in definitions.iter().enumerate() {
            if definitions[..index]
                .iter()
                .any(|previous| previous.name == definition.name)
            {
                return Err(RuntimeError {
                    code: "duplicate_tool_name",
                    message: format!("tool '{}' is registered more than once", definition.name),
                });
            }
            if definition.description.trim().is_empty() {
                return Err(RuntimeError {
                    code: "invalid_tool_description",
                    message: format!("tool '{}' is missing a description", definition.name),
                });
            }
            if definition.side_effects.is_empty() {
                return Err(RuntimeError {
                    code: "invalid_tool_side_effects",
                    message: format!(
                        "tool '{}' must declare at least one side effect",
                        definition.name
                    ),
                });
            }
            if definition.timeout_secs == 0 {
                return Err(RuntimeError {
                    code: "invalid_tool_timeout",
                    message: format!("tool '{}' must declare a positive timeout", definition.name),
                });
            }
        }

        Ok(Self { definitions })
    }

    pub(crate) fn builtin() -> Result<Self, RuntimeError> {
        Self::new(vec![
            ToolDefinition::safe_read(
                "file.read",
                "Read one file inside an approved workspace",
                ToolInputSchema::FileRead,
                &[ToolSideEffect::ReadFiles],
                5,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::safe_read(
                "file.search",
                "Search text files inside an approved workspace",
                ToolInputSchema::FileSearch,
                &[ToolSideEffect::SearchFiles],
                10,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::safe_read(
                "git.status",
                "Read git status inside an approved workspace",
                ToolInputSchema::GitStatus,
                &[ToolSideEffect::ReadGitMetadata],
                10,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::safe_read(
                "file.list",
                "List directory contents inside an approved workspace",
                ToolInputSchema::FileList,
                &[ToolSideEffect::ListFiles],
                5,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::safe_read(
                "git.log",
                "Read git log inside an approved workspace",
                ToolInputSchema::GitLog,
                &[ToolSideEffect::ReadGitLog],
                10,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::approval_placeholder(
                "file.write",
                "Write or replace files inside an approved workspace",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::WorkspaceMutation,
                &[ToolSideEffect::EditWorkspace],
                30,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::PathScoped,
                false,
                false,
            ),
            ToolDefinition::approval_placeholder(
                "file.patch",
                "Apply a patch inside an approved workspace",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::WorkspaceMutation,
                &[ToolSideEffect::EditWorkspace],
                30,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::PathScoped,
                false,
                false,
            ),
            ToolDefinition::approval_placeholder(
                "worker.apply",
                "Apply a daemon-owned worker patch to a registered workspace",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::WorkspaceMutation,
                &[ToolSideEffect::EditWorkspace],
                60,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::PathScoped,
                false,
                false,
            ),
            ToolDefinition::safe_read(
                "git.diff",
                "Read git diff output for an approved workspace",
                ToolInputSchema::GitDiff,
                &[ToolSideEffect::ReadGitDiff],
                20,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::approval_placeholder(
                "git.worktree.create",
                "Create a CADIS-managed git worktree",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::GitMutation,
                &[ToolSideEffect::CreateWorktree],
                60,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::CadisManagedWorktree,
                false,
                false,
            ),
            ToolDefinition::approval_placeholder(
                "git.worktree.remove",
                "Remove a CADIS-managed git worktree",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::GitMutation,
                &[ToolSideEffect::RemoveWorktree],
                60,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::CadisManagedWorktree,
                false,
                false,
            ),
            ToolDefinition::approval_placeholder(
                "shell.run",
                "Run a local shell command in an approved workspace",
                cadis_protocol::RiskClass::SystemChange,
                ToolInputSchema::ShellRun,
                &[ToolSideEffect::RunSubprocess],
                900,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::RequiresWorkspace,
                false,
                true,
            ),
            ToolDefinition::approval_placeholder(
                "git.commit",
                "Stage and commit changes inside an approved workspace",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::GitMutation,
                &[ToolSideEffect::GitCommit],
                30,
                ToolCancellationBehavior::Cooperative,
                ToolWorkspaceBehavior::PathScoped,
                false,
                false,
            ),
        ])
    }

    pub(crate) fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.name == name)
    }

    pub(crate) fn is_auto_executable_safe_read(&self, name: &str) -> bool {
        self.get(name)
            .is_some_and(|definition| definition.execution == ToolExecutionMode::AutoExecute)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolDefinition {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) risk_class: cadis_protocol::RiskClass,
    pub(crate) input_schema: ToolInputSchema,
    pub(crate) execution: ToolExecutionMode,
    pub(crate) side_effects: &'static [ToolSideEffect],
    pub(crate) timeout_secs: u64,
    pub(crate) timeout_behavior: ToolTimeoutBehavior,
    pub(crate) cancellation_behavior: ToolCancellationBehavior,
    pub(crate) workspace_behavior: ToolWorkspaceBehavior,
    pub(crate) needs_network: bool,
    pub(crate) may_read_secrets: bool,
}

impl ToolDefinition {
    pub(crate) fn safe_read(
        name: &'static str,
        description: &'static str,
        input_schema: ToolInputSchema,
        side_effects: &'static [ToolSideEffect],
        timeout_secs: u64,
        workspace_behavior: ToolWorkspaceBehavior,
    ) -> Self {
        Self {
            name,
            description,
            risk_class: cadis_protocol::RiskClass::SafeRead,
            input_schema,
            execution: ToolExecutionMode::AutoExecute,
            side_effects,
            timeout_secs,
            timeout_behavior: ToolTimeoutBehavior::FailClosed,
            cancellation_behavior: ToolCancellationBehavior::NotSupported,
            workspace_behavior,
            needs_network: false,
            may_read_secrets: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn approval_placeholder(
        name: &'static str,
        description: &'static str,
        risk_class: cadis_protocol::RiskClass,
        input_schema: ToolInputSchema,
        side_effects: &'static [ToolSideEffect],
        timeout_secs: u64,
        cancellation_behavior: ToolCancellationBehavior,
        workspace_behavior: ToolWorkspaceBehavior,
        needs_network: bool,
        may_read_secrets: bool,
    ) -> Self {
        Self {
            name,
            description,
            risk_class,
            input_schema,
            execution: ToolExecutionMode::ApprovalPlaceholder,
            side_effects,
            timeout_secs,
            timeout_behavior: ToolTimeoutBehavior::FailClosed,
            cancellation_behavior,
            workspace_behavior,
            needs_network,
            may_read_secrets,
        }
    }

    pub(crate) fn policy_reason(&self) -> String {
        match self.execution {
            ToolExecutionMode::AutoExecute => format!(
                "{}: {} | schema={:?} | timeout={}s | workspace={:?}",
                self.name,
                self.description,
                self.input_schema,
                self.timeout_secs,
                self.workspace_behavior,
            ),
            ToolExecutionMode::ApprovalPlaceholder => format!(
                "{}: {} | risk={:?} | schema={:?} | timeout={}s | workspace={:?}",
                self.name,
                self.description,
                self.risk_class,
                self.input_schema,
                self.timeout_secs,
                self.workspace_behavior,
            ),
        }
    }

    pub(crate) fn approval_summary(&self) -> String {
        format!(
            "{} requires approval before execution; side effects: {}; cancellation: {:?}; network: {}; secrets: {}",
            self.name,
            self.side_effects
                .iter()
                .map(|effect| effect.label())
                .collect::<Vec<_>>()
                .join(", "),
            self.cancellation_behavior,
            self.needs_network,
            self.may_read_secrets,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum ToolInputSchema {
    FileRead,
    FileSearch,
    FileList,
    GitStatus,
    GitDiff,
    GitLog,
    GitCommit,
    ShellRun,
    WorkspaceMutation,
    GitMutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolExecutionMode {
    AutoExecute,
    ApprovalPlaceholder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolSideEffect {
    ReadFiles,
    SearchFiles,
    ListFiles,
    ReadGitMetadata,
    ReadGitLog,
    EditWorkspace,
    ReadGitDiff,
    CreateWorktree,
    RemoveWorktree,
    RunSubprocess,
    GitCommit,
}

impl ToolSideEffect {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ReadFiles => "read_files",
            Self::SearchFiles => "search_files",
            Self::ListFiles => "list_files",
            Self::ReadGitMetadata => "read_git_metadata",
            Self::ReadGitLog => "read_git_log",
            Self::EditWorkspace => "edit_workspace",
            Self::ReadGitDiff => "read_git_diff",
            Self::CreateWorktree => "create_worktree",
            Self::RemoveWorktree => "remove_worktree",
            Self::RunSubprocess => "run_subprocess",
            Self::GitCommit => "git_commit",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolTimeoutBehavior {
    FailClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolCancellationBehavior {
    NotSupported,
    Cooperative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolWorkspaceBehavior {
    PathScoped,
    RequiresWorkspace,
    CadisManagedWorktree,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolExecutionResult {
    pub(crate) summary: String,
    pub(crate) output: serde_json::Value,
}

// ── Track D: ToolExecutor trait ──────────────────────────────────────
// Defined as a planned extension point for structured tool backends.
// Existing tools dispatch through match arms; migration to this trait
// is tracked as future Track D work.

/// Context passed to tool executors at invocation time.
/// Reserved for future Track D work: structured tool execution migration.
#[allow(dead_code)]
pub(crate) struct ToolContext {
    pub(crate) workspace: PathBuf,
    pub(crate) tool_name: String,
    pub(crate) timeout_secs: u64,
}

/// Trait for structured tool execution backends.
/// Reserved for future Track D work: structured tool execution migration.
#[allow(dead_code)]
pub(crate) trait ToolExecutor: Send + Sync {
    fn execute(
        &self,
        input: &serde_json::Value,
        context: &ToolContext,
    ) -> Result<ToolExecutionResult, RuntimeError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FilePatchOperation {
    Write {
        path: String,
        content: String,
    },
    Replace {
        path: String,
        old: String,
        new: String,
    },
}

impl FilePatchOperation {
    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Write { path, .. } | Self::Replace { path, .. } => path,
        }
    }

    pub(crate) fn action(&self) -> &'static str {
        match self {
            Self::Write { .. } => "write",
            Self::Replace { .. } => "replace",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedFilePatch {
    pub(crate) path: PathBuf,
    pub(crate) display_path: String,
    pub(crate) action: &'static str,
    pub(crate) content: String,
    pub(crate) mtime: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundedOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShellRunResult {
    pub(crate) status_success: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: BoundedOutput,
    pub(crate) stderr: BoundedOutput,
    pub(crate) timed_out: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchMatch {
    pub(crate) path: String,
    pub(crate) line_number: usize,
    pub(crate) line: String,
}

pub(crate) fn tool_workspace_summary(input: &serde_json::Value) -> Option<String> {
    tool_workspace_id(input)
        .or_else(|| input_string(input, "workspace"))
        .or_else(|| input_string(input, "cwd"))
}

pub(crate) fn tool_workspace_id(input: &serde_json::Value) -> Option<String> {
    input_string(input, "workspace_id")
}

pub(crate) fn required_tool_access(tool_name: &str) -> WorkspaceAccess {
    match tool_name {
        "shell.run" => WorkspaceAccess::Exec,
        "file.write" | "file.patch" | "worker.apply" => WorkspaceAccess::Write,
        _ => WorkspaceAccess::Read,
    }
}

pub(crate) fn tool_command_summary(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    match tool_name {
        "shell.run" => input_string(input, "command").or_else(|| {
            input.get("args").and_then(|value| {
                value.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
            })
        }),
        "file.patch" => file_patch_path_summary(input),
        "worker.apply" => {
            input_string(input, "worker_id").map(|worker_id| format!("worker {worker_id} patch"))
        }
        _ => input_string(input, "path"),
    }
    .map(|value| redact(&value))
}

pub(crate) fn file_patch_path_summary(input: &serde_json::Value) -> Option<String> {
    if let Some(path) = input_string(input, "path").or_else(|| input_string(input, "target")) {
        return Some(path);
    }

    input
        .get("operations")
        .and_then(serde_json::Value::as_array)
        .map(|operations| {
            operations
                .iter()
                .filter_map(|operation| {
                    input_string(operation, "path").or_else(|| input_string(operation, "target"))
                })
                .take(FILE_PATCH_OUTPUT_MAX_FILES)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|summary| !summary.is_empty())
}

pub(crate) fn parse_file_patch_operations(
    input: &serde_json::Value,
) -> Result<Vec<FilePatchOperation>, ErrorPayload> {
    let operations = if let Some(operations) = input.get("operations") {
        let Some(items) = operations.as_array() else {
            return Err(tool_error(
                "invalid_tool_input",
                "file.patch operations must be an array",
                false,
            ));
        };
        items
            .iter()
            .map(parse_file_patch_operation)
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![parse_file_patch_operation(input)?]
    };

    if operations.is_empty() {
        return Err(tool_error(
            "invalid_tool_input",
            "file.patch requires at least one operation",
            false,
        ));
    }
    if operations.len() > FILE_PATCH_MAX_OPERATIONS {
        return Err(tool_error(
            "invalid_tool_input",
            format!("file.patch supports at most {FILE_PATCH_MAX_OPERATIONS} operations"),
            false,
        ));
    }

    Ok(operations)
}

pub(crate) fn parse_file_patch_operation(
    input: &serde_json::Value,
) -> Result<FilePatchOperation, ErrorPayload> {
    let path = input_string(input, "path")
        .or_else(|| input_string(input, "target"))
        .ok_or_else(|| tool_error("invalid_tool_input", "file.patch requires path", false))?;
    let action = input_string(input, "op")
        .or_else(|| input_string(input, "action"))
        .map(|value| value.to_ascii_lowercase());

    match action.as_deref() {
        Some("write") | Some("replace_file") | Some("create") => {
            let content = input_raw_string(input, "content").ok_or_else(|| {
                tool_error(
                    "invalid_tool_input",
                    "file.patch write operation requires content",
                    false,
                )
            })?;
            validate_patch_text_size("content", &content)?;
            Ok(FilePatchOperation::Write { path, content })
        }
        Some("replace") => parse_file_patch_replace(path, input),
        Some(other) => Err(tool_error(
            "invalid_tool_input",
            format!("unsupported file.patch operation '{other}'"),
            false,
        )),
        None if input.get("content").is_some() => {
            let content = input_raw_string(input, "content").ok_or_else(|| {
                tool_error(
                    "invalid_tool_input",
                    "file.patch content must be a string",
                    false,
                )
            })?;
            validate_patch_text_size("content", &content)?;
            Ok(FilePatchOperation::Write { path, content })
        }
        None => parse_file_patch_replace(path, input),
    }
}

pub(crate) fn parse_file_patch_replace(
    path: String,
    input: &serde_json::Value,
) -> Result<FilePatchOperation, ErrorPayload> {
    let old = input_raw_string(input, "old").ok_or_else(|| {
        tool_error(
            "invalid_tool_input",
            "file.patch replace operation requires old",
            false,
        )
    })?;
    let new = input_raw_string(input, "new").ok_or_else(|| {
        tool_error(
            "invalid_tool_input",
            "file.patch replace operation requires new",
            false,
        )
    })?;
    if old.is_empty() {
        return Err(tool_error(
            "invalid_tool_input",
            "file.patch replace operation old cannot be empty",
            false,
        ));
    }
    validate_patch_text_size("old", &old)?;
    validate_patch_text_size("new", &new)?;
    Ok(FilePatchOperation::Replace { path, old, new })
}

pub(crate) fn validate_patch_text_size(label: &str, value: &str) -> Result<(), ErrorPayload> {
    if value.len() > FILE_PATCH_MAX_FILE_BYTES {
        Err(tool_error(
            "file_patch_too_large",
            format!("file.patch {label} exceeds {FILE_PATCH_MAX_FILE_BYTES} bytes"),
            false,
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_file_patch_input(
    workspace: &Path,
    input: &serde_json::Value,
) -> Result<(), ErrorPayload> {
    let operations = parse_file_patch_operations(input)?;
    for operation in &operations {
        let target = resolve_file_patch_target(workspace, operation.path())?;
        validate_file_patch_target(operation, &target)?;
    }
    Ok(())
}

pub(crate) fn validate_file_patch_target(
    operation: &FilePatchOperation,
    target: &Path,
) -> Result<(), ErrorPayload> {
    match operation {
        FilePatchOperation::Write { .. } => {
            if let Ok(metadata) = fs::metadata(target) {
                if !metadata.is_file() {
                    return Err(tool_error(
                        "unsupported_file_type",
                        "file.patch can only write regular files",
                        false,
                    ));
                }
            }
            Ok(())
        }
        FilePatchOperation::Replace { .. } => {
            let metadata = fs::metadata(target).map_err(|error| {
                tool_error(
                    "path_resolution_failed",
                    format!("could not read file.patch target: {error}"),
                    false,
                )
            })?;
            if !metadata.is_file() {
                return Err(tool_error(
                    "unsupported_file_type",
                    "file.patch can only replace regular files",
                    false,
                ));
            }
            if metadata.len() > FILE_PATCH_MAX_FILE_BYTES as u64 {
                return Err(tool_error(
                    "file_patch_too_large",
                    format!("file.patch target exceeds {FILE_PATCH_MAX_FILE_BYTES} bytes"),
                    false,
                ));
            }
            Ok(())
        }
    }
}

pub(crate) fn prepare_file_patch(
    workspace: &Path,
    operations: &[FilePatchOperation],
) -> Result<Vec<PreparedFilePatch>, ErrorPayload> {
    let mut staged = HashMap::<PathBuf, String>::new();
    let mut prepared = Vec::new();

    for operation in operations {
        let path = resolve_file_patch_target(workspace, operation.path())?;
        validate_file_patch_target(operation, &path)?;
        let mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        let content = match operation {
            FilePatchOperation::Write { content, .. } => content.clone(),
            FilePatchOperation::Replace { old, new, .. } => {
                let current = match staged.get(&path) {
                    Some(content) => content.clone(),
                    None => read_patch_target(&path)?,
                };
                replace_once(&current, old, new)?
            }
        };
        validate_patch_text_size("result", &content)?;
        staged.insert(path.clone(), content.clone());
        prepared.push(PreparedFilePatch {
            display_path: display_relative_path(workspace, &path),
            path,
            action: operation.action(),
            content,
            mtime,
        });
    }

    Ok(prepared)
}

pub(crate) fn read_patch_target(path: &Path) -> Result<String, ErrorPayload> {
    fs::read_to_string(path).map_err(|error| {
        tool_error(
            "file_patch_read_failed",
            format!("could not read patch target: {error}"),
            false,
        )
    })
}

pub(crate) fn replace_once(content: &str, old: &str, new: &str) -> Result<String, ErrorPayload> {
    let matches = content.match_indices(old).take(2).count();
    match matches {
        0 => Err(tool_error(
            "file_patch_replace_mismatch",
            "file.patch old text was not found exactly once",
            false,
        )),
        1 => Ok(content.replacen(old, new, 1)),
        _ => Err(tool_error(
            "file_patch_replace_ambiguous",
            "file.patch old text matched more than once",
            false,
        )),
    }
}

pub(crate) fn resolve_file_patch_target(
    workspace: &Path,
    user_path: &str,
) -> Result<PathBuf, ErrorPayload> {
    let relative = Path::new(user_path);
    if relative.is_absolute() || path_has_parent_or_root(relative) {
        return Err(tool_error(
            "outside_workspace",
            "file.patch paths must be relative to the workspace",
            false,
        ));
    }
    if relative.file_name().is_none() {
        return Err(tool_error(
            "invalid_tool_input",
            "file.patch path must name a file",
            false,
        ));
    }
    validate_file_patch_relative_path(relative)?;

    let workspace = workspace.canonicalize().map_err(|error| {
        tool_error(
            "path_resolution_failed",
            format!("could not resolve workspace: {error}"),
            false,
        )
    })?;
    let candidate = workspace.join(relative);
    if let Ok(resolved) = candidate.canonicalize() {
        if !resolved.starts_with(&workspace) {
            return Err(tool_error(
                "outside_workspace",
                "file.patch target resolves outside the workspace",
                false,
            ));
        }
        if resolved.is_dir() {
            return Err(tool_error(
                "unsupported_file_type",
                "file.patch target must be a file",
                false,
            ));
        }
        if let Ok(resolved_relative) = resolved.strip_prefix(&workspace) {
            validate_file_patch_relative_path(resolved_relative)?;
        }
        return Ok(resolved);
    }

    let parent = candidate.parent().ok_or_else(|| {
        tool_error(
            "invalid_tool_input",
            "file.patch path must have a parent directory",
            false,
        )
    })?;
    let parent = parent.canonicalize().map_err(|error| {
        tool_error(
            "path_resolution_failed",
            format!("could not resolve file.patch parent: {error}"),
            false,
        )
    })?;
    if !parent.starts_with(&workspace) {
        return Err(tool_error(
            "outside_workspace",
            "file.patch parent resolves outside the workspace",
            false,
        ));
    }
    let file_name = candidate.file_name().ok_or_else(|| {
        tool_error(
            "invalid_tool_input",
            "file.patch path must name a file",
            false,
        )
    })?;
    let resolved = parent.join(file_name);
    if let Ok(resolved_relative) = resolved.strip_prefix(&workspace) {
        validate_file_patch_relative_path(resolved_relative)?;
    }
    Ok(resolved)
}

pub(crate) fn validate_file_patch_relative_path(path: &Path) -> Result<(), ErrorPayload> {
    if file_patch_path_is_protected(path) {
        return Err(tool_error(
            "protected_path",
            "file.patch refuses to modify protected workspace metadata paths",
            false,
        ));
    }
    if file_patch_path_is_secret_like(path) {
        return Err(tool_error(
            "secret_path_rejected",
            "file.patch refuses to modify secret-like paths",
            false,
        ));
    }
    Ok(())
}

pub(crate) fn path_has_parent_or_root(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

pub(crate) fn file_patch_path_is_protected(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => matches!(value.to_str(), Some(".git" | ".cadis")),
        _ => false,
    })
}

pub(crate) fn file_patch_path_is_secret_like(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(value) = component else {
            return false;
        };
        let name = value.to_string_lossy().to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || matches!(
                name.as_str(),
                ".netrc"
                    | ".npmrc"
                    | ".pypirc"
                    | ".git-credentials"
                    | "id_rsa"
                    | "id_dsa"
                    | "id_ecdsa"
                    | "id_ed25519"
                    | ".ssh"
                    | ".aws"
                    | ".gnupg"
            )
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name.contains("secret")
            || name.contains("credential")
            || name.contains("token")
            || name.contains("api_key")
            || name.contains("apikey")
            || name.contains("private_key")
    })
}

/// Item 17: Returns true if an environment variable name looks secret-bearing.
/// Reserved for future use in policy enforcement.
#[allow(dead_code)]
pub(crate) fn is_secret_env_var(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("secret")
        || lower.contains("password")
        || lower.contains("token")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("private_key")
        || lower.contains("credential")
        || lower.ends_with("_key")
        || matches!(
            lower.as_str(),
            "aws_secret_access_key"
                | "openai_api_key"
                | "cadis_openai_api_key"
                | "ssh_auth_sock"
                | "github_token"
                | "gh_token"
        )
}

/// Item 17: Returns true if a config value looks like it contains a secret.
/// Reserved for future use in policy enforcement.
#[allow(dead_code)]
pub(crate) fn is_secret_config_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("secret=")
        || lower.contains("password=")
        || lower.contains("token=")
        || lower.contains("api_key=")
        || lower.contains("apikey=")
        || lower.contains("private_key=")
}

pub(crate) fn validate_git_pathspec(pathspec: &str) -> Result<String, ErrorPayload> {
    let trimmed = pathspec.trim();
    if trimmed.is_empty() {
        return Err(tool_error(
            "invalid_tool_input",
            "git.diff pathspec cannot be empty",
            false,
        ));
    }
    if trimmed.starts_with(':') {
        return Err(tool_error(
            "invalid_tool_input",
            "git.diff pathspec magic is not supported",
            false,
        ));
    }

    let path = Path::new(trimmed);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(tool_error(
            "outside_workspace",
            "git.diff pathspec must be relative to the workspace",
            false,
        ));
    }

    Ok(trimmed.to_owned())
}

pub(crate) fn resolve_inside_workspace(
    workspace: &Path,
    user_path: &str,
) -> Result<PathBuf, ErrorPayload> {
    let candidate = PathBuf::from(user_path);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        workspace.join(candidate)
    };
    let resolved = candidate.canonicalize().map_err(|error| {
        tool_error(
            "path_resolution_failed",
            format!("could not resolve {}: {error}", candidate.display()),
            false,
        )
    })?;

    if resolved.starts_with(workspace) {
        Ok(resolved)
    } else {
        Err(tool_error(
            "outside_workspace",
            format!(
                "{} is outside workspace {}",
                resolved.display(),
                workspace.display()
            ),
            false,
        ))
    }
}

pub(crate) fn display_relative_path(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(crate) fn search_files(
    workspace: &Path,
    root: &Path,
    query: &str,
    max_results: usize,
    matches: &mut Vec<SearchMatch>,
) {
    if matches.len() >= max_results {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        if matches.len() >= max_results {
            return;
        }
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        let Ok(resolved) = path.canonicalize() else {
            continue;
        };
        if !resolved.starts_with(workspace) {
            continue;
        }
        let Ok(metadata) = fs::metadata(&resolved) else {
            continue;
        };
        if metadata.is_dir() {
            search_files(workspace, &resolved, query, max_results, matches);
        } else if metadata.is_file() && metadata.len() <= FILE_SEARCH_LIMIT_BYTES {
            search_file(workspace, &resolved, query, max_results, matches);
        }
    }
}

pub(crate) fn search_file(
    workspace: &Path,
    path: &Path,
    query: &str,
    max_results: usize,
    matches: &mut Vec<SearchMatch>,
) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    for (index, line) in content.lines().enumerate() {
        if matches.len() >= max_results {
            return;
        }
        if line.contains(query) {
            matches.push(SearchMatch {
                path: display_relative_path(workspace, path),
                line_number: index + 1,
                line: line.to_owned(),
            });
        }
    }
}

pub(crate) fn tool_error(
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> ErrorPayload {
    ErrorPayload {
        code: code.into(),
        message: redact(&message.into()),
        retryable,
    }
}
