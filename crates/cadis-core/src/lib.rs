//! Core CADIS request handling and event production.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use cadis_models::{
    provider_catalog, ModelInvocation, ModelProvider, ModelRequest, ModelStreamEvent,
    ProviderCatalogEntry, ProviderReadiness,
};
use cadis_policy::{PolicyDecision, PolicyEngine};
use cadis_protocol::{
    AgentEventPayload, AgentId, AgentListPayload, AgentModelChangedPayload, AgentRenamedPayload,
    AgentSessionEventPayload, AgentSessionId, AgentSessionStatus, AgentSpawnRequest, AgentStatus,
    AgentStatusChangedPayload, ApprovalDecision, ApprovalId, ApprovalRequestPayload,
    ApprovalResolvedPayload, ApprovalResponseRequest, CadisEvent, ClientRequest, DaemonResponse,
    DaemonStatusPayload, ErrorPayload, EventEnvelope, EventId, MessageCompletedPayload,
    MessageDeltaPayload, MessageSendRequest, ModelDescriptor, ModelInvocationPayload,
    ModelReadiness, ModelsListPayload, OrchestratorRoutePayload, ProtocolVersion,
    RequestAcceptedPayload, RequestEnvelope, RequestId, ResponseEnvelope, SessionEventPayload,
    SessionId, Timestamp, ToolCallId, ToolCallRequest, ToolEventPayload, ToolFailedPayload,
    UiPreferencesPayload, WorkerArtifactLocations, WorkerEventPayload, WorkerLogDeltaPayload,
    WorkerTailRequest, WorkerWorktreeCleanupPolicy, WorkerWorktreeIntent, WorkerWorktreeState,
    WorkspaceAccess, WorkspaceDoctorCheck, WorkspaceDoctorPayload, WorkspaceDoctorRequest,
    WorkspaceGrantId, WorkspaceGrantPayload, WorkspaceGrantRequest, WorkspaceId, WorkspaceKind,
    WorkspaceListPayload, WorkspaceListRequest, WorkspaceRecordPayload, WorkspaceRegisterRequest,
    WorkspaceRevokeRequest,
};
use cadis_store::{
    redact, ApprovalRecord, ApprovalState, ApprovalStore, CadisConfig, CadisHome, CheckpointPolicy,
    GrantSource as StoreGrantSource, ProfileHome, StateRecoveryDiagnostic, StateStore,
    WorkspaceAccess as StoreWorkspaceAccess, WorkspaceAlias,
    WorkspaceGrantRecord as StoreWorkspaceGrantRecord, WorkspaceKind as StoreWorkspaceKind,
    WorkspaceMetadata, WorkspaceRegistry, WorkspaceVcs,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

const FILE_READ_LIMIT_BYTES: usize = 64 * 1024;
const FILE_SEARCH_LIMIT_BYTES: u64 = 1024 * 1024;
const FILE_SEARCH_DEFAULT_LIMIT: usize = 50;
const APPROVAL_TIMEOUT_MINUTES: i64 = 5;
const WORKER_TAIL_DEFAULT_LINES: usize = 64;
const WORKER_TAIL_MAX_LINES: usize = 1_000;

/// Runtime options supplied by the daemon process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeOptions {
    /// Local CADIS state directory.
    pub cadis_home: PathBuf,
    /// Active profile ID under `CADIS_HOME/profiles`.
    pub profile_id: String,
    /// Socket path this daemon listens on.
    pub socket_path: Option<PathBuf>,
    /// Configured model provider label.
    pub model_provider: String,
    /// Initial daemon-owned UI preferences.
    pub ui_preferences: serde_json::Value,
}

/// Limits for request-driven `agent.spawn`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSpawnLimits {
    /// Maximum child depth below a root agent.
    pub max_depth: usize,
    /// Maximum direct children any one parent may own.
    pub max_children_per_parent: usize,
    /// Maximum total registered agents, including built-in agents.
    pub max_total_agents: usize,
}

impl Default for AgentSpawnLimits {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_children_per_parent: 4,
            max_total_agents: 32,
        }
    }
}

/// Configuration for daemon-owned orchestration behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrchestratorConfig {
    /// Whether explicit `/worker`, `/spawn`, `/route`, and `/delegate` actions are enabled.
    pub worker_delegation_enabled: bool,
    /// Role used when `/worker` does not include an explicit `Role:` prefix.
    pub default_worker_role: String,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            worker_delegation_enabled: true,
            default_worker_role: "Worker".to_owned(),
        }
    }
}

/// Configuration for the in-memory AgentSession state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentRuntimeConfig {
    /// Default timeout for a per-route agent session.
    pub default_timeout_sec: i64,
    /// Maximum state-machine steps a single agent route may consume.
    pub max_steps_per_session: u32,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            default_timeout_sec: 900,
            max_steps_per_session: 1,
        }
    }
}

/// Result of handling one client request.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestOutcome {
    /// Immediate response for the request.
    pub response: ResponseEnvelope,
    /// Follow-up daemon events.
    pub events: Vec<EventEnvelope>,
}

/// CADIS core runtime.
pub struct Runtime {
    options: RuntimeOptions,
    provider: Box<dyn ModelProvider>,
    tools: ToolRegistry,
    started_at: Instant,
    next_event: u64,
    next_session: u64,
    next_agent: u64,
    next_agent_session: u64,
    next_route: u64,
    next_worker: u64,
    sessions: HashMap<SessionId, SessionRecord>,
    agents: HashMap<AgentId, AgentRecord>,
    agent_sessions: HashMap<AgentSessionId, AgentSessionRecord>,
    workers: HashMap<String, WorkerRecord>,
    orchestrator: Orchestrator,
    ui_preferences: serde_json::Value,
    spawn_limits: AgentSpawnLimits,
    agent_runtime: AgentRuntimeConfig,
    policy: PolicyEngine,
    approval_store: ApprovalStore,
    state_store: StateStore,
    recovery_diagnostics: Vec<RuntimeRecoveryDiagnostic>,
    profile_home: ProfileHome,
    pending_approvals: HashMap<ApprovalId, PendingApproval>,
    workspaces: HashMap<WorkspaceId, WorkspaceRecord>,
    workspace_grants: HashMap<WorkspaceGrantId, WorkspaceGrantRecord>,
    next_tool: u64,
    next_approval: u64,
    next_workspace_grant: u64,
}

impl Runtime {
    /// Creates a runtime with the supplied model provider.
    pub fn new(options: RuntimeOptions, provider: Box<dyn ModelProvider>) -> Self {
        let ui_preferences = options.ui_preferences.clone();
        let spawn_limits = AgentSpawnLimits::from_options(&options.ui_preferences);
        let agent_runtime = AgentRuntimeConfig::from_options(&options.ui_preferences);
        let orchestrator =
            Orchestrator::new(OrchestratorConfig::from_options(&options.ui_preferences));
        let approval_store = ApprovalStore::new(&options.cadis_home);
        let state_store = StateStore::new(&CadisConfig {
            cadis_home: options.cadis_home.clone(),
            ..CadisConfig::default()
        });
        let _ = state_store.ensure_layout();
        let profile_home = CadisHome::new(&options.cadis_home)
            .init_profile(&options.profile_id)
            .unwrap_or_else(|_| CadisHome::new(&options.cadis_home).profile(&options.profile_id));
        let workspaces = load_workspace_registry(&profile_home);
        let workspace_grants = load_workspace_grants(&profile_home, &workspaces);
        let next_workspace_grant = next_workspace_grant_counter(&workspace_grants);
        let sessions = recover_session_records(&state_store);
        let agent_session_recovery = recover_agent_session_records(&state_store);
        let agent_sessions = agent_session_recovery.records;
        let recovery_diagnostics = agent_session_recovery.diagnostics;
        let mut agents = default_agents(&options.model_provider);
        for (agent_id, record) in recover_agent_records(&state_store) {
            agents.insert(agent_id, record);
        }
        let next_session = next_session_counter(&sessions);
        let next_agent = next_agent_counter(&agents);
        let next_agent_session = next_agent_session_counter(&agent_sessions);
        let next_route = next_route_counter(&agent_sessions);

        Self {
            options,
            provider,
            tools: ToolRegistry::builtin().expect("built-in tool registry should be valid"),
            started_at: Instant::now(),
            next_event: 1,
            next_session,
            next_agent,
            next_agent_session,
            next_route,
            next_worker: 1,
            sessions,
            agents,
            agent_sessions,
            workers: HashMap::new(),
            orchestrator,
            ui_preferences,
            spawn_limits,
            agent_runtime,
            policy: PolicyEngine,
            approval_store,
            state_store,
            recovery_diagnostics,
            profile_home,
            pending_approvals: HashMap::new(),
            workspaces,
            workspace_grants,
            next_tool: 1,
            next_approval: 1,
            next_workspace_grant,
        }
    }

    /// Handles one protocol request.
    pub fn handle_request(&mut self, envelope: RequestEnvelope) -> RequestOutcome {
        let request_id = envelope.request_id.clone();

        if let Err(error) = envelope.protocol_version.ensure_supported() {
            return self.reject(
                request_id,
                "unsupported_protocol_version",
                error.to_string(),
                false,
            );
        }

        match envelope.request {
            ClientRequest::EventsSubscribe(request) => {
                let events = if request.include_snapshot {
                    self.snapshot_events()
                } else {
                    Vec::new()
                };
                self.accept(request_id, events)
            }
            ClientRequest::EventsSnapshot(_) => {
                let events = self.snapshot_events();
                self.accept(request_id, events)
            }
            ClientRequest::DaemonStatus(_) => self.status(request_id),
            ClientRequest::SessionCreate(request) => {
                let title = request.title;
                let session_id = self.create_session(title.clone(), request.cwd);
                let event = self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionStarted(SessionEventPayload { session_id, title }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::SessionCancel(request) => {
                if self.sessions.remove(&request.session_id).is_some() {
                    let mut events = self.cancel_agent_sessions_for_session(&request.session_id);
                    let _ = self
                        .state_store
                        .remove_session_metadata(&request.session_id);
                    let event = self.session_event(
                        request.session_id.clone(),
                        CadisEvent::SessionCompleted(SessionEventPayload {
                            session_id: request.session_id,
                            title: None,
                        }),
                    );
                    events.push(event);
                    self.accept(request_id, events)
                } else {
                    self.reject(
                        request_id,
                        "session_not_found",
                        "session was not found",
                        false,
                    )
                }
            }
            ClientRequest::SessionSubscribe(request) => {
                if let Some(session) = self.sessions.get(&request.session_id) {
                    let title = session.title.clone();
                    let event = self.session_event(
                        request.session_id.clone(),
                        CadisEvent::SessionUpdated(SessionEventPayload {
                            session_id: request.session_id,
                            title,
                        }),
                    );
                    self.accept(request_id, vec![event])
                } else {
                    self.reject(
                        request_id,
                        "session_not_found",
                        "session was not found",
                        false,
                    )
                }
            }
            ClientRequest::MessageSend(request) => self.handle_message(request_id, request),
            ClientRequest::AgentRename(request) => {
                let display_name = normalize_agent_name(&request.display_name, &request.agent_id);
                let Some(agent) = self.agents.get_mut(&request.agent_id) else {
                    return self.reject(
                        request_id,
                        "agent_not_found",
                        format!("agent '{}' was not found", request.agent_id),
                        false,
                    );
                };
                agent.display_name = display_name.clone();
                let _ = self.persist_agent_record(&request.agent_id);
                let event = self.event(
                    None,
                    CadisEvent::AgentRenamed(AgentRenamedPayload {
                        agent_id: request.agent_id,
                        display_name,
                    }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::AgentModelSet(request) => {
                let Some(agent) = self.agents.get_mut(&request.agent_id) else {
                    return self.reject(
                        request_id,
                        "agent_not_found",
                        format!("agent '{}' was not found", request.agent_id),
                        false,
                    );
                };
                agent.model = request.model.clone();
                let _ = self.persist_agent_record(&request.agent_id);
                let event = self.event(
                    None,
                    CadisEvent::AgentModelChanged(AgentModelChangedPayload {
                        agent_id: request.agent_id,
                        model: request.model,
                    }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::AgentList(_) => {
                let agents = self
                    .agent_records_sorted()
                    .into_iter()
                    .map(AgentRecord::event_payload)
                    .collect();
                let event = self.event(
                    None,
                    CadisEvent::AgentListResponse(AgentListPayload { agents }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::AgentSpawn(request) => self.spawn_agent(request_id, request),
            ClientRequest::AgentKill(request) => self.kill_agent(request_id, request.agent_id),
            ClientRequest::WorkspaceList(request) => self.workspace_list(request_id, request),
            ClientRequest::WorkspaceRegister(request) => {
                self.workspace_register(request_id, request)
            }
            ClientRequest::WorkspaceGrant(request) => self.workspace_grant(request_id, request),
            ClientRequest::WorkspaceRevoke(request) => self.workspace_revoke(request_id, request),
            ClientRequest::WorkspaceDoctor(request) => self.workspace_doctor(request_id, request),
            ClientRequest::ModelsList(_) => {
                let models = provider_catalog()
                    .into_iter()
                    .map(model_descriptor_from_catalog_entry)
                    .collect();
                let event = self.event(
                    None,
                    CadisEvent::ModelsListResponse(ModelsListPayload { models }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::UiPreferencesGet(_) => {
                let event = self.event(
                    None,
                    CadisEvent::UiPreferencesUpdated(UiPreferencesPayload {
                        preferences: self.ui_preferences.clone(),
                    }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::UiPreferencesSet(request) => {
                self.ui_preferences = merge_json(self.ui_preferences.clone(), request.patch);
                let event = self.event(
                    None,
                    CadisEvent::UiPreferencesUpdated(UiPreferencesPayload {
                        preferences: self.ui_preferences.clone(),
                    }),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::ConfigReload(_) => self.accept(request_id, Vec::new()),
            ClientRequest::ToolCall(request) => self.handle_tool_call(request_id, request),
            ClientRequest::ApprovalRespond(request) => {
                self.handle_approval_response(request_id, request)
            }
            ClientRequest::VoicePreview(_) => {
                let started = self.event(None, CadisEvent::VoicePreviewStarted(Default::default()));
                let completed =
                    self.event(None, CadisEvent::VoicePreviewCompleted(Default::default()));
                self.accept(request_id, vec![started, completed])
            }
            ClientRequest::VoiceStop(_) => {
                let completed =
                    self.event(None, CadisEvent::VoicePreviewCompleted(Default::default()));
                self.accept(request_id, vec![completed])
            }
            ClientRequest::WorkerTail(request) => self.worker_tail(request_id, request),
            ClientRequest::SessionUnsubscribe(_) => self.reject(
                request_id,
                "not_implemented",
                "this request is defined in the protocol but is not implemented in the desktop MVP",
                false,
            ),
        }
    }

    fn status(&self, request_id: RequestId) -> RequestOutcome {
        RequestOutcome {
            response: ResponseEnvelope::new(
                request_id,
                DaemonResponse::DaemonStatus(DaemonStatusPayload {
                    status: "ok".to_owned(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    protocol_version: ProtocolVersion::current(),
                    cadis_home: self.options.cadis_home.display().to_string(),
                    socket_path: self
                        .options
                        .socket_path
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    sessions: self.sessions.len(),
                    model_provider: self.options.model_provider.clone(),
                    uptime_seconds: self.started_at.elapsed().as_secs(),
                }),
            ),
            events: Vec::new(),
        }
    }

    fn handle_tool_call(
        &mut self,
        request_id: RequestId,
        request: ToolCallRequest,
    ) -> RequestOutcome {
        let Some(tool) = self.tools.get(&request.tool_name) else {
            return self.reject(
                request_id,
                "tool_denied",
                format!("{}: unknown tool is denied by default", request.tool_name),
                false,
            );
        };
        let risk_class = tool.risk_class;
        let policy_decision = self.policy.decide(risk_class);
        let policy_reason = tool.policy_reason();

        let (session_id, mut events) =
            self.resolve_tool_session(request.session_id.clone(), &request.input);
        let tool_call_id = self.next_tool_call_id();
        events.push(self.session_event(
            session_id.clone(),
            CadisEvent::ToolRequested(ToolEventPayload {
                tool_call_id: tool_call_id.clone(),
                tool_name: request.tool_name.clone(),
                summary: Some(policy_reason.clone()),
                risk_class: Some(risk_class),
                output: None,
            }),
        ));

        let required_access = required_tool_access(&request.tool_name);
        let workspace = match self.resolved_granted_workspace(
            &session_id,
            request.agent_id.as_ref(),
            &request.input,
            required_access,
        ) {
            Ok(workspace) => workspace,
            Err(error) => {
                events.push(self.session_event(
                    session_id,
                    CadisEvent::ToolFailed(ToolFailedPayload {
                        tool_call_id,
                        tool_name: request.tool_name,
                        error,
                        risk_class: Some(risk_class),
                    }),
                ));
                return self.accept(request_id, events);
            }
        };

        match policy_decision {
            PolicyDecision::Allow => {
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::ToolStarted(ToolEventPayload {
                        tool_call_id: tool_call_id.clone(),
                        tool_name: request.tool_name.clone(),
                        summary: Some("tool execution started".to_owned()),
                        risk_class: Some(risk_class),
                        output: None,
                    }),
                ));

                match self.execute_safe_tool(&workspace.root, &request) {
                    Ok(result) => events.push(self.session_event(
                        session_id,
                        CadisEvent::ToolCompleted(ToolEventPayload {
                            tool_call_id,
                            tool_name: request.tool_name,
                            summary: Some(result.summary),
                            risk_class: Some(risk_class),
                            output: Some(result.output),
                        }),
                    )),
                    Err(error) => events.push(self.session_event(
                        session_id,
                        CadisEvent::ToolFailed(ToolFailedPayload {
                            tool_call_id,
                            tool_name: request.tool_name,
                            error,
                            risk_class: Some(risk_class),
                        }),
                    )),
                }
                self.accept(request_id, events)
            }
            PolicyDecision::RequireApproval => {
                let approval_id = self.next_approval_id();
                let requested_at = now_timestamp();
                let expires_at = timestamp_after_minutes(APPROVAL_TIMEOUT_MINUTES);
                let command = tool_command_summary(&request.tool_name, &request.input);
                let workspace = Some(workspace.root.display().to_string());
                let record = ApprovalRecord {
                    approval_id: approval_id.clone(),
                    session_id: session_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    tool_name: request.tool_name.clone(),
                    risk_class,
                    title: format!("Approve {}", request.tool_name),
                    summary: format!("{} is blocked until approved", request.tool_name),
                    command: command.clone(),
                    workspace: workspace.clone(),
                    requested_at,
                    expires_at: expires_at.clone(),
                    state: ApprovalState::Pending,
                    decision: None,
                    reason: None,
                    resolved_at: None,
                };

                if let Err(error) = self.approval_store.save(&record) {
                    return self.reject(
                        request_id,
                        "approval_persistence_failed",
                        error.to_string(),
                        false,
                    );
                }
                self.pending_approvals.insert(
                    approval_id.clone(),
                    PendingApproval {
                        record: record.clone(),
                    },
                );

                events.push(self.session_event(
                    session_id,
                    CadisEvent::ApprovalRequested(ApprovalRequestPayload {
                        approval_id,
                        session_id: record.session_id,
                        tool_call_id,
                        risk_class,
                        title: record.title,
                        summary: record.summary,
                        command,
                        workspace,
                        expires_at,
                    }),
                ));
                self.accept(request_id, events)
            }
            PolicyDecision::Deny => self.reject(
                request_id,
                "tool_denied",
                format!("{}: {policy_reason}", request.tool_name),
                false,
            ),
        }
    }

    fn handle_approval_response(
        &mut self,
        request_id: RequestId,
        request: ApprovalResponseRequest,
    ) -> RequestOutcome {
        let mut record = match self.pending_approvals.remove(&request.approval_id) {
            Some(pending) => pending.record,
            None => match self.approval_store.load(&request.approval_id) {
                Ok(Some(record)) => record,
                Ok(None) => {
                    return self.reject(
                        request_id,
                        "approval_not_found",
                        format!("approval '{}' was not found", request.approval_id),
                        false,
                    )
                }
                Err(error) => {
                    return self.reject(
                        request_id,
                        "approval_persistence_failed",
                        error.to_string(),
                        false,
                    )
                }
            },
        };

        if record.state != ApprovalState::Pending {
            return self.reject(
                request_id,
                "approval_already_resolved",
                format!("approval '{}' is already resolved", request.approval_id),
                false,
            );
        }

        let effective_decision = if approval_is_expired(&record) {
            record.state = ApprovalState::Expired;
            ApprovalDecision::Denied
        } else {
            record.state = ApprovalState::Resolved;
            request.decision
        };
        record.decision = Some(effective_decision);
        record.reason = request.reason.map(|reason| redact(&reason));
        record.resolved_at = Some(now_timestamp());

        if let Err(error) = self.approval_store.save(&record) {
            return self.reject(
                request_id,
                "approval_persistence_failed",
                error.to_string(),
                false,
            );
        }

        let session_id = record.session_id.clone();
        let mut events = vec![self.session_event(
            session_id.clone(),
            CadisEvent::ApprovalResolved(ApprovalResolvedPayload {
                approval_id: record.approval_id.clone(),
                decision: effective_decision,
            }),
        )];
        let error = match effective_decision {
            ApprovalDecision::Approved => ErrorPayload {
                code: "tool_execution_blocked".to_owned(),
                message: "approval was recorded, but risky tool execution is not implemented in this baseline".to_owned(),
                retryable: false,
            },
            ApprovalDecision::Denied => ErrorPayload {
                code: if record.state == ApprovalState::Expired {
                    "approval_expired".to_owned()
                } else {
                    "approval_denied".to_owned()
                },
                message: "approval did not authorize tool execution".to_owned(),
                retryable: false,
            },
        };
        events.push(self.session_event(
            session_id,
            CadisEvent::ToolFailed(ToolFailedPayload {
                tool_call_id: record.tool_call_id,
                tool_name: record.tool_name,
                error,
                risk_class: Some(record.risk_class),
            }),
        ));

        self.accept(request_id, events)
    }

    fn handle_message(
        &mut self,
        request_id: RequestId,
        request: MessageSendRequest,
    ) -> RequestOutcome {
        let content_kind = request.content_kind;
        let content = request.content;
        let decision =
            match self
                .orchestrator
                .route_message(request.target_agent_id, &content, &self.agents)
            {
                Ok(decision) => decision,
                Err(error) => return self.reject(request_id, error.code, error.message, false),
            };

        let (route, spawned_agent) = match decision {
            OrchestratorDecision::Route(route) => (route, None),
            OrchestratorDecision::SpawnAndRoute(spawn) => {
                let record = match self.spawn_agent_record(AgentSpawnRequest {
                    role: spawn.role.clone(),
                    parent_agent_id: Some(AgentId::from("main")),
                    display_name: None,
                    model: None,
                }) {
                    Ok(record) => record,
                    Err(error) => return self.reject(request_id, error.code, error.message, false),
                };
                (
                    RouteDecision {
                        agent_id: record.id.clone(),
                        agent_name: record.display_name.clone(),
                        content: normalize_route_content(spawn.content),
                        reason: spawn.reason,
                        worker_summary: Some(spawn.worker_summary),
                    },
                    Some(record),
                )
            }
        };
        let session_id_request = request.session_id;
        let (session_id, mut events) = match session_id_request {
            Some(session_id) if self.sessions.contains_key(&session_id) => (session_id, Vec::new()),
            Some(session_id) => {
                let title = Some(title_from_message(&content));
                self.insert_session_record(
                    session_id.clone(),
                    SessionRecord {
                        title: title.clone(),
                        _cwd: None,
                    },
                );
                let event = self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionStarted(SessionEventPayload {
                        session_id: session_id.clone(),
                        title,
                    }),
                );
                (session_id, vec![event])
            }
            None => {
                let session_id = self.create_session(Some(title_from_message(&content)), None);
                let title = self
                    .sessions
                    .get(&session_id)
                    .and_then(|session| session.title.clone());
                let event = self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionStarted(SessionEventPayload {
                        session_id: session_id.clone(),
                        title,
                    }),
                );
                (session_id, vec![event])
            }
        };

        if let Some(record) = spawned_agent {
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::AgentSpawned(record.clone().event_payload()),
            ));
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id: record.id,
                    status: AgentStatus::Idle,
                    task: Some("spawned by orchestrator action".to_owned()),
                }),
            ));
        }

        let route_id = format!("route_{:06}", self.next_route);
        self.next_route += 1;
        let (agent_session_id, agent_session_started) = self.start_agent_session(
            session_id.clone(),
            route_id.clone(),
            route.agent_id.clone(),
            route.content.clone(),
        );
        events.push(agent_session_started);

        events.push(self.session_event(
            session_id.clone(),
            CadisEvent::OrchestratorRoute(OrchestratorRoutePayload {
                id: route_id,
                source: "cadisd".to_owned(),
                target_agent_id: route.agent_id.clone(),
                target_agent_name: route.agent_name.clone(),
                reason: route.reason.clone(),
            }),
        ));

        let session_workspace = self.session_workspace(&session_id);
        let artifact_root = self.options.cadis_home.join("artifacts").join("workers");
        let worker = route.worker_summary.as_ref().map(|summary| {
            let worker_id = self.next_worker_id();
            WorkerDelegation {
                worktree: planned_worker_worktree(
                    &worker_id,
                    session_workspace.as_deref(),
                    &route.content,
                ),
                artifacts: worker_artifact_locations(&artifact_root, &worker_id),
                worker_id,
                parent_agent_id: self
                    .agents
                    .get(&route.agent_id)
                    .and_then(|agent| agent.parent_agent_id.clone())
                    .or_else(|| (route.agent_id.as_str() != "main").then(|| AgentId::from("main"))),
                summary: summary.clone(),
            }
        });

        if let Some(worker) = &worker {
            events.extend(self.start_worker(session_id.clone(), route.agent_id.clone(), worker));
        }

        if route.agent_id.as_str() != "main" {
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id: AgentId::from("main"),
                    status: AgentStatus::Running,
                    task: Some(format!(
                        "Routing request to {} ({})",
                        route.agent_name, route.reason
                    )),
                }),
            ));
        }

        events.push(self.session_event(
            session_id.clone(),
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: route.agent_id.clone(),
                status: AgentStatus::Running,
                task: Some(route.content.clone()),
            }),
        ));

        if let Some(event) = self.consume_agent_session_step(&agent_session_id) {
            events.push(event);
        }
        if self
            .agent_sessions
            .get(&agent_session_id)
            .is_some_and(|record| record.status == AgentSessionStatus::BudgetExceeded)
        {
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id: route.agent_id.clone(),
                    status: AgentStatus::Failed,
                    task: Some("agent budget exceeded".to_owned()),
                }),
            ));
            events.push(self.session_event(
                session_id,
                CadisEvent::SessionFailed(ErrorPayload {
                    code: "agent_budget_exceeded".to_owned(),
                    message: "agent session exceeded its configured step budget".to_owned(),
                    retryable: false,
                }),
            ));
            return self.accept(request_id, events);
        }

        let prompt = self.agent_prompt(&route.agent_id, &route.content);
        let selected_model = self
            .agents
            .get(&route.agent_id)
            .map(|agent| agent.model.clone())
            .filter(|model| !model.trim().is_empty());
        let mut streamed_deltas = Vec::new();
        let stream_result = self.provider.stream_chat(
            ModelRequest::new(&prompt).with_selected_model(selected_model.as_deref()),
            &mut |event| {
                if let ModelStreamEvent::Delta(delta) = event {
                    streamed_deltas.push(delta);
                }
                Ok(())
            },
        );

        match stream_result {
            Ok(response) => {
                let model = Some(model_invocation_payload(&response.invocation));
                let deltas = if streamed_deltas.is_empty() {
                    response.deltas
                } else {
                    streamed_deltas
                };
                let mut final_content = String::new();
                for delta in deltas {
                    final_content.push_str(&delta);
                    events.push(self.session_event(
                        session_id.clone(),
                        CadisEvent::MessageDelta(MessageDeltaPayload {
                            delta,
                            content_kind,
                            agent_id: Some(route.agent_id.clone()),
                            agent_name: Some(route.agent_name.clone()),
                            model: model.clone(),
                        }),
                    ));
                }

                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::MessageCompleted(MessageCompletedPayload {
                        content_kind,
                        content: Some(final_content.clone()),
                        agent_id: Some(route.agent_id.clone()),
                        agent_name: Some(route.agent_name.clone()),
                        model,
                    }),
                ));
                if self.agent_session_timed_out(&agent_session_id) {
                    let error_message = format!(
                        "agent session exceeded default_timeout_sec={}",
                        self.agent_runtime.default_timeout_sec
                    );
                    if let Some(event) = self.fail_agent_session(
                        &agent_session_id,
                        AgentSessionStatus::TimedOut,
                        "agent_timeout",
                        error_message.clone(),
                    ) {
                        events.push(event);
                    }
                    events.push(self.session_event(
                        session_id.clone(),
                        CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                            agent_id: route.agent_id.clone(),
                            status: AgentStatus::Failed,
                            task: Some(error_message.clone()),
                        }),
                    ));
                    events.push(self.session_event(
                        session_id,
                        CadisEvent::SessionFailed(ErrorPayload {
                            code: "agent_timeout".to_owned(),
                            message: error_message,
                            retryable: true,
                        }),
                    ));
                    return self.accept(request_id, events);
                }
                if let Some(event) =
                    self.complete_agent_session(&agent_session_id, final_content.clone())
                {
                    events.push(event);
                }
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                        agent_id: route.agent_id.clone(),
                        status: AgentStatus::Completed,
                        task: None,
                    }),
                ));
                if route.agent_id.as_str() != "main" {
                    events.push(self.session_event(
                        session_id.clone(),
                        CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                            agent_id: AgentId::from("main"),
                            status: AgentStatus::Completed,
                            task: None,
                        }),
                    ));
                }
                if let Some(worker) = &worker {
                    events.extend(self.complete_worker(
                        &worker.worker_id,
                        "completed",
                        worker.summary.clone(),
                    ));
                }
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionCompleted(SessionEventPayload {
                        session_id,
                        title: None,
                    }),
                ));
            }
            Err(error) => {
                let error_message = error.message().to_owned();
                if let Some(event) = self.fail_agent_session(
                    &agent_session_id,
                    AgentSessionStatus::Failed,
                    error.code(),
                    error_message.clone(),
                ) {
                    events.push(event);
                }
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                        agent_id: route.agent_id.clone(),
                        status: AgentStatus::Failed,
                        task: Some(error_message.clone()),
                    }),
                ));
                if let Some(worker) = &worker {
                    events.extend(self.complete_worker(
                        &worker.worker_id,
                        "failed",
                        error_message.clone(),
                    ));
                }
                events.push(self.session_event(
                    session_id,
                    CadisEvent::SessionFailed(ErrorPayload {
                        code: error.code().to_owned(),
                        message: error_message,
                        retryable: error.retryable(),
                    }),
                ));
            }
        }

        self.accept(request_id, events)
    }

    fn worker_tail(&mut self, request_id: RequestId, request: WorkerTailRequest) -> RequestOutcome {
        let Some(worker) = self.workers.get(&request.worker_id).cloned() else {
            return self.reject(
                request_id,
                "worker_not_found",
                format!("worker '{}' was not found", request.worker_id),
                false,
            );
        };

        let line_limit = request
            .lines
            .and_then(|lines| usize::try_from(lines).ok())
            .unwrap_or(WORKER_TAIL_DEFAULT_LINES)
            .min(WORKER_TAIL_MAX_LINES);
        let start = worker.log_lines.len().saturating_sub(line_limit);
        let events = worker.log_lines[start..]
            .iter()
            .map(|line| {
                self.session_event(
                    worker.session_id.clone(),
                    CadisEvent::WorkerLogDelta(WorkerLogDeltaPayload {
                        worker_id: worker.worker_id.clone(),
                        delta: line.clone(),
                        agent_id: worker.agent_id.clone(),
                        parent_agent_id: worker.parent_agent_id.clone(),
                    }),
                )
            })
            .collect();

        self.accept(request_id, events)
    }

    fn spawn_agent(&mut self, request_id: RequestId, request: AgentSpawnRequest) -> RequestOutcome {
        let record = match self.spawn_agent_record(request) {
            Ok(record) => record,
            Err(error) => return self.reject(request_id, error.code, error.message, false),
        };

        let event = self.event(
            None,
            CadisEvent::AgentSpawned(record.clone().event_payload()),
        );
        let status = self.event(
            None,
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: record.id,
                status: AgentStatus::Idle,
                task: Some("spawned and ready".to_owned()),
            }),
        );
        self.accept(request_id, vec![event, status])
    }

    fn spawn_agent_record(
        &mut self,
        request: AgentSpawnRequest,
    ) -> Result<AgentRecord, RuntimeError> {
        let role = normalize_role(&request.role);
        if role.is_empty() {
            return Err(RuntimeError {
                code: "invalid_agent_role",
                message: "agent role is empty".to_owned(),
            });
        }

        let parent_agent_id = if let Some(parent_agent_id) = request.parent_agent_id {
            if !self.agents.contains_key(&parent_agent_id) {
                return Err(RuntimeError {
                    code: "parent_agent_not_found",
                    message: format!("parent agent '{parent_agent_id}' was not found"),
                });
            }
            Some(parent_agent_id)
        } else {
            Some(AgentId::from("main"))
        };
        let parent_agent_id = parent_agent_id.expect("spawn parent is always resolved");

        if self.agents.len() >= self.spawn_limits.max_total_agents {
            return Err(RuntimeError {
                code: "agent_spawn_total_limit_exceeded",
                message: format!(
                    "agent.spawn would exceed max_total_agents={}",
                    self.spawn_limits.max_total_agents
                ),
            });
        }

        let child_count = self.child_count(&parent_agent_id);
        if child_count >= self.spawn_limits.max_children_per_parent {
            return Err(RuntimeError {
                code: "agent_spawn_children_limit_exceeded",
                message: format!(
                    "parent agent '{parent_agent_id}' already has {child_count} children; max_children_per_parent={}",
                    self.spawn_limits.max_children_per_parent
                ),
            });
        }

        let child_depth = self.agent_depth(&parent_agent_id) + 1;
        if child_depth > self.spawn_limits.max_depth {
            return Err(RuntimeError {
                code: "agent_spawn_depth_limit_exceeded",
                message: format!(
                    "agent.spawn child depth {child_depth} would exceed max_depth={}",
                    self.spawn_limits.max_depth
                ),
            });
        }

        let agent_id = self.next_agent_id(&role);
        let display_name = request
            .display_name
            .as_deref()
            .map(|name| normalize_agent_name(name, &agent_id))
            .unwrap_or_else(|| default_agent_name(&role, &agent_id));
        let model = request
            .model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| self.options.model_provider.clone());
        let record = AgentRecord {
            id: agent_id.clone(),
            role,
            display_name,
            parent_agent_id: Some(parent_agent_id),
            model,
            status: AgentStatus::Idle,
        };
        self.agents.insert(agent_id.clone(), record.clone());
        let _ = self.persist_agent_record(&agent_id);
        Ok(record)
    }

    fn kill_agent(&mut self, request_id: RequestId, agent_id: AgentId) -> RequestOutcome {
        if agent_id.as_str() == "main" {
            return self.reject(
                request_id,
                "cannot_kill_main_agent",
                "the main orchestrator agent cannot be killed",
                false,
            );
        }
        let Some(mut record) = self.agents.remove(&agent_id) else {
            return self.reject(
                request_id,
                "agent_not_found",
                format!("agent '{agent_id}' was not found"),
                false,
            );
        };
        let _ = self.state_store.remove_agent_metadata(&agent_id);
        record.status = AgentStatus::Completed;
        let event = self.event(None, CadisEvent::AgentCompleted(record.event_payload()));
        self.accept(request_id, vec![event])
    }

    fn workspace_list(
        &mut self,
        request_id: RequestId,
        request: WorkspaceListRequest,
    ) -> RequestOutcome {
        let event = self.event(
            None,
            CadisEvent::WorkspaceListResponse(WorkspaceListPayload {
                workspaces: self.workspace_payloads(),
                grants: if request.include_grants {
                    self.workspace_grant_payloads()
                } else {
                    Vec::new()
                },
            }),
        );
        self.accept(request_id, vec![event])
    }

    fn workspace_register(
        &mut self,
        request_id: RequestId,
        request: WorkspaceRegisterRequest,
    ) -> RequestOutcome {
        let workspace_id = request.workspace_id;
        let root = match canonical_workspace_root(&request.root) {
            Ok(root) => root,
            Err(error) => {
                return self.reject(
                    request_id,
                    "invalid_workspace_root",
                    error.to_string(),
                    false,
                )
            }
        };
        if let Err(error) = validate_workspace_root(&root, &self.options.cadis_home) {
            return self.reject(request_id, error.code, error.message, error.retryable);
        }
        let record = WorkspaceRecord {
            id: workspace_id.clone(),
            kind: request.kind,
            root,
            aliases: normalize_aliases(request.aliases),
            vcs: request.vcs.filter(|value| !value.trim().is_empty()),
            trusted: request.trusted,
            worktree_root: request
                .worktree_root
                .filter(|value| !value.trim().is_empty()),
            artifact_root: request
                .artifact_root
                .filter(|value| !value.trim().is_empty()),
        };
        let mut workspaces = self.workspaces.clone();
        workspaces.insert(workspace_id, record.clone());
        if let Err(error) = save_workspace_registry(&self.profile_home, &workspaces) {
            return self.reject(
                request_id,
                "workspace_registry_persist_failed",
                format!("could not persist workspace registry: {error}"),
                true,
            );
        }
        self.workspaces = workspaces;

        let event = self.event(
            None,
            CadisEvent::WorkspaceRegistered(record.event_payload()),
        );
        self.accept(request_id, vec![event])
    }

    fn workspace_grant(
        &mut self,
        request_id: RequestId,
        request: WorkspaceGrantRequest,
    ) -> RequestOutcome {
        let Some(workspace) = self.workspaces.get(&request.workspace_id).cloned() else {
            return self.reject(
                request_id,
                "workspace_not_found",
                format!("workspace '{}' was not registered", request.workspace_id),
                false,
            );
        };

        let access = normalize_workspace_access(request.access);
        let grant_id = self.next_workspace_grant_id();
        let record = WorkspaceGrantRecord {
            grant_id: grant_id.clone(),
            agent_id: request.agent_id,
            workspace_id: workspace.id,
            root: workspace.root,
            access,
            created_at: now_timestamp(),
            expires_at: request.expires_at,
            source: request
                .source
                .filter(|source| !source.trim().is_empty())
                .unwrap_or_else(|| "user".to_owned()),
        };
        if let Err(error) = self.persist_workspace_grant(&record) {
            return self.reject(
                request_id,
                "workspace_grant_persist_failed",
                format!("could not persist workspace grant: {error}"),
                true,
            );
        }
        self.workspace_grants.insert(grant_id, record.clone());

        let event = self.event(
            None,
            CadisEvent::WorkspaceGrantCreated(record.event_payload()),
        );
        self.accept(request_id, vec![event])
    }

    fn workspace_revoke(
        &mut self,
        request_id: RequestId,
        request: WorkspaceRevokeRequest,
    ) -> RequestOutcome {
        if request.grant_id.is_none() && request.workspace_id.is_none() {
            return self.reject(
                request_id,
                "invalid_workspace_revoke",
                "workspace.revoke requires grant_id or workspace_id",
                false,
            );
        }

        let revoked_ids = self
            .workspace_grants
            .iter()
            .filter(|(grant_id, grant)| {
                request.grant_id.as_ref().is_none_or(|id| id == *grant_id)
                    && request
                        .workspace_id
                        .as_ref()
                        .is_none_or(|id| id == &grant.workspace_id)
                    && request
                        .agent_id
                        .as_ref()
                        .is_none_or(|id| grant.agent_id.as_ref() == Some(id))
            })
            .map(|(grant_id, _)| grant_id.clone())
            .collect::<Vec<_>>();

        if revoked_ids.is_empty() {
            return self.reject(
                request_id,
                "workspace_grant_not_found",
                "no matching workspace grant was found",
                false,
            );
        }

        let mut events = Vec::new();
        for grant_id in revoked_ids {
            if let Some(record) = self.workspace_grants.remove(&grant_id) {
                events.push(self.event(
                    None,
                    CadisEvent::WorkspaceGrantRevoked(record.event_payload()),
                ));
            }
        }
        if let Err(error) = self.persist_workspace_grants() {
            return self.reject(
                request_id,
                "workspace_grant_persist_failed",
                format!("could not persist workspace grant revocation: {error}"),
                true,
            );
        }
        self.accept(request_id, events)
    }

    fn workspace_doctor(
        &mut self,
        request_id: RequestId,
        request: WorkspaceDoctorRequest,
    ) -> RequestOutcome {
        let checks = self.workspace_doctor_checks(request);
        let event = self.event(
            None,
            CadisEvent::WorkspaceDoctorResponse(WorkspaceDoctorPayload { checks }),
        );
        self.accept(request_id, vec![event])
    }

    fn agent_prompt(&self, agent_id: &AgentId, content: &str) -> String {
        let Some(agent) = self.agents.get(agent_id) else {
            return content.to_owned();
        };
        if agent.id.as_str() == "main" {
            return content.to_owned();
        }
        format!(
            "You are {} ({}) in the CADIS multi-agent runtime. Answer only for your role and keep the response concise unless the user asks for detail.\n\nUser request:\n{}",
            agent.display_name, agent.role, content
        )
    }

    fn agent_records_sorted(&self) -> Vec<AgentRecord> {
        let mut agents = self.agents.values().cloned().collect::<Vec<_>>();
        agents.sort_by(|left, right| left.id.cmp(&right.id));
        agents
    }

    fn session_records_sorted(&self) -> Vec<(SessionId, SessionRecord)> {
        let mut sessions = self
            .sessions
            .iter()
            .map(|(session_id, session)| (session_id.clone(), session.clone()))
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.0.cmp(&right.0));
        sessions
    }

    fn worker_records_sorted(&self) -> Vec<WorkerRecord> {
        let mut workers = self.workers.values().cloned().collect::<Vec<_>>();
        workers.sort_by(|left, right| left.worker_id.cmp(&right.worker_id));
        workers
    }

    fn agent_session_records_sorted(&self) -> Vec<AgentSessionRecord> {
        let mut records = self.agent_sessions.values().cloned().collect::<Vec<_>>();
        records.sort_by(|left, right| left.id.cmp(&right.id));
        records
    }

    fn snapshot_events(&mut self) -> Vec<EventEnvelope> {
        let agents = self
            .agent_records_sorted()
            .into_iter()
            .map(AgentRecord::event_payload)
            .collect();
        let mut events = vec![
            self.event(
                None,
                CadisEvent::AgentListResponse(AgentListPayload { agents }),
            ),
            self.event(
                None,
                CadisEvent::WorkspaceListResponse(WorkspaceListPayload {
                    workspaces: self.workspace_payloads(),
                    grants: self.workspace_grant_payloads(),
                }),
            ),
            self.event(
                None,
                CadisEvent::UiPreferencesUpdated(UiPreferencesPayload {
                    preferences: self.ui_preferences.clone(),
                }),
            ),
        ];

        for diagnostic in self.recovery_diagnostics.clone() {
            events.push(self.event(
                None,
                CadisEvent::DaemonError(ErrorPayload {
                    code: diagnostic.code,
                    message: diagnostic.message,
                    retryable: false,
                }),
            ));
        }

        for (session_id, session) in self.session_records_sorted() {
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::SessionUpdated(SessionEventPayload {
                    session_id,
                    title: session.title,
                }),
            ));
        }

        for record in self.agent_session_records_sorted() {
            let event = match record.status {
                AgentSessionStatus::Completed => {
                    CadisEvent::AgentSessionCompleted(record.event_payload())
                }
                AgentSessionStatus::Failed
                | AgentSessionStatus::TimedOut
                | AgentSessionStatus::BudgetExceeded => {
                    CadisEvent::AgentSessionFailed(record.event_payload())
                }
                AgentSessionStatus::Cancelled => {
                    CadisEvent::AgentSessionCancelled(record.event_payload())
                }
                AgentSessionStatus::Started | AgentSessionStatus::Running => {
                    CadisEvent::AgentSessionUpdated(record.event_payload())
                }
            };
            events.push(self.session_event(record.session_id.clone(), event));
        }

        for worker in self.worker_records_sorted() {
            let event = if worker.status == "running" {
                CadisEvent::WorkerStarted(worker.event_payload())
            } else {
                CadisEvent::WorkerCompleted(worker.event_payload())
            };
            events.push(self.session_event(worker.session_id.clone(), event));
        }

        events
    }

    fn next_agent_id(&mut self, role: &str) -> AgentId {
        loop {
            let suffix = self.next_agent;
            self.next_agent += 1;
            let id = AgentId::from(format!("{}_{}", slugify(role), suffix));
            if !self.agents.contains_key(&id) {
                return id;
            }
        }
    }

    fn next_worker_id(&mut self) -> String {
        let worker_id = format!("worker_{:06}", self.next_worker);
        self.next_worker += 1;
        worker_id
    }

    fn next_agent_session_id(&mut self) -> AgentSessionId {
        let agent_session_id = AgentSessionId::from(format!("ags_{:06}", self.next_agent_session));
        self.next_agent_session += 1;
        agent_session_id
    }

    fn start_agent_session(
        &mut self,
        session_id: SessionId,
        route_id: String,
        agent_id: AgentId,
        task: String,
    ) -> (AgentSessionId, EventEnvelope) {
        let parent_agent_id = self
            .agents
            .get(&agent_id)
            .and_then(|agent| agent.parent_agent_id.clone());
        let id = self.next_agent_session_id();
        let record = AgentSessionRecord {
            id: id.clone(),
            session_id: session_id.clone(),
            route_id,
            agent_id,
            parent_agent_id,
            task,
            status: AgentSessionStatus::Running,
            timeout_at: timestamp_after_seconds(self.agent_runtime.default_timeout_sec),
            budget_steps: self.agent_runtime.max_steps_per_session,
            steps_used: 0,
            result: None,
            error_code: None,
            error: None,
            cancellation_requested_at: None,
        };
        let event = self.session_event(
            session_id,
            CadisEvent::AgentSessionStarted(record.event_payload()),
        );
        self.agent_sessions.insert(id.clone(), record);
        let _ = self.persist_agent_session_record(&id);
        (id, event)
    }

    fn consume_agent_session_step(
        &mut self,
        agent_session_id: &AgentSessionId,
    ) -> Option<EventEnvelope> {
        let (session_id, payload) = {
            let record = self.agent_sessions.get_mut(agent_session_id)?;
            if record.steps_used >= record.budget_steps {
                record.status = AgentSessionStatus::BudgetExceeded;
                record.error_code = Some("agent_budget_exceeded".to_owned());
                record.error = Some(format!(
                    "agent session exceeded max_steps_per_session={}",
                    record.budget_steps
                ));
                (
                    record.session_id.clone(),
                    CadisEvent::AgentSessionFailed(record.event_payload()),
                )
            } else {
                record.steps_used += 1;
                (
                    record.session_id.clone(),
                    CadisEvent::AgentSessionUpdated(record.event_payload()),
                )
            }
        };
        let _ = self.persist_agent_session_record(agent_session_id);
        Some(self.session_event(session_id, payload))
    }

    fn complete_agent_session(
        &mut self,
        agent_session_id: &AgentSessionId,
        result: impl Into<String>,
    ) -> Option<EventEnvelope> {
        let (session_id, payload) = {
            let record = self.agent_sessions.get_mut(agent_session_id)?;
            record.status = AgentSessionStatus::Completed;
            record.result = Some(redact(&result.into()));
            (record.session_id.clone(), record.event_payload())
        };
        let _ = self.persist_agent_session_record(agent_session_id);
        Some(self.session_event(session_id, CadisEvent::AgentSessionCompleted(payload)))
    }

    fn fail_agent_session(
        &mut self,
        agent_session_id: &AgentSessionId,
        status: AgentSessionStatus,
        code: impl Into<String>,
        error: impl Into<String>,
    ) -> Option<EventEnvelope> {
        let (session_id, payload) = {
            let record = self.agent_sessions.get_mut(agent_session_id)?;
            record.status = status;
            record.error_code = Some(code.into());
            record.error = Some(redact(&error.into()));
            (record.session_id.clone(), record.event_payload())
        };
        let _ = self.persist_agent_session_record(agent_session_id);
        Some(self.session_event(session_id, CadisEvent::AgentSessionFailed(payload)))
    }

    fn agent_session_timed_out(&self, agent_session_id: &AgentSessionId) -> bool {
        self.agent_sessions
            .get(agent_session_id)
            .is_some_and(|record| timestamp_is_past(&record.timeout_at))
    }

    fn cancel_agent_sessions_for_session(&mut self, session_id: &SessionId) -> Vec<EventEnvelope> {
        let agent_session_ids = self
            .agent_sessions
            .iter()
            .filter(|(_, record)| {
                &record.session_id == session_id && !agent_session_is_terminal(record.status)
            })
            .map(|(agent_session_id, _)| agent_session_id.clone())
            .collect::<Vec<_>>();
        let cancellation_requested_at = now_timestamp();

        let mut events = Vec::new();
        for agent_session_id in agent_session_ids {
            let Some((event_session_id, payload, agent_id)) = ({
                self.agent_sessions
                    .get_mut(&agent_session_id)
                    .map(|record| {
                        record.status = AgentSessionStatus::Cancelled;
                        record.error_code = Some("session_cancelled".to_owned());
                        record.error = Some("session was cancelled".to_owned());
                        record.cancellation_requested_at = Some(cancellation_requested_at.clone());
                        (
                            record.session_id.clone(),
                            record.event_payload(),
                            record.agent_id.clone(),
                        )
                    })
            }) else {
                continue;
            };
            let _ = self.persist_agent_session_record(&agent_session_id);
            events.push(self.session_event(
                event_session_id.clone(),
                CadisEvent::AgentSessionCancelled(payload),
            ));
            events.push(self.session_event(
                event_session_id,
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id,
                    status: AgentStatus::Cancelled,
                    task: Some("session cancelled".to_owned()),
                }),
            ));
        }
        events
    }

    fn start_worker(
        &mut self,
        session_id: SessionId,
        agent_id: AgentId,
        worker: &WorkerDelegation,
    ) -> Vec<EventEnvelope> {
        let record = WorkerRecord::from_delegation(session_id, Some(agent_id), worker);
        let worker_id = record.worker_id.clone();
        let mut events = vec![self.session_event(
            record.session_id.clone(),
            CadisEvent::WorkerStarted(record.event_payload()),
        )];
        self.workers.insert(worker_id.clone(), record);
        if let Some(event) =
            self.append_worker_log(&worker_id, format!("started: {}\n", worker.summary))
        {
            events.push(event);
        }
        events
    }

    fn complete_worker(
        &mut self,
        worker_id: &str,
        status: &str,
        summary: String,
    ) -> Vec<EventEnvelope> {
        let mut events = Vec::new();
        if let Some(event) = self.append_worker_log(worker_id, format!("{status}: {summary}\n")) {
            events.push(event);
        }
        if let Some(event) = self.update_worker_status(worker_id, status, Some(summary)) {
            events.push(event);
        }
        events
    }

    fn append_worker_log(
        &mut self,
        worker_id: &str,
        delta: impl Into<String>,
    ) -> Option<EventEnvelope> {
        let delta = redact(&delta.into());
        let (session_id, payload) = {
            let worker = self.workers.get_mut(worker_id)?;
            worker.log_lines.push(delta.clone());
            (
                worker.session_id.clone(),
                WorkerLogDeltaPayload {
                    worker_id: worker.worker_id.clone(),
                    delta,
                    agent_id: worker.agent_id.clone(),
                    parent_agent_id: worker.parent_agent_id.clone(),
                },
            )
        };

        Some(self.session_event(session_id, CadisEvent::WorkerLogDelta(payload)))
    }

    fn update_worker_status(
        &mut self,
        worker_id: &str,
        status: &str,
        summary: Option<String>,
    ) -> Option<EventEnvelope> {
        let (session_id, payload) = {
            let worker = self.workers.get_mut(worker_id)?;
            worker.status = status.to_owned();
            if let Some(summary) = summary {
                worker.summary = Some(summary);
            }
            (worker.session_id.clone(), worker.event_payload())
        };

        Some(self.session_event(session_id, CadisEvent::WorkerCompleted(payload)))
    }

    fn next_tool_call_id(&mut self) -> ToolCallId {
        let tool_call_id = ToolCallId::from(format!("tool_{:06}", self.next_tool));
        self.next_tool += 1;
        tool_call_id
    }

    fn next_approval_id(&mut self) -> ApprovalId {
        let approval_id = ApprovalId::from(format!("apr_{:06}", self.next_approval));
        self.next_approval += 1;
        approval_id
    }

    fn next_workspace_grant_id(&mut self) -> WorkspaceGrantId {
        let grant_id = WorkspaceGrantId::from(format!("grant_{:06}", self.next_workspace_grant));
        self.next_workspace_grant += 1;
        grant_id
    }

    fn workspace_payloads(&self) -> Vec<WorkspaceRecordPayload> {
        let mut records = self.workspaces.values().cloned().collect::<Vec<_>>();
        records.sort_by(|left, right| left.id.cmp(&right.id));
        records
            .into_iter()
            .map(WorkspaceRecord::event_payload)
            .collect()
    }

    fn workspace_grant_payloads(&self) -> Vec<WorkspaceGrantPayload> {
        let mut records = self.workspace_grants.values().cloned().collect::<Vec<_>>();
        records.sort_by(|left, right| left.grant_id.cmp(&right.grant_id));
        records
            .into_iter()
            .filter(|grant| !grant.is_expired())
            .map(WorkspaceGrantRecord::event_payload)
            .collect()
    }

    fn persist_workspace_grant(
        &self,
        record: &WorkspaceGrantRecord,
    ) -> Result<(), cadis_store::StoreError> {
        self.profile_home
            .workspace_grants()
            .append(&record.clone().into_store(self.profile_home.profile_id()))
    }

    fn persist_workspace_grants(&self) -> Result<(), cadis_store::StoreError> {
        let mut records = self.workspace_grants.values().cloned().collect::<Vec<_>>();
        records.sort_by(|left, right| left.grant_id.cmp(&right.grant_id));
        let records = records
            .into_iter()
            .map(|record| record.into_store(self.profile_home.profile_id()))
            .collect::<Vec<_>>();
        self.profile_home.workspace_grants().replace_all(&records)
    }

    fn workspace_doctor_checks(
        &self,
        request: WorkspaceDoctorRequest,
    ) -> Vec<WorkspaceDoctorCheck> {
        let mut checks = Vec::new();

        if self.workspaces.is_empty() {
            checks.push(WorkspaceDoctorCheck {
                name: "registry".to_owned(),
                status: "warn".to_owned(),
                message: "no workspaces are registered".to_owned(),
            });
        } else {
            checks.push(WorkspaceDoctorCheck {
                name: "registry".to_owned(),
                status: "ok".to_owned(),
                message: format!("{} workspace(s) registered", self.workspaces.len()),
            });
        }

        if let Some(workspace_id) = request.workspace_id {
            match self.workspaces.get(&workspace_id) {
                Some(workspace) => {
                    checks.push(root_check("workspace.root", &workspace.root));
                    let active_grants = self
                        .workspace_grants
                        .values()
                        .filter(|grant| grant.workspace_id == workspace_id && !grant.is_expired())
                        .count();
                    checks.push(WorkspaceDoctorCheck {
                        name: "workspace.grants".to_owned(),
                        status: if active_grants == 0 { "warn" } else { "ok" }.to_owned(),
                        message: format!("{active_grants} active grant(s)"),
                    });
                }
                None => checks.push(WorkspaceDoctorCheck {
                    name: "workspace.lookup".to_owned(),
                    status: "error".to_owned(),
                    message: format!("workspace '{workspace_id}' is not registered"),
                }),
            }
        }

        if let Some(root) = request.root {
            match canonical_workspace_root(&root) {
                Ok(root) => checks.push(root_check("request.root", &root)),
                Err(error) => checks.push(WorkspaceDoctorCheck {
                    name: "request.root".to_owned(),
                    status: "error".to_owned(),
                    message: error.to_string(),
                }),
            }
        }

        checks
    }

    fn resolve_tool_session(
        &mut self,
        requested_session_id: Option<SessionId>,
        input: &serde_json::Value,
    ) -> (SessionId, Vec<EventEnvelope>) {
        let cwd = tool_workspace_summary(input);
        match requested_session_id {
            Some(session_id) if self.sessions.contains_key(&session_id) => {
                if let Some(cwd) = cwd {
                    if let Some(session) = self.sessions.get_mut(&session_id) {
                        session._cwd = Some(cwd);
                    }
                }
                (session_id, Vec::new())
            }
            Some(session_id) => {
                let title = Some("Tool request".to_owned());
                self.sessions.insert(
                    session_id.clone(),
                    SessionRecord {
                        title: title.clone(),
                        _cwd: cwd,
                    },
                );
                let event = self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionStarted(SessionEventPayload {
                        session_id: session_id.clone(),
                        title,
                    }),
                );
                (session_id, vec![event])
            }
            None => {
                let session_id = self.create_session(Some("Tool request".to_owned()), cwd);
                let title = self
                    .sessions
                    .get(&session_id)
                    .and_then(|session| session.title.clone());
                let event = self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionStarted(SessionEventPayload {
                        session_id: session_id.clone(),
                        title,
                    }),
                );
                (session_id, vec![event])
            }
        }
    }

    fn session_workspace(&self, session_id: &SessionId) -> Option<String> {
        self.sessions
            .get(session_id)
            .and_then(|session| session._cwd.clone())
    }

    fn execute_safe_tool(
        &self,
        workspace: &Path,
        request: &ToolCallRequest,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        match request.tool_name.as_str() {
            tool_name if self.tools.is_auto_executable_safe_read(tool_name) => match tool_name {
                "file.read" => self.execute_file_read(workspace, &request.input),
                "file.search" => self.execute_file_search(workspace, &request.input),
                "git.status" => self.execute_git_status(workspace, &request.input),
                _ => Err(tool_error(
                    "tool_not_implemented",
                    format!("{tool_name} has no native execution backend"),
                    false,
                )),
            },
            _ => Err(tool_error(
                "tool_not_allowed",
                format!(
                    "{} is not an auto-allowed safe-read tool",
                    request.tool_name
                ),
                false,
            )),
        }
    }

    fn execute_file_read(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let path = input_string(input, "path")
            .ok_or_else(|| tool_error("invalid_tool_input", "file.read requires path", false))?;
        let path = resolve_inside_workspace(workspace, &path)?;
        let bytes = fs::read(&path).map_err(|error| {
            tool_error(
                "file_read_failed",
                format!("could not read {}: {error}", path.display()),
                false,
            )
        })?;
        let truncated = bytes.len() > FILE_READ_LIMIT_BYTES;
        let visible = if truncated {
            &bytes[..FILE_READ_LIMIT_BYTES]
        } else {
            &bytes
        };
        let content = redact(&String::from_utf8_lossy(visible));
        let relative = display_relative_path(workspace, &path);

        Ok(ToolExecutionResult {
            summary: content.clone(),
            output: serde_json::json!({
                "path": relative,
                "content": content,
                "truncated": truncated
            }),
        })
    }

    fn execute_file_search(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let query = input_string(input, "query")
            .ok_or_else(|| tool_error("invalid_tool_input", "file.search requires query", false))?;
        if query.is_empty() {
            return Err(tool_error(
                "invalid_tool_input",
                "file.search query cannot be empty",
                false,
            ));
        }
        let root = input_string(input, "path").unwrap_or_else(|| ".".to_owned());
        let root = resolve_inside_workspace(workspace, &root)?;
        let max_results = input_usize(input, "max_results").unwrap_or(FILE_SEARCH_DEFAULT_LIMIT);
        let mut matches = Vec::new();
        search_files(workspace, &root, &query, max_results, &mut matches);
        let truncated = matches.len() >= max_results;
        let summary = if matches.is_empty() {
            "no matches".to_owned()
        } else {
            matches
                .iter()
                .map(|entry| {
                    format!(
                        "{}:{}:{}",
                        entry.path,
                        entry.line_number,
                        redact(entry.line.trim())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let output_matches = matches
            .into_iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path,
                    "line_number": entry.line_number,
                    "line": redact(&entry.line)
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolExecutionResult {
            summary,
            output: serde_json::json!({
                "query": query,
                "matches": output_matches,
                "truncated": truncated
            }),
        })
    }

    fn execute_git_status(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let cwd = input_string(input, "path")
            .or_else(|| input_string(input, "cwd"))
            .unwrap_or_else(|| ".".to_owned());
        let cwd = resolve_inside_workspace(workspace, &cwd)?;
        let output = Command::new("git")
            .arg("-C")
            .arg(&cwd)
            .args(["status", "--short", "--branch"])
            .output()
            .map_err(|error| tool_error("git_status_failed", error.to_string(), false))?;
        if !output.status.success() {
            let stderr = redact(&String::from_utf8_lossy(&output.stderr));
            return Err(tool_error(
                "git_status_failed",
                if stderr.trim().is_empty() {
                    "git status failed".to_owned()
                } else {
                    stderr
                },
                false,
            ));
        }
        let stdout = redact(&String::from_utf8_lossy(&output.stdout));
        Ok(ToolExecutionResult {
            summary: stdout.clone(),
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "status": stdout
            }),
        })
    }

    fn resolved_granted_workspace(
        &self,
        session_id: &SessionId,
        agent_id: Option<&AgentId>,
        input: &serde_json::Value,
        required_access: WorkspaceAccess,
    ) -> Result<ResolvedWorkspace, ErrorPayload> {
        let workspace_ref = tool_workspace_id(input)
            .or_else(|| tool_workspace_summary(input))
            .or_else(|| self.session_workspace(session_id))
            .ok_or_else(|| {
                tool_error(
                    "workspace_required",
                    "tool call requires workspace_id, workspace, cwd, or a session workspace",
                    false,
                )
            })?;
        let workspace = self.resolve_registered_workspace(&workspace_ref)?;
        if self.has_workspace_grant(&workspace.id, agent_id, required_access) {
            Ok(ResolvedWorkspace {
                root: workspace.root.clone(),
            })
        } else {
            Err(tool_error(
                "workspace_grant_required",
                format!(
                    "no active {:?} grant for workspace '{}'",
                    required_access, workspace.id
                ),
                false,
            ))
        }
    }

    fn resolve_registered_workspace(
        &self,
        workspace_ref: &str,
    ) -> Result<&WorkspaceRecord, ErrorPayload> {
        let workspace_id = WorkspaceId::from(workspace_ref.to_owned());
        if let Some(workspace) = self.workspaces.get(&workspace_id) {
            return Ok(workspace);
        }

        if let Some(workspace) = self
            .workspaces
            .values()
            .find(|workspace| workspace.aliases.iter().any(|alias| alias == workspace_ref))
        {
            return Ok(workspace);
        }

        let candidate = PathBuf::from(workspace_ref);
        let Ok(root) = candidate.canonicalize() else {
            return Err(tool_error(
                "workspace_not_found",
                format!("workspace '{workspace_ref}' is not registered"),
                false,
            ));
        };
        self.workspaces
            .values()
            .find(|workspace| workspace.root == root)
            .ok_or_else(|| {
                tool_error(
                    "workspace_not_found",
                    format!("workspace root {} is not registered", root.display()),
                    false,
                )
            })
    }

    fn has_workspace_grant(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: Option<&AgentId>,
        required_access: WorkspaceAccess,
    ) -> bool {
        self.workspace_grants.values().any(|grant| {
            grant.workspace_id == *workspace_id
                && workspace_grant_matches_agent(grant.agent_id.as_ref(), agent_id)
                && !grant.is_expired()
                && workspace_access_allows(&grant.access, required_access)
        })
    }

    fn child_count(&self, parent_agent_id: &AgentId) -> usize {
        self.agents
            .values()
            .filter(|agent| agent.parent_agent_id.as_ref() == Some(parent_agent_id))
            .count()
    }

    fn agent_depth(&self, agent_id: &AgentId) -> usize {
        let mut depth = 0;
        let mut current = agent_id;
        while let Some(parent_agent_id) = self
            .agents
            .get(current)
            .and_then(|agent| agent.parent_agent_id.as_ref())
        {
            depth += 1;
            current = parent_agent_id;
        }
        depth
    }

    fn create_session(&mut self, title: Option<String>, cwd: Option<String>) -> SessionId {
        let session_id = SessionId::from(format!("ses_{:06}", self.next_session));
        self.next_session += 1;
        self.insert_session_record(session_id.clone(), SessionRecord { title, _cwd: cwd });
        session_id
    }

    fn insert_session_record(&mut self, session_id: SessionId, record: SessionRecord) {
        self.sessions.insert(session_id.clone(), record);
        let _ = self.persist_session_record(&session_id);
    }

    fn persist_session_record(
        &self,
        session_id: &SessionId,
    ) -> Result<(), cadis_store::StoreError> {
        if let Some(record) = self.sessions.get(session_id) {
            self.state_store.write_session_metadata(
                session_id,
                &SessionMetadata::from_record(session_id.clone(), record),
            )?;
        }
        Ok(())
    }

    fn persist_agent_record(&self, agent_id: &AgentId) -> Result<(), cadis_store::StoreError> {
        if let Some(record) = self.agents.get(agent_id) {
            self.state_store
                .write_agent_metadata(agent_id, &AgentMetadata::from_record(record))?;
        }
        Ok(())
    }

    fn persist_agent_session_record(
        &self,
        agent_session_id: &AgentSessionId,
    ) -> Result<(), cadis_store::StoreError> {
        if let Some(record) = self.agent_sessions.get(agent_session_id) {
            self.state_store.write_agent_session_metadata(
                agent_session_id,
                &AgentSessionMetadata::from_record(record),
            )?;
        }
        Ok(())
    }

    fn accept(&self, request_id: RequestId, events: Vec<EventEnvelope>) -> RequestOutcome {
        RequestOutcome {
            response: ResponseEnvelope::new(
                request_id.clone(),
                DaemonResponse::RequestAccepted(RequestAcceptedPayload { request_id }),
            ),
            events,
        }
    }

    fn reject(
        &self,
        request_id: RequestId,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> RequestOutcome {
        RequestOutcome {
            response: ResponseEnvelope::new(
                request_id,
                DaemonResponse::RequestRejected(ErrorPayload {
                    code: code.into(),
                    message: message.into(),
                    retryable,
                }),
            ),
            events: Vec::new(),
        }
    }

    fn session_event(&mut self, session_id: SessionId, event: CadisEvent) -> EventEnvelope {
        self.event(Some(session_id), event)
    }

    fn event(&mut self, session_id: Option<SessionId>, event: CadisEvent) -> EventEnvelope {
        let event_id = EventId::from(format!("evt_{:06}", self.next_event));
        self.next_event += 1;

        EventEnvelope::new(event_id, now_timestamp(), "cadisd", session_id, event)
    }
}

impl AgentSpawnLimits {
    fn from_options(options: &serde_json::Value) -> Self {
        let defaults = Self::default();
        let Some(agent_spawn) = options.get("agent_spawn") else {
            return defaults;
        };

        Self {
            max_depth: json_usize(agent_spawn, "max_depth").unwrap_or(defaults.max_depth),
            max_children_per_parent: json_usize(agent_spawn, "max_children_per_parent")
                .unwrap_or(defaults.max_children_per_parent),
            max_total_agents: json_usize(agent_spawn, "max_total_agents")
                .unwrap_or(defaults.max_total_agents),
        }
    }
}

impl AgentRuntimeConfig {
    fn from_options(options: &serde_json::Value) -> Self {
        let defaults = Self::default();
        let Some(agent_runtime) = options.get("agent_runtime") else {
            return defaults;
        };

        Self {
            default_timeout_sec: json_i64(agent_runtime, "default_timeout_sec")
                .filter(|value| *value > 0)
                .unwrap_or(defaults.default_timeout_sec),
            max_steps_per_session: json_u32(agent_runtime, "max_steps_per_session")
                .unwrap_or(defaults.max_steps_per_session),
        }
    }
}

impl OrchestratorConfig {
    fn from_options(options: &serde_json::Value) -> Self {
        let defaults = Self::default();
        let Some(orchestrator) = options.get("orchestrator") else {
            return defaults;
        };

        Self {
            worker_delegation_enabled: orchestrator
                .get("worker_delegation_enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(defaults.worker_delegation_enabled),
            default_worker_role: orchestrator
                .get("default_worker_role")
                .and_then(serde_json::Value::as_str)
                .map(normalize_role)
                .filter(|role| !role.is_empty())
                .unwrap_or(defaults.default_worker_role),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Orchestrator {
    config: OrchestratorConfig,
}

impl Orchestrator {
    fn new(config: OrchestratorConfig) -> Self {
        Self { config }
    }

    fn route_message(
        &self,
        explicit_agent_id: Option<AgentId>,
        content: &str,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Result<OrchestratorDecision, RouteError> {
        let route = if let Some(agent_id) = explicit_agent_id {
            let content = self.strip_matching_leading_mention(&agent_id, content, agents);
            self.route_to_agent(
                agent_id,
                content,
                "explicit target_agent_id".to_owned(),
                None,
                agents,
            )?
        } else if let Some((mention, remaining)) = leading_mention(content) {
            let Some(agent_id) = self.resolve_agent_mention(&mention, agents) else {
                return Err(RouteError {
                    code: "agent_not_found",
                    message: format!("no agent matches @{mention}"),
                });
            };
            self.route_to_agent(
                agent_id,
                remaining,
                format!("@{mention} mention"),
                None,
                agents,
            )?
        } else {
            self.route_to_agent(
                AgentId::from("main"),
                content.to_owned(),
                "default orchestrator".to_owned(),
                None,
                agents,
            )?
        };

        if route.agent_id.as_str() == "main" {
            self.apply_explicit_action(route, agents)
        } else {
            Ok(OrchestratorDecision::Route(route))
        }
    }

    fn apply_explicit_action(
        &self,
        route: RouteDecision,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Result<OrchestratorDecision, RouteError> {
        let Some(action) = parse_orchestrator_action(&route.content, &self.config) else {
            return Ok(OrchestratorDecision::Route(route));
        };

        if !self.config.worker_delegation_enabled {
            return Err(RouteError {
                code: "orchestrator_worker_delegation_disabled",
                message: "orchestrator worker delegation actions are disabled".to_owned(),
            });
        }

        match action {
            ExplicitOrchestratorAction::Route {
                mention,
                content,
                worker_summary,
            } => {
                let Some(agent_id) = self.resolve_agent_mention(&mention, agents) else {
                    return Err(RouteError {
                        code: "agent_not_found",
                        message: format!("no agent matches @{mention}"),
                    });
                };
                self.route_to_agent(
                    agent_id,
                    content,
                    format!("orchestrator action: route @{mention}"),
                    Some(worker_summary),
                    agents,
                )
                .map(OrchestratorDecision::Route)
            }
            ExplicitOrchestratorAction::Spawn {
                role,
                content,
                worker_summary,
            } => Ok(OrchestratorDecision::SpawnAndRoute(SpawnRouteDecision {
                role,
                content,
                reason: "orchestrator action: spawn worker".to_owned(),
                worker_summary,
            })),
        }
    }

    fn route_to_agent(
        &self,
        agent_id: AgentId,
        content: String,
        reason: String,
        worker_summary: Option<String>,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Result<RouteDecision, RouteError> {
        let Some(agent) = agents.get(&agent_id) else {
            return Err(RouteError {
                code: "agent_not_found",
                message: format!("agent '{agent_id}' was not found"),
            });
        };
        Ok(RouteDecision {
            agent_id: agent.id.clone(),
            agent_name: agent.display_name.clone(),
            content: normalize_route_content(content),
            reason,
            worker_summary,
        })
    }

    fn resolve_agent_mention(
        &self,
        mention: &str,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Option<AgentId> {
        let normalized = normalize_lookup(mention);
        agents
            .values()
            .find(|agent| {
                [
                    agent.id.as_str(),
                    agent.display_name.as_str(),
                    agent.role.as_str(),
                ]
                .into_iter()
                .any(|candidate| normalize_lookup(candidate) == normalized)
            })
            .map(|agent| agent.id.clone())
    }

    fn strip_matching_leading_mention(
        &self,
        agent_id: &AgentId,
        content: &str,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> String {
        let Some((mention, remaining)) = leading_mention(content) else {
            return content.to_owned();
        };
        if self.resolve_agent_mention(&mention, agents).as_ref() == Some(agent_id) {
            remaining
        } else {
            content.to_owned()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionRecord {
    title: Option<String>,
    _cwd: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct SessionMetadata {
    session_id: SessionId,
    title: Option<String>,
    cwd: Option<String>,
}

impl SessionMetadata {
    fn from_record(session_id: SessionId, record: &SessionRecord) -> Self {
        Self {
            session_id,
            title: record.title.clone(),
            cwd: record._cwd.clone(),
        }
    }

    fn into_record(self) -> (SessionId, SessionRecord) {
        (
            self.session_id,
            SessionRecord {
                title: self.title,
                _cwd: self.cwd,
            },
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentRecord {
    id: AgentId,
    role: String,
    display_name: String,
    parent_agent_id: Option<AgentId>,
    model: String,
    status: AgentStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct AgentMetadata {
    agent_id: AgentId,
    role: String,
    display_name: String,
    parent_agent_id: Option<AgentId>,
    model: String,
    status: AgentStatus,
}

impl AgentMetadata {
    fn from_record(record: &AgentRecord) -> Self {
        Self {
            agent_id: record.id.clone(),
            role: record.role.clone(),
            display_name: record.display_name.clone(),
            parent_agent_id: record.parent_agent_id.clone(),
            model: record.model.clone(),
            status: record.status,
        }
    }

    fn into_record(self) -> (AgentId, AgentRecord) {
        let id = self.agent_id;
        (
            id.clone(),
            AgentRecord {
                id,
                role: self.role,
                display_name: self.display_name,
                parent_agent_id: self.parent_agent_id,
                model: self.model,
                status: self.status,
            },
        )
    }
}

impl AgentRecord {
    fn event_payload(self) -> AgentEventPayload {
        AgentEventPayload {
            agent_id: self.id,
            role: Some(self.role),
            display_name: Some(self.display_name),
            parent_agent_id: self.parent_agent_id,
            model: Some(self.model),
            status: Some(self.status),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeRecoveryDiagnostic {
    code: String,
    message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentSessionRecovery {
    records: HashMap<AgentSessionId, AgentSessionRecord>,
    diagnostics: Vec<RuntimeRecoveryDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentSessionRecord {
    id: AgentSessionId,
    session_id: SessionId,
    route_id: String,
    agent_id: AgentId,
    parent_agent_id: Option<AgentId>,
    task: String,
    status: AgentSessionStatus,
    timeout_at: Timestamp,
    budget_steps: u32,
    steps_used: u32,
    result: Option<String>,
    error_code: Option<String>,
    error: Option<String>,
    cancellation_requested_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct AgentSessionMetadata {
    agent_session_id: AgentSessionId,
    session_id: SessionId,
    route_id: String,
    agent_id: AgentId,
    parent_agent_id: Option<AgentId>,
    task: String,
    status: AgentSessionStatus,
    timeout_at: Timestamp,
    budget_steps: u32,
    steps_used: u32,
    result: Option<String>,
    error_code: Option<String>,
    error: Option<String>,
    cancellation_requested_at: Option<Timestamp>,
}

impl AgentSessionMetadata {
    fn from_record(record: &AgentSessionRecord) -> Self {
        Self {
            agent_session_id: record.id.clone(),
            session_id: record.session_id.clone(),
            route_id: record.route_id.clone(),
            agent_id: record.agent_id.clone(),
            parent_agent_id: record.parent_agent_id.clone(),
            task: record.task.clone(),
            status: record.status,
            timeout_at: record.timeout_at.clone(),
            budget_steps: record.budget_steps,
            steps_used: record.steps_used,
            result: record.result.clone(),
            error_code: record.error_code.clone(),
            error: record.error.clone(),
            cancellation_requested_at: record.cancellation_requested_at.clone(),
        }
    }

    fn into_record(self) -> (AgentSessionId, AgentSessionRecord) {
        let id = self.agent_session_id;
        (
            id.clone(),
            AgentSessionRecord {
                id,
                session_id: self.session_id,
                route_id: self.route_id,
                agent_id: self.agent_id,
                parent_agent_id: self.parent_agent_id,
                task: self.task,
                status: self.status,
                timeout_at: self.timeout_at,
                budget_steps: self.budget_steps,
                steps_used: self.steps_used,
                result: self.result,
                error_code: self.error_code,
                error: self.error,
                cancellation_requested_at: self.cancellation_requested_at,
            },
        )
    }
}

impl AgentSessionRecord {
    fn event_payload(&self) -> AgentSessionEventPayload {
        AgentSessionEventPayload {
            agent_session_id: self.id.clone(),
            session_id: self.session_id.clone(),
            route_id: self.route_id.clone(),
            agent_id: self.agent_id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            task: self.task.clone(),
            status: self.status,
            timeout_at: self.timeout_at.clone(),
            budget_steps: self.budget_steps,
            steps_used: self.steps_used,
            result: self.result.clone(),
            error_code: self.error_code.clone(),
            error: self.error.clone(),
            cancellation_requested_at: self.cancellation_requested_at.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceRecord {
    id: WorkspaceId,
    kind: WorkspaceKind,
    root: PathBuf,
    aliases: Vec<String>,
    vcs: Option<String>,
    trusted: bool,
    worktree_root: Option<String>,
    artifact_root: Option<String>,
}

impl WorkspaceRecord {
    fn event_payload(self) -> WorkspaceRecordPayload {
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

    fn into_store(self) -> WorkspaceMetadata {
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
struct WorkspaceGrantRecord {
    grant_id: WorkspaceGrantId,
    agent_id: Option<AgentId>,
    workspace_id: WorkspaceId,
    root: PathBuf,
    access: Vec<WorkspaceAccess>,
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
    source: String,
}

impl WorkspaceGrantRecord {
    fn event_payload(self) -> WorkspaceGrantPayload {
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

    fn into_store(self, profile_id: &str) -> StoreWorkspaceGrantRecord {
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

    fn is_expired(&self) -> bool {
        self.expires_at
            .as_ref()
            .and_then(|expires_at| DateTime::parse_from_rfc3339(expires_at.as_str()).ok())
            .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedWorkspace {
    root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RouteDecision {
    agent_id: AgentId,
    agent_name: String,
    content: String,
    reason: String,
    worker_summary: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SpawnRouteDecision {
    role: String,
    content: String,
    reason: String,
    worker_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OrchestratorDecision {
    Route(RouteDecision),
    SpawnAndRoute(SpawnRouteDecision),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExplicitOrchestratorAction {
    Route {
        mention: String,
        content: String,
        worker_summary: String,
    },
    Spawn {
        role: String,
        content: String,
        worker_summary: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerDelegation {
    worker_id: String,
    parent_agent_id: Option<AgentId>,
    summary: String,
    worktree: WorkerWorktreeIntent,
    artifacts: WorkerArtifactLocations,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerRecord {
    worker_id: String,
    session_id: SessionId,
    agent_id: Option<AgentId>,
    parent_agent_id: Option<AgentId>,
    status: String,
    cli: Option<String>,
    cwd: Option<String>,
    summary: Option<String>,
    worktree: Option<WorkerWorktreeIntent>,
    artifacts: Option<WorkerArtifactLocations>,
    log_lines: Vec<String>,
}

impl WorkerRecord {
    fn from_delegation(
        session_id: SessionId,
        agent_id: Option<AgentId>,
        worker: &WorkerDelegation,
    ) -> Self {
        Self {
            worker_id: worker.worker_id.clone(),
            session_id,
            agent_id,
            parent_agent_id: worker.parent_agent_id.clone(),
            status: "running".to_owned(),
            cli: None,
            cwd: None,
            summary: Some(worker.summary.clone()),
            worktree: Some(worker.worktree.clone()),
            artifacts: Some(worker.artifacts.clone()),
            log_lines: Vec::new(),
        }
    }

    fn event_payload(&self) -> WorkerEventPayload {
        WorkerEventPayload {
            worker_id: self.worker_id.clone(),
            agent_id: self.agent_id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            status: Some(self.status.clone()),
            cli: self.cli.clone(),
            cwd: self.cwd.clone(),
            summary: self.summary.clone(),
            worktree: self.worktree.clone(),
            artifacts: self.artifacts.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingApproval {
    record: ApprovalRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolRegistry {
    definitions: Vec<ToolDefinition>,
}

impl ToolRegistry {
    fn new(definitions: Vec<ToolDefinition>) -> Result<Self, RuntimeError> {
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
        }

        Ok(Self { definitions })
    }

    fn builtin() -> Result<Self, RuntimeError> {
        Self::new(vec![
            ToolDefinition::safe_read("file.read", ToolInputSchema::FileRead),
            ToolDefinition::safe_read("file.search", ToolInputSchema::FileSearch),
            ToolDefinition::safe_read("git.status", ToolInputSchema::GitStatus),
            ToolDefinition::approval_placeholder(
                "file.write",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::WorkspaceMutation,
            ),
            ToolDefinition::approval_placeholder(
                "file.patch",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::WorkspaceMutation,
            ),
            ToolDefinition::approval_placeholder(
                "git.diff",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::GitMutation,
            ),
            ToolDefinition::approval_placeholder(
                "git.worktree.create",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::GitMutation,
            ),
            ToolDefinition::approval_placeholder(
                "git.worktree.remove",
                cadis_protocol::RiskClass::WorkspaceEdit,
                ToolInputSchema::GitMutation,
            ),
            ToolDefinition::approval_placeholder(
                "shell.run",
                cadis_protocol::RiskClass::SystemChange,
                ToolInputSchema::ShellRun,
            ),
        ])
    }

    fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.name == name)
    }

    fn is_auto_executable_safe_read(&self, name: &str) -> bool {
        self.get(name)
            .is_some_and(|definition| definition.execution == ToolExecutionMode::AutoExecute)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolDefinition {
    name: &'static str,
    risk_class: cadis_protocol::RiskClass,
    input_schema: ToolInputSchema,
    execution: ToolExecutionMode,
}

impl ToolDefinition {
    fn safe_read(name: &'static str, input_schema: ToolInputSchema) -> Self {
        Self {
            name,
            risk_class: cadis_protocol::RiskClass::SafeRead,
            input_schema,
            execution: ToolExecutionMode::AutoExecute,
        }
    }

    fn approval_placeholder(
        name: &'static str,
        risk_class: cadis_protocol::RiskClass,
        input_schema: ToolInputSchema,
    ) -> Self {
        Self {
            name,
            risk_class,
            input_schema,
            execution: ToolExecutionMode::ApprovalPlaceholder,
        }
    }

    fn policy_reason(&self) -> String {
        match self.execution {
            ToolExecutionMode::AutoExecute => format!(
                "{} is a read-only tool using {:?} input schema",
                self.name, self.input_schema
            ),
            ToolExecutionMode::ApprovalPlaceholder => format!(
                "{} requires approval for {:?} risk using {:?} input schema",
                self.name, self.risk_class, self.input_schema
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolInputSchema {
    FileRead,
    FileSearch,
    GitStatus,
    ShellRun,
    WorkspaceMutation,
    GitMutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolExecutionMode {
    AutoExecute,
    ApprovalPlaceholder,
}

#[derive(Clone, Debug, PartialEq)]
struct ToolExecutionResult {
    summary: String,
    output: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SearchMatch {
    path: String,
    line_number: usize,
    line: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RouteError {
    code: &'static str,
    message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeError {
    code: &'static str,
    message: String,
}

fn model_descriptor_from_catalog_entry(entry: ProviderCatalogEntry) -> ModelDescriptor {
    ModelDescriptor {
        provider: entry.provider,
        model: entry.model,
        display_name: entry.display_name,
        capabilities: entry.capabilities,
        readiness: Some(protocol_readiness(entry.readiness)),
        effective_provider: Some(entry.effective_provider),
        effective_model: Some(entry.effective_model),
        fallback: entry.fallback,
    }
}

fn protocol_readiness(readiness: ProviderReadiness) -> ModelReadiness {
    match readiness {
        ProviderReadiness::Ready => ModelReadiness::Ready,
        ProviderReadiness::Fallback => ModelReadiness::Fallback,
        ProviderReadiness::RequiresConfiguration => ModelReadiness::RequiresConfiguration,
        ProviderReadiness::Unavailable => ModelReadiness::Unavailable,
    }
}

fn recover_session_records(state_store: &StateStore) -> HashMap<SessionId, SessionRecord> {
    state_store
        .recover_session_metadata::<SessionMetadata>()
        .map(|recovery| recovery.records)
        .unwrap_or_default()
        .into_iter()
        .map(|record| record.metadata.into_record())
        .collect()
}

fn recover_agent_records(state_store: &StateStore) -> HashMap<AgentId, AgentRecord> {
    state_store
        .recover_agent_metadata::<AgentMetadata>()
        .map(|recovery| recovery.records)
        .unwrap_or_default()
        .into_iter()
        .map(|record| record.metadata.into_record())
        .collect()
}

fn recover_agent_session_records(state_store: &StateStore) -> AgentSessionRecovery {
    match state_store.recover_agent_session_metadata::<AgentSessionMetadata>() {
        Ok(recovery) => AgentSessionRecovery {
            records: recovery
                .records
                .into_iter()
                .map(|record| record.metadata.into_record())
                .collect(),
            diagnostics: recovery
                .diagnostics
                .into_iter()
                .map(agent_session_recovery_diagnostic)
                .collect(),
        },
        Err(error) => AgentSessionRecovery {
            records: HashMap::new(),
            diagnostics: vec![RuntimeRecoveryDiagnostic {
                code: "agent_session_recovery_failed".to_owned(),
                message: redact(&format!(
                    "could not scan durable AgentSession metadata: {error}"
                )),
            }],
        },
    }
}

fn agent_session_recovery_diagnostic(
    diagnostic: StateRecoveryDiagnostic,
) -> RuntimeRecoveryDiagnostic {
    let file_name = diagnostic
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    RuntimeRecoveryDiagnostic {
        code: "agent_session_recovery_skipped".to_owned(),
        message: redact(&format!(
            "skipped durable AgentSession metadata state/agent-sessions/{file_name}: {}",
            diagnostic.reason
        )),
    }
}

fn next_session_counter(sessions: &HashMap<SessionId, SessionRecord>) -> u64 {
    sessions
        .keys()
        .filter_map(|session_id| session_id.as_str().strip_prefix("ses_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn next_agent_session_counter(agent_sessions: &HashMap<AgentSessionId, AgentSessionRecord>) -> u64 {
    agent_sessions
        .keys()
        .filter_map(|agent_session_id| agent_session_id.as_str().strip_prefix("ags_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn next_route_counter(agent_sessions: &HashMap<AgentSessionId, AgentSessionRecord>) -> u64 {
    agent_sessions
        .values()
        .filter_map(|record| record.route_id.strip_prefix("route_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn next_agent_counter(agents: &HashMap<AgentId, AgentRecord>) -> u64 {
    agents
        .keys()
        .filter_map(|agent_id| agent_id.as_str().rsplit_once('_'))
        .filter_map(|(_, suffix)| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn load_workspace_registry(profile_home: &ProfileHome) -> HashMap<WorkspaceId, WorkspaceRecord> {
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

fn save_workspace_registry(
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

fn load_workspace_grants(
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

fn workspace_record_from_store(record: WorkspaceMetadata) -> Option<WorkspaceRecord> {
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

fn workspace_grant_record_from_store(
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

fn next_workspace_grant_counter(grants: &HashMap<WorkspaceGrantId, WorkspaceGrantRecord>) -> u64 {
    grants
        .keys()
        .filter_map(|grant_id| grant_id.as_str().strip_prefix("grant_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn model_invocation_payload(invocation: &ModelInvocation) -> ModelInvocationPayload {
    ModelInvocationPayload {
        requested_model: invocation.requested_model.clone(),
        effective_provider: invocation.effective_provider.clone(),
        effective_model: invocation.effective_model.clone(),
        fallback: invocation.fallback,
        fallback_reason: invocation.fallback_reason.clone(),
    }
}

fn default_agents(model_provider: &str) -> HashMap<AgentId, AgentRecord> {
    [
        ("main", "Orchestrator", "CADIS"),
        ("codex", "Coding", "Codex"),
        ("atlas", "Research", "Atlas"),
        ("forge", "Automation", "Forge"),
        ("sentry", "System", "Sentry"),
        ("bash", "Shell", "Bash"),
        ("mneme", "Memory", "Mneme"),
        ("chronos", "Schedule", "Chronos"),
        ("muse", "Creative", "Muse"),
        ("relay", "Network", "Relay"),
        ("prism", "Data", "Prism"),
        ("aegis", "Security", "Aegis"),
        ("echo", "Voice I/O", "Echo"),
    ]
    .into_iter()
    .map(|(id, role, display_name)| {
        let id = AgentId::from(id);
        (
            id.clone(),
            AgentRecord {
                id,
                role: role.to_owned(),
                display_name: display_name.to_owned(),
                parent_agent_id: None,
                model: model_provider.to_owned(),
                status: AgentStatus::Idle,
            },
        )
    })
    .collect()
}

fn now_timestamp() -> Timestamp {
    let value = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    Timestamp::new_utc(value).expect("chrono UTC timestamp should satisfy protocol")
}

fn timestamp_after_minutes(minutes: i64) -> Timestamp {
    let value =
        (Utc::now() + Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Secs, true);
    Timestamp::new_utc(value).expect("chrono UTC timestamp should satisfy protocol")
}

fn timestamp_after_seconds(seconds: i64) -> Timestamp {
    let value =
        (Utc::now() + Duration::seconds(seconds)).to_rfc3339_opts(SecondsFormat::Secs, true);
    Timestamp::new_utc(value).expect("chrono UTC timestamp should satisfy protocol")
}

fn timestamp_is_past(timestamp: &Timestamp) -> bool {
    DateTime::parse_from_rfc3339(timestamp.as_str())
        .map(|timestamp| timestamp.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

fn approval_is_expired(record: &ApprovalRecord) -> bool {
    DateTime::parse_from_rfc3339(record.expires_at.as_str())
        .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

fn input_string(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn input_usize(input: &serde_json::Value, key: &str) -> Option<usize> {
    input
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn tool_workspace_summary(input: &serde_json::Value) -> Option<String> {
    tool_workspace_id(input)
        .or_else(|| input_string(input, "workspace"))
        .or_else(|| input_string(input, "cwd"))
}

fn tool_workspace_id(input: &serde_json::Value) -> Option<String> {
    input_string(input, "workspace_id")
}

fn required_tool_access(tool_name: &str) -> WorkspaceAccess {
    match tool_name {
        "shell.run" => WorkspaceAccess::Exec,
        "file.write" | "file.patch" => WorkspaceAccess::Write,
        _ => WorkspaceAccess::Read,
    }
}

fn normalize_workspace_access(access: Vec<WorkspaceAccess>) -> Vec<WorkspaceAccess> {
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

fn workspace_access_allows(granted: &[WorkspaceAccess], required: WorkspaceAccess) -> bool {
    granted.contains(&WorkspaceAccess::Admin)
        || granted.contains(&required)
        || (required == WorkspaceAccess::Read && granted.contains(&WorkspaceAccess::Write))
}

fn workspace_grant_matches_agent(
    grant_agent_id: Option<&AgentId>,
    request_agent_id: Option<&AgentId>,
) -> bool {
    grant_agent_id.is_none() || grant_agent_id == request_agent_id
}

fn store_workspace_kind(kind: WorkspaceKind) -> StoreWorkspaceKind {
    match kind {
        WorkspaceKind::Project => StoreWorkspaceKind::Project,
        WorkspaceKind::Documents => StoreWorkspaceKind::Documents,
        WorkspaceKind::Sandbox => StoreWorkspaceKind::Sandbox,
        WorkspaceKind::Worktree => StoreWorkspaceKind::Worktree,
    }
}

fn protocol_workspace_kind(kind: StoreWorkspaceKind) -> WorkspaceKind {
    match kind {
        StoreWorkspaceKind::Project => WorkspaceKind::Project,
        StoreWorkspaceKind::Documents => WorkspaceKind::Documents,
        StoreWorkspaceKind::Sandbox => WorkspaceKind::Sandbox,
        StoreWorkspaceKind::Worktree => WorkspaceKind::Worktree,
    }
}

fn store_workspace_access(access: WorkspaceAccess) -> StoreWorkspaceAccess {
    match access {
        WorkspaceAccess::Read => StoreWorkspaceAccess::Read,
        WorkspaceAccess::Write => StoreWorkspaceAccess::Write,
        WorkspaceAccess::Exec => StoreWorkspaceAccess::Exec,
        WorkspaceAccess::Admin => StoreWorkspaceAccess::Admin,
    }
}

fn protocol_workspace_access(access: StoreWorkspaceAccess) -> WorkspaceAccess {
    match access {
        StoreWorkspaceAccess::Read => WorkspaceAccess::Read,
        StoreWorkspaceAccess::Write => WorkspaceAccess::Write,
        StoreWorkspaceAccess::Exec => WorkspaceAccess::Exec,
        StoreWorkspaceAccess::Admin => WorkspaceAccess::Admin,
    }
}

fn store_workspace_vcs(vcs: Option<&str>) -> WorkspaceVcs {
    match vcs.unwrap_or_default().trim().to_lowercase().as_str() {
        "git" => WorkspaceVcs::Git,
        _ => WorkspaceVcs::None,
    }
}

fn store_grant_source(source: &str) -> StoreGrantSource {
    match source.trim().to_lowercase().as_str() {
        "route" => StoreGrantSource::Route,
        "policy" => StoreGrantSource::Policy,
        "worker_spawn" | "worker-spawn" => StoreGrantSource::WorkerSpawn,
        _ => StoreGrantSource::User,
    }
}

fn protocol_grant_source(source: StoreGrantSource) -> &'static str {
    match source {
        StoreGrantSource::Route => "route",
        StoreGrantSource::User => "user",
        StoreGrantSource::Policy => "policy",
        StoreGrantSource::WorkerSpawn => "worker_spawn",
    }
}

fn normalize_aliases(aliases: Vec<String>) -> Vec<String> {
    let mut aliases = aliases
        .into_iter()
        .map(|alias| alias.trim().to_owned())
        .filter(|alias| !alias.is_empty())
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

fn canonical_workspace_root(root: &str) -> Result<PathBuf, std::io::Error> {
    let path = if let Some(rest) = root.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(rest)
    } else {
        PathBuf::from(root)
    };
    path.canonicalize()
}

fn validate_workspace_root(root: &Path, cadis_home: &Path) -> Result<(), ErrorPayload> {
    if root.parent().is_none() {
        return Err(tool_error(
            "workspace_root_too_broad",
            "workspace root cannot be the filesystem root",
            false,
        ));
    }

    if ["/etc", "/dev", "/proc", "/sys", "/run"]
        .iter()
        .any(|path| root == Path::new(path))
    {
        return Err(tool_error(
            "workspace_root_denied",
            format!(
                "workspace root {} is a protected system path",
                root.display()
            ),
            false,
        ));
    }

    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok())
    {
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

fn root_check(name: &str, root: &Path) -> WorkspaceDoctorCheck {
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

fn tool_command_summary(tool_name: &str, input: &serde_json::Value) -> Option<String> {
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
        _ => input_string(input, "path"),
    }
    .map(|value| redact(&value))
}

fn resolve_inside_workspace(workspace: &Path, user_path: &str) -> Result<PathBuf, ErrorPayload> {
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

fn display_relative_path(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn search_files(
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

fn search_file(
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

fn tool_error(
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

fn title_from_message(message: &str) -> String {
    let title = message
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "Untitled session".to_owned()
    } else {
        title
    }
}

fn normalize_agent_name(value: &str, agent_id: &AgentId) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let fallback = if agent_id.as_str() == "main" {
        "CADIS"
    } else {
        agent_id.as_str()
    };
    let name = if normalized.is_empty() {
        fallback.to_owned()
    } else {
        normalized
    };
    name.chars().take(32).collect()
}

fn normalize_role(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn default_agent_name(role: &str, agent_id: &AgentId) -> String {
    let name = role
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    chars.as_str().to_ascii_lowercase()
                ),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    normalize_agent_name(&name, agent_id)
}

fn leading_mention(content: &str) -> Option<(String, String)> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let mut end = 0;
    for (index, character) in rest.char_indices() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            end = index + character.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let mention = rest[..end].to_owned();
    let remaining = rest[end..].trim_start().to_owned();
    Some((mention, remaining))
}

fn parse_orchestrator_action(
    content: &str,
    config: &OrchestratorConfig,
) -> Option<ExplicitOrchestratorAction> {
    let trimmed = content.trim_start();
    let (command, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    let command = command.strip_prefix('/')?.to_ascii_lowercase();
    let rest = rest.trim_start();

    match command.as_str() {
        "route" | "delegate" => {
            let (mention, remaining) = leading_mention(rest)?;
            let content = normalize_route_content(remaining);
            let worker_summary = format!("Route @{mention}: {}", summarize_task(&content));
            Some(ExplicitOrchestratorAction::Route {
                mention,
                content,
                worker_summary,
            })
        }
        "spawn" => {
            let (role, content) = parse_spawn_action(rest, true, &config.default_worker_role)?;
            let content = normalize_route_content(content);
            let worker_summary = format!("Spawn {role}: {}", summarize_task(&content));
            Some(ExplicitOrchestratorAction::Spawn {
                role,
                content,
                worker_summary,
            })
        }
        "worker" => {
            let (role, content) = parse_spawn_action(rest, false, &config.default_worker_role)?;
            let content = normalize_route_content(content);
            let worker_summary = format!("Worker {role}: {}", summarize_task(&content));
            Some(ExplicitOrchestratorAction::Spawn {
                role,
                content,
                worker_summary,
            })
        }
        _ => None,
    }
}

fn parse_spawn_action(
    rest: &str,
    require_role: bool,
    default_worker_role: &str,
) -> Option<(String, String)> {
    let rest = rest.trim();
    if rest.is_empty() {
        if require_role {
            return None;
        }
        return Some((default_worker_role.to_owned(), String::new()));
    }

    if let Some((role, content)) = rest.split_once(':') {
        let role = normalize_role(role);
        if role.is_empty() {
            return None;
        }
        return Some((role, content.trim_start().to_owned()));
    }

    if require_role {
        Some((normalize_role(rest), String::new()))
    } else {
        Some((default_worker_role.to_owned(), rest.to_owned()))
    }
}

fn normalize_route_content(content: String) -> String {
    let content = content.trim().to_owned();
    if content.is_empty() {
        "Continue.".to_owned()
    } else {
        content
    }
}

fn summarize_task(content: &str) -> String {
    let summary = content
        .split_whitespace()
        .take(12)
        .collect::<Vec<_>>()
        .join(" ");
    if summary.is_empty() {
        "Continue.".to_owned()
    } else {
        summary
    }
}

fn planned_worker_worktree(
    worker_id: &str,
    workspace: Option<&str>,
    task: &str,
) -> WorkerWorktreeIntent {
    let worktree_root = workspace
        .map(|workspace| {
            Path::new(workspace)
                .join(".cadis")
                .join("worktrees")
                .display()
                .to_string()
        })
        .unwrap_or_else(|| ".cadis/worktrees".to_owned());
    let worktree_path = Path::new(&worktree_root)
        .join(worker_id)
        .display()
        .to_string();

    WorkerWorktreeIntent {
        workspace_id: None,
        project_root: workspace.map(ToOwned::to_owned),
        worktree_root,
        worktree_path,
        branch_name: format!("cadis/{worker_id}/{}", branch_slug(task)),
        base_ref: Some("HEAD".to_owned()),
        state: WorkerWorktreeState::Planned,
        cleanup_policy: WorkerWorktreeCleanupPolicy::Explicit,
    }
}

fn worker_artifact_locations(root: &Path, worker_id: &str) -> WorkerArtifactLocations {
    let root = root.join(worker_id);
    let root_display = root.display().to_string();

    WorkerArtifactLocations {
        root: root_display,
        patch: root.join("patch.diff").display().to_string(),
        test_report: root.join("test-report.json").display().to_string(),
        summary: root.join("summary.md").display().to_string(),
        changed_files: root.join("changed-files.json").display().to_string(),
        memory_candidates: root.join("memory-candidates.jsonl").display().to_string(),
    }
}

fn normalize_lookup(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('_');
            last_was_separator = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    if slug.is_empty() {
        "agent".to_owned()
    } else {
        slug
    }
}

fn branch_slug(value: &str) -> String {
    let slug = slugify(value).replace('_', "-");
    if slug.is_empty() {
        "task".to_owned()
    } else {
        slug.chars().take(32).collect()
    }
}

fn agent_session_is_terminal(status: AgentSessionStatus) -> bool {
    matches!(
        status,
        AgentSessionStatus::Completed
            | AgentSessionStatus::Failed
            | AgentSessionStatus::Cancelled
            | AgentSessionStatus::TimedOut
            | AgentSessionStatus::BudgetExceeded
    )
}

fn json_usize(value: &serde_json::Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn json_u32(value: &serde_json::Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn json_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(serde_json::Value::as_i64)
}

fn merge_json(mut base: serde_json::Value, patch: serde_json::Value) -> serde_json::Value {
    match (&mut base, patch) {
        (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                let current = base.remove(&key).unwrap_or(serde_json::Value::Null);
                base.insert(key, merge_json(current, value));
            }
            serde_json::Value::Object(base.clone())
        }
        (_, patch) => patch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadis_models::{provider_from_config, EchoProvider};
    use cadis_protocol::{
        AgentModelSetRequest, AgentRenameRequest, AgentSpawnRequest, ApprovalResponseRequest,
        ClientId, ContentKind, EmptyPayload, EventSubscriptionRequest, EventsSnapshotRequest,
        MessageSendRequest, RequestId, ServerFrame, SessionCreateRequest, SessionTargetRequest,
        ToolCallRequest, WorkerTailRequest, WorkspaceAccess, WorkspaceGrantRequest, WorkspaceId,
        WorkspaceKind, WorkspaceRegisterRequest,
    };

    fn runtime() -> Runtime {
        runtime_with_spawn_limits(AgentSpawnLimits::default())
    }

    fn runtime_with_spawn_limits(spawn_limits: AgentSpawnLimits) -> Runtime {
        runtime_with_options(spawn_limits, OrchestratorConfig::default())
    }

    fn runtime_with_options(
        spawn_limits: AgentSpawnLimits,
        orchestrator_config: OrchestratorConfig,
    ) -> Runtime {
        runtime_with_home_and_options(
            test_workspace("cadis-home"),
            spawn_limits,
            orchestrator_config,
        )
    }

    fn runtime_with_home(cadis_home: PathBuf) -> Runtime {
        runtime_with_home_and_options(
            cadis_home,
            AgentSpawnLimits::default(),
            OrchestratorConfig::default(),
        )
    }

    fn runtime_with_home_and_options(
        cadis_home: PathBuf,
        spawn_limits: AgentSpawnLimits,
        orchestrator_config: OrchestratorConfig,
    ) -> Runtime {
        Runtime::new(
            RuntimeOptions {
                cadis_home,
                profile_id: "default".to_owned(),
                socket_path: Some(PathBuf::from("/tmp/cadis-test.sock")),
                model_provider: "echo".to_owned(),
                ui_preferences: serde_json::json!({
                    "hud": {
                        "theme": "arc",
                        "avatar_style": "orb",
                        "background_opacity": 90
                    },
                    "voice": {
                        "enabled": false,
                        "voice_id": "id-ID-GadisNeural"
                    },
                    "agent_spawn": {
                        "max_depth": spawn_limits.max_depth,
                        "max_children_per_parent": spawn_limits.max_children_per_parent,
                        "max_total_agents": spawn_limits.max_total_agents
                    },
                    "orchestrator": {
                        "worker_delegation_enabled": orchestrator_config.worker_delegation_enabled,
                        "default_worker_role": orchestrator_config.default_worker_role
                    }
                }),
            },
            Box::<EchoProvider>::default(),
        )
    }

    fn runtime_with_provider(provider: Box<dyn ModelProvider>, model_provider: &str) -> Runtime {
        Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cadis-home"),
                profile_id: "default".to_owned(),
                socket_path: Some(PathBuf::from("/tmp/cadis-test.sock")),
                model_provider: model_provider.to_owned(),
                ui_preferences: serde_json::json!({
                    "agent_spawn": {
                        "max_depth": AgentSpawnLimits::default().max_depth,
                        "max_children_per_parent": AgentSpawnLimits::default().max_children_per_parent,
                        "max_total_agents": AgentSpawnLimits::default().max_total_agents
                    },
                    "orchestrator": {
                        "worker_delegation_enabled": true,
                        "default_worker_role": "Worker"
                    }
                }),
            },
            provider,
        )
    }

    fn runtime_with_agent_runtime_config(agent_runtime: AgentRuntimeConfig) -> Runtime {
        Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cadis-home"),
                profile_id: "default".to_owned(),
                socket_path: Some(PathBuf::from("/tmp/cadis-test.sock")),
                model_provider: "echo".to_owned(),
                ui_preferences: serde_json::json!({
                    "agent_spawn": {
                        "max_depth": AgentSpawnLimits::default().max_depth,
                        "max_children_per_parent": AgentSpawnLimits::default().max_children_per_parent,
                        "max_total_agents": AgentSpawnLimits::default().max_total_agents
                    },
                    "agent_runtime": {
                        "default_timeout_sec": agent_runtime.default_timeout_sec,
                        "max_steps_per_session": agent_runtime.max_steps_per_session
                    },
                    "orchestrator": {
                        "worker_delegation_enabled": true,
                        "default_worker_role": "Worker"
                    }
                }),
            },
            Box::<EchoProvider>::default(),
        )
    }

    fn test_workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cadis-core-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&path).expect("test workspace should be created");
        path
    }

    fn register_workspace(runtime: &mut Runtime, workspace_id: &str, root: &Path) {
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from(format!("req_register_{workspace_id}")),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceRegister(WorkspaceRegisterRequest {
                workspace_id: WorkspaceId::from(workspace_id),
                kind: WorkspaceKind::Project,
                root: root.display().to_string(),
                aliases: Vec::new(),
                vcs: Some("git".to_owned()),
                trusted: true,
                worktree_root: Some(".cadis/worktrees".to_owned()),
                artifact_root: Some(".cadis/artifacts".to_owned()),
            }),
        ));
        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
    }

    fn grant_workspace(runtime: &mut Runtime, workspace_id: &str, access: Vec<WorkspaceAccess>) {
        grant_workspace_for_agent(runtime, workspace_id, access, None)
    }

    fn grant_workspace_for_agent(
        runtime: &mut Runtime,
        workspace_id: &str,
        access: Vec<WorkspaceAccess>,
        agent_id: Option<AgentId>,
    ) {
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from(format!("req_grant_{workspace_id}")),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceGrant(WorkspaceGrantRequest {
                agent_id,
                workspace_id: WorkspaceId::from(workspace_id),
                access,
                expires_at: None,
                source: Some("test".to_owned()),
            }),
        ));
        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
    }

    #[test]
    fn status_returns_typed_response() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::DaemonStatus(EmptyPayload::default()),
        ));

        match outcome.response.response {
            DaemonResponse::DaemonStatus(status) => {
                assert_eq!(status.status, "ok");
                assert_eq!(status.model_provider, "echo");
            }
            other => panic!("unexpected response: {other:?}"),
        }
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn builtin_tool_registry_contains_safe_and_gated_tools() {
        let registry = ToolRegistry::builtin().expect("registry should build");

        assert!(registry.is_auto_executable_safe_read("file.read"));
        assert!(registry.is_auto_executable_safe_read("file.search"));
        assert!(registry.is_auto_executable_safe_read("git.status"));
        assert!(!registry.is_auto_executable_safe_read("shell.run"));

        let shell = registry.get("shell.run").expect("shell tool exists");
        assert_eq!(shell.risk_class, cadis_protocol::RiskClass::SystemChange);
        assert_eq!(shell.execution, ToolExecutionMode::ApprovalPlaceholder);
    }

    #[test]
    fn tool_registry_rejects_duplicate_names() {
        let result = ToolRegistry::new(vec![
            ToolDefinition::safe_read("file.read", ToolInputSchema::FileRead),
            ToolDefinition::safe_read("file.read", ToolInputSchema::FileSearch),
        ]);

        let error = result.expect_err("duplicate tool names should be rejected");
        assert_eq!(error.code, "duplicate_tool_name");
        assert!(error.message.contains("file.read"));
    }

    #[test]
    fn safe_file_read_tool_runs_without_approval() {
        let workspace = test_workspace("file-read");
        fs::write(workspace.join("README.md"), "hello from tool\n")
            .expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-read", &workspace);
        grant_workspace(&mut runtime, "file-read", vec![WorkspaceAccess::Read]);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-read",
                    "path": "README.md"
                }),
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolCompleted(payload)
                    if payload.tool_name == "file.read"
                        && payload.summary.as_deref() == Some("hello from tool\n")
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
    }

    #[test]
    fn file_read_fails_closed_outside_workspace() {
        let workspace = test_workspace("outside-workspace");
        let outside = test_workspace("outside-file");
        fs::write(outside.join("secret.txt"), "secret").expect("outside file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "outside-workspace", &workspace);
        grant_workspace(
            &mut runtime,
            "outside-workspace",
            vec![WorkspaceAccess::Read],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "outside-workspace",
                    "path": outside.join("secret.txt")
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "outside_workspace"
            )
        }));
    }

    #[test]
    fn file_read_requires_registered_workspace_grant() {
        let workspace = test_workspace("grant-required");
        fs::write(workspace.join("README.md"), "hello").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "grant-required", &workspace);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "grant-required",
                    "path": "README.md"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "workspace_grant_required"
            )
        }));
    }

    #[test]
    fn workspace_register_rejects_broad_roots() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_broad_workspace"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceRegister(WorkspaceRegisterRequest {
                workspace_id: WorkspaceId::from("root"),
                kind: WorkspaceKind::Project,
                root: "/".to_owned(),
                aliases: Vec::new(),
                vcs: None,
                trusted: false,
                worktree_root: None,
                artifact_root: None,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestRejected(error)
                if error.code == "workspace_root_too_broad"
        ));
    }

    #[test]
    fn workspace_registry_and_grants_survive_runtime_restart() {
        let cadis_home = test_workspace("persistent-cadis-home");
        let workspace = test_workspace("persistent-workspace");
        fs::write(workspace.join("README.md"), "persisted").expect("test file should write");

        {
            let mut runtime = runtime_with_home(cadis_home.clone());
            register_workspace(&mut runtime, "persistent-workspace", &workspace);
            grant_workspace(
                &mut runtime,
                "persistent-workspace",
                vec![WorkspaceAccess::Read],
            );
        }

        let mut runtime = runtime_with_home(cadis_home);
        assert!(runtime
            .workspaces
            .contains_key(&WorkspaceId::from("persistent-workspace")));
        assert_eq!(runtime.workspace_grants.len(), 1);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_persisted_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "persistent-workspace",
                    "path": "README.md"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolCompleted(payload)
                    if payload.summary.as_deref() == Some("persisted")
            )
        }));
    }

    #[test]
    fn session_metadata_survives_runtime_restart_and_cancel_removes_it() {
        let cadis_home = test_workspace("persistent-session-home");
        let cwd = test_workspace("persistent-session-cwd");
        let session_id = {
            let mut runtime = runtime_with_home(cadis_home.clone());
            let outcome = runtime.handle_request(RequestEnvelope::new(
                RequestId::from("req_create_session"),
                ClientId::from("cli_1"),
                ClientRequest::SessionCreate(SessionCreateRequest {
                    title: Some("Durable session".to_owned()),
                    cwd: Some(cwd.display().to_string()),
                }),
            ));

            outcome
                .events
                .into_iter()
                .find_map(|event| match event.event {
                    CadisEvent::SessionStarted(payload) => Some(payload.session_id),
                    _ => None,
                })
                .expect("session.started should be emitted")
        };

        let mut runtime = runtime_with_home(cadis_home.clone());
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_subscribe_session"),
            ClientId::from("cli_1"),
            ClientRequest::SessionSubscribe(SessionTargetRequest {
                session_id: session_id.clone(),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::SessionUpdated(payload)
                    if payload.session_id == session_id
                        && payload.title.as_deref() == Some("Durable session")
            )
        }));

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancel_session"),
            ClientId::from("cli_1"),
            ClientRequest::SessionCancel(SessionTargetRequest {
                session_id: session_id.clone(),
            }),
        ));
        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));

        let mut runtime = runtime_with_home(cadis_home);
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_subscribe_removed_session"),
            ClientId::from("cli_1"),
            ClientRequest::SessionSubscribe(SessionTargetRequest { session_id }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestRejected(error) if error.code == "session_not_found"
        ));
    }

    #[test]
    fn spawned_agent_metadata_survives_runtime_restart() {
        let cadis_home = test_workspace("persistent-agent-home");
        let agent_id = {
            let mut runtime = runtime_with_home(cadis_home.clone());
            let outcome = runtime.handle_request(RequestEnvelope::new(
                RequestId::from("req_spawn_agent"),
                ClientId::from("cli_1"),
                ClientRequest::AgentSpawn(AgentSpawnRequest {
                    role: "Research".to_owned(),
                    parent_agent_id: Some(AgentId::from("main")),
                    display_name: Some("Research Scout".to_owned()),
                    model: Some("echo/cadis-local-fallback".to_owned()),
                }),
            ));

            outcome
                .events
                .iter()
                .find_map(|event| match &event.event {
                    CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                    _ => None,
                })
                .expect("agent.spawned should be emitted")
        };

        let mut runtime = runtime_with_home(cadis_home);
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_agent_list"),
            ClientId::from("cli_1"),
            ClientRequest::AgentList(EmptyPayload::default()),
        ));

        assert!(outcome.events.iter().any(|event| {
            let CadisEvent::AgentListResponse(payload) = &event.event else {
                return false;
            };
            payload.agents.iter().any(|agent| {
                agent.agent_id == agent_id
                    && agent.display_name.as_deref() == Some("Research Scout")
                    && agent.model.as_deref() == Some("echo/cadis-local-fallback")
                    && agent.parent_agent_id.as_ref() == Some(&AgentId::from("main"))
            })
        }));
    }

    #[test]
    fn agent_scoped_workspace_grant_requires_matching_tool_agent() {
        let workspace = test_workspace("agent-grant");
        fs::write(workspace.join("README.md"), "agent scoped").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "agent-grant", &workspace);
        grant_workspace_for_agent(
            &mut runtime,
            "agent-grant",
            vec![WorkspaceAccess::Read],
            Some(AgentId::from("codex")),
        );

        let without_agent = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_without_agent"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "agent-grant",
                    "path": "README.md"
                }),
            }),
        ));
        assert!(without_agent.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "workspace_grant_required"
            )
        }));

        let matching_agent = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_matching_agent"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: Some(AgentId::from("codex")),
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "agent-grant",
                    "path": "README.md"
                }),
            }),
        ));
        assert!(matching_agent.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolCompleted(payload)
                    if payload.summary.as_deref() == Some("agent scoped")
            )
        }));
    }

    #[test]
    fn tool_workspace_id_persists_on_session() {
        let workspace = test_workspace("session-workspace");
        fs::write(workspace.join("README.md"), "session workspace")
            .expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "session-workspace", &workspace);
        grant_workspace(
            &mut runtime,
            "session-workspace",
            vec![WorkspaceAccess::Read],
        );
        let session_id = SessionId::from("ses_tool");

        let first = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_first_session_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: Some(session_id.clone()),
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "session-workspace",
                    "path": "README.md"
                }),
            }),
        ));
        assert!(first
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolCompleted(_))));

        let second = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_second_session_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: Some(session_id),
                agent_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "path": "README.md"
                }),
            }),
        ));

        assert!(second.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolCompleted(payload)
                    if payload.summary.as_deref() == Some("session workspace")
            )
        }));
    }

    #[test]
    fn risky_shell_tool_requests_approval_and_denial_fails_tool() {
        let workspace = test_workspace("shell-approval");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "shell-approval", &workspace);
        grant_workspace(&mut runtime, "shell-approval", vec![WorkspaceAccess::Exec]);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "shell-approval",
                    "command": "rm -rf target"
                }),
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let approval_id = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::ApprovalRequested(payload) => Some(payload.approval_id.clone()),
                _ => None,
            })
            .expect("approval.requested should be emitted");
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_deny"),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id,
                decision: ApprovalDecision::Denied,
                reason: Some("not needed".to_owned()),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Denied
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "approval_denied"
            )
        }));
    }

    #[test]
    fn message_send_creates_session_and_message_events() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "hello".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::SessionStarted(_))));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::MessageDelta(_))));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::MessageCompleted(_))));
    }

    #[test]
    fn message_send_emits_agent_session_lifecycle_events() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_agent_session"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let started = outcome.events.iter().find_map(|event| match &event.event {
            CadisEvent::AgentSessionStarted(payload) => Some(payload),
            _ => None,
        });
        let started = started.expect("agent.session.started should be emitted");
        assert_eq!(started.agent_id.as_str(), "codex");
        assert_eq!(started.route_id, "route_000001");
        assert_eq!(started.status, AgentSessionStatus::Running);
        assert_eq!(started.task, "run tests");
        assert_eq!(started.budget_steps, 1);
        assert_eq!(started.steps_used, 0);

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionUpdated(payload)
                    if payload.agent_session_id == started.agent_session_id
                        && payload.steps_used == 1
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionCompleted(payload)
                    if payload.agent_session_id == started.agent_session_id
                        && payload.result.as_deref().is_some_and(|result| result.contains("run tests"))
            )
        }));
    }

    #[test]
    fn agent_session_metadata_survives_runtime_restart_and_snapshot() {
        let cadis_home = test_workspace("persistent-agent-session-home");
        let completed = {
            let mut runtime = runtime_with_home(cadis_home.clone());
            let outcome = runtime.handle_request(RequestEnvelope::new(
                RequestId::from("req_agent_session_persist"),
                ClientId::from("cli_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: Some(AgentId::from("codex")),
                    content: "run recovery tests".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ));

            outcome
                .events
                .iter()
                .find_map(|event| match &event.event {
                    CadisEvent::AgentSessionCompleted(payload) => Some(payload.clone()),
                    _ => None,
                })
                .expect("agent.session.completed should be emitted")
        };

        let store = StateStore::new(&CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        });
        assert!(store
            .agent_session_path(&completed.agent_session_id)
            .is_file());

        let mut runtime = runtime_with_home(cadis_home);
        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_agent_session_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionCompleted(payload)
                    if payload.agent_session_id == completed.agent_session_id
                        && payload.route_id == completed.route_id
                        && payload.status == AgentSessionStatus::Completed
                        && payload.steps_used == 1
                        && payload.result.as_deref().is_some_and(|result| result.contains("run recovery tests"))
            )
        }));

        let next = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_agent_session_next"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "next route".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(next.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionStarted(payload)
                    if payload.agent_session_id.as_str() == "ags_000002"
                        && payload.route_id == "route_000002"
            )
        }));
    }

    #[test]
    fn corrupt_agent_session_metadata_reports_diagnostic_and_ignores_partial_temp_file() {
        let cadis_home = test_workspace("corrupt-agent-session-home");
        let store = StateStore::new(&CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        });
        store.ensure_layout().expect("state layout should exist");
        let valid = AgentSessionMetadata {
            agent_session_id: AgentSessionId::from("ags_000041"),
            session_id: SessionId::from("ses_recovered"),
            route_id: "route_000041".to_owned(),
            agent_id: AgentId::from("codex"),
            parent_agent_id: Some(AgentId::from("main")),
            task: "recover me".to_owned(),
            status: AgentSessionStatus::Running,
            timeout_at: Timestamp::new_utc("2026-04-26T00:15:00Z")
                .expect("test timestamp should be valid"),
            budget_steps: 2,
            steps_used: 1,
            result: None,
            error_code: None,
            error: None,
            cancellation_requested_at: None,
        };

        store
            .write_agent_session_metadata(&valid.agent_session_id, &valid)
            .expect("valid AgentSession metadata should write");
        let agent_sessions_dir = cadis_home.join("state/agent-sessions");
        fs::write(agent_sessions_dir.join("corrupt.json"), "{")
            .expect("corrupt AgentSession metadata should write");
        fs::write(agent_sessions_dir.join(".ags_partial.json.tmp.1"), "{")
            .expect("partial AgentSession temp metadata should write");

        let mut runtime = runtime_with_home(cadis_home);
        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_corrupt_agent_session_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionUpdated(payload)
                    if payload.agent_session_id.as_str() == "ags_000041"
                        && payload.task == "recover me"
            )
        }));
        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::DaemonError(error)
                    if error.code == "agent_session_recovery_skipped"
                        && error.message.contains("corrupt.json")
                        && !error.message.contains(".ags_partial")
            )
        }));
    }

    #[test]
    fn agent_session_budget_limit_fails_before_provider_execution() {
        let mut runtime = runtime_with_agent_runtime_config(AgentRuntimeConfig {
            default_timeout_sec: 900,
            max_steps_per_session: 0,
        });
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_budget"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionFailed(payload)
                    if payload.status == AgentSessionStatus::BudgetExceeded
                        && payload.error_code.as_deref() == Some("agent_budget_exceeded")
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::MessageCompleted(_))));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::SessionFailed(payload)
                    if payload.code == "agent_budget_exceeded"
            )
        }));
    }

    #[test]
    fn session_cancel_marks_running_agent_sessions_cancelled() {
        let mut runtime = runtime();
        let session_id = runtime.create_session(Some("Cancelable".to_owned()), None);
        let (agent_session_id, _event) = runtime.start_agent_session(
            session_id.clone(),
            "route_000001".to_owned(),
            AgentId::from("codex"),
            "wait for user".to_owned(),
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancel"),
            ClientId::from("cli_1"),
            ClientRequest::SessionCancel(SessionTargetRequest {
                session_id: session_id.clone(),
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionCancelled(payload)
                    if payload.agent_session_id == agent_session_id
                        && payload.status == AgentSessionStatus::Cancelled
                        && payload.cancellation_requested_at.is_some()
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentStatusChanged(payload)
                    if payload.agent_id.as_str() == "codex"
                        && payload.status == AgentStatus::Cancelled
            )
        }));
    }

    #[test]
    fn response_and_events_can_be_framed() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::SessionCreate(SessionCreateRequest {
                title: Some("Test".to_owned()),
                cwd: None,
            }),
        ));

        let response_frame = ServerFrame::Response(outcome.response);
        let event_frame = ServerFrame::Event(outcome.events[0].clone());

        serde_json::to_string(&response_frame).expect("response frame should serialize");
        serde_json::to_string(&event_frame).expect("event frame should serialize");
    }

    #[test]
    fn events_snapshot_emits_current_runtime_state() {
        let mut runtime = runtime();
        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_create"),
            ClientId::from("cli_1"),
            ClientRequest::SessionCreate(SessionCreateRequest {
                title: Some("Snapshot session".to_owned()),
                cwd: None,
            }),
        ));

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::AgentListResponse(_))));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::UiPreferencesUpdated(_))));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::SessionUpdated(payload)
                    if payload.title.as_deref() == Some("Snapshot session")
            )
        }));
    }

    #[test]
    fn events_subscribe_can_skip_initial_snapshot() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_subscribe"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSubscribe(EventSubscriptionRequest {
                include_snapshot: false,
                ..EventSubscriptionRequest::default()
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn agent_rename_is_confirmed_by_event() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::AgentRename(AgentRenameRequest {
                agent_id: AgentId::from("main"),
                display_name: "  Local   CADIS  ".to_owned(),
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentRenamed(payload)
                    if payload.agent_id.as_str() == "main"
                        && payload.display_name == "Local CADIS"
            )
        }));
    }

    #[test]
    fn agent_model_set_is_confirmed_by_event() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::AgentModelSet(AgentModelSetRequest {
                agent_id: AgentId::from("main"),
                model: "ollama/llama3.2".to_owned(),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentModelChanged(payload)
                    if payload.agent_id.as_str() == "main"
                        && payload.model == "ollama/llama3.2"
            )
        }));
    }

    #[test]
    fn message_send_passes_selected_agent_model_to_provider_router() {
        let provider = provider_from_config(
            "openai",
            "http://127.0.0.1:11434",
            "llama3.2",
            "https://api.openai.com/v1",
            "gpt-5.2",
            None,
        );
        let mut runtime = runtime_with_provider(provider, "openai");

        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_model"),
            ClientId::from("hud_1"),
            ClientRequest::AgentModelSet(AgentModelSetRequest {
                agent_id: AgentId::from("codex"),
                model: "echo/cadis-local-fallback".to_owned(),
            }),
        ));
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_chat"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "hello".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::MessageCompleted(payload)
                    if payload.model.as_ref().is_some_and(|model|
                        model.requested_model.as_deref() == Some("echo/cadis-local-fallback")
                            && model.effective_provider == "echo"
                            && model.effective_model == "cadis-local-fallback"
                            && !model.fallback
                    )
            )
        }));
    }

    #[test]
    fn agent_list_returns_roster_snapshot() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::AgentList(EmptyPayload::default()),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentListResponse(payload)
                    if payload.agents.iter().any(|agent| agent.agent_id.as_str() == "main")
                        && payload.agents.iter().any(|agent| agent.agent_id.as_str() == "codex")
            )
        }));
    }

    #[test]
    fn models_list_includes_readiness_metadata() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::ModelsList(EmptyPayload::default()),
        ));

        let models = outcome.events.iter().find_map(|event| match &event.event {
            CadisEvent::ModelsListResponse(payload) => Some(&payload.models),
            _ => None,
        });
        let models = models.expect("models.list.response should be emitted");

        let echo = models
            .iter()
            .find(|model| model.provider == "echo")
            .expect("echo fallback should be listed");
        assert_eq!(echo.readiness, Some(ModelReadiness::Fallback));
        assert_eq!(echo.effective_provider.as_deref(), Some("echo"));
        assert_eq!(
            echo.effective_model.as_deref(),
            Some("cadis-local-fallback")
        );
        assert!(echo.fallback);

        let ollama = models
            .iter()
            .find(|model| model.provider == "ollama")
            .expect("ollama should be listed");
        assert_eq!(
            ollama.readiness,
            Some(ModelReadiness::RequiresConfiguration)
        );
        assert_eq!(ollama.effective_provider.as_deref(), Some("ollama"));
        assert!(!ollama.fallback);
    }

    #[test]
    fn agent_spawn_registers_child_agent() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Coding".to_owned(),
                parent_agent_id: Some(AgentId::from("main")),
                display_name: Some("Builder".to_owned()),
                model: Some("openai/gpt-5.5".to_owned()),
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSpawned(payload)
                    if payload.display_name.as_deref() == Some("Builder")
                        && payload.role.as_deref() == Some("Coding")
                        && payload.parent_agent_id.as_ref().map(AgentId::as_str) == Some("main")
                        && payload.model.as_deref() == Some("openai/gpt-5.5")
            )
        }));
    }

    #[test]
    fn agent_spawn_rejects_depth_limit() {
        let mut runtime = runtime_with_spawn_limits(AgentSpawnLimits {
            max_depth: 1,
            max_children_per_parent: 4,
            max_total_agents: 32,
        });
        let first = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Planner".to_owned(),
                parent_agent_id: Some(AgentId::from("main")),
                display_name: None,
                model: None,
            }),
        ));
        assert!(matches!(
            first.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let parent = first
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                _ => None,
            })
            .expect("first spawn should emit agent.spawned");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_2"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Worker".to_owned(),
                parent_agent_id: Some(parent),
                display_name: None,
                model: None,
            }),
        ));

        assert_rejected(outcome, "agent_spawn_depth_limit_exceeded");
    }

    #[test]
    fn agent_spawn_rejects_children_limit() {
        let mut runtime = runtime_with_spawn_limits(AgentSpawnLimits {
            max_depth: 2,
            max_children_per_parent: 1,
            max_total_agents: 32,
        });
        let first = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Builder".to_owned(),
                parent_agent_id: Some(AgentId::from("main")),
                display_name: None,
                model: None,
            }),
        ));
        assert!(matches!(
            first.response.response,
            DaemonResponse::RequestAccepted(_)
        ));

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_2"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Reviewer".to_owned(),
                parent_agent_id: Some(AgentId::from("main")),
                display_name: None,
                model: None,
            }),
        ));

        assert_rejected(outcome, "agent_spawn_children_limit_exceeded");
    }

    #[test]
    fn agent_spawn_rejects_total_limit() {
        let mut runtime = runtime_with_spawn_limits(AgentSpawnLimits {
            max_depth: 2,
            max_children_per_parent: 4,
            max_total_agents: 13,
        });
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Builder".to_owned(),
                parent_agent_id: Some(AgentId::from("main")),
                display_name: None,
                model: None,
            }),
        ));

        assert_rejected(outcome, "agent_spawn_total_limit_exceeded");
    }

    #[test]
    fn message_send_routes_leading_mention() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "@codex run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::OrchestratorRoute(payload)
                    if payload.target_agent_id.as_str() == "codex"
                        && payload.reason == "@codex mention"
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::MessageDelta(payload)
                    if payload.agent_id.as_ref().map(AgentId::as_str) == Some("codex")
                        && payload.agent_name.as_deref() == Some("Codex")
            )
        }));
    }

    #[test]
    fn explicit_target_strips_matching_leading_mention_from_provider_prompt() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "@codex run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let completed = outcome.events.iter().find_map(|event| match &event.event {
            CadisEvent::MessageCompleted(payload) => payload.content.as_deref(),
            _ => None,
        });

        let completed = completed.expect("message.completed should include provider output");
        assert!(completed.contains("User request:\nrun tests"));
        assert!(!completed.contains("User request:\n@codex run tests"));
    }

    #[test]
    fn orchestrator_route_action_delegates_to_existing_worker() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "/route @codex run focused tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::OrchestratorRoute(payload)
                    if payload.target_agent_id.as_str() == "codex"
                        && payload.reason == "orchestrator action: route @codex"
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerStarted(payload)
                    if payload.agent_id.as_ref().map(AgentId::as_str) == Some("codex")
                        && payload.status.as_deref() == Some("running")
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCompleted(payload)
                    if payload.agent_id.as_ref().map(AgentId::as_str) == Some("codex")
                        && payload.status.as_deref() == Some("completed")
            )
        }));

        let started = outcome.events.iter().find_map(|event| match &event.event {
            CadisEvent::WorkerStarted(payload) => Some(payload),
            _ => None,
        });
        let started = started.expect("worker.started should be emitted");
        let worktree = started
            .worktree
            .as_ref()
            .expect("worker event should include worktree intent");
        assert_eq!(worktree.state, WorkerWorktreeState::Planned);
        assert_eq!(worktree.worktree_root, ".cadis/worktrees");
        assert_eq!(worktree.worktree_path, ".cadis/worktrees/worker_000001");
        assert_eq!(
            worktree.branch_name,
            "cadis/worker_000001/run-focused-tests"
        );
        let expected_patch = runtime
            .options
            .cadis_home
            .join("artifacts/workers/worker_000001/patch.diff")
            .display()
            .to_string();
        assert_eq!(
            started
                .artifacts
                .as_ref()
                .map(|artifacts| artifacts.patch.as_str()),
            Some(expected_patch.as_str())
        );
    }

    #[test]
    fn worker_tail_replays_registered_worker_log_lines() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_worker_route"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "/route @codex run focused tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let worker_id = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerStarted(payload) => Some(payload.worker_id.clone()),
                _ => None,
            })
            .expect("worker.started should be emitted");
        let live_log_count = outcome
            .events
            .iter()
            .filter(|event| matches!(event.event, CadisEvent::WorkerLogDelta(_)))
            .count();
        assert_eq!(live_log_count, 2);

        let tail = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_worker_tail"),
            ClientId::from("cli_1"),
            ClientRequest::WorkerTail(WorkerTailRequest {
                worker_id: worker_id.clone(),
                lines: Some(1),
            }),
        ));
        assert!(matches!(
            tail.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let deltas = tail
            .events
            .iter()
            .filter_map(|event| match &event.event {
                CadisEvent::WorkerLogDelta(payload) => Some(payload),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].worker_id, worker_id);
        assert_eq!(
            deltas[0].agent_id.as_ref().map(AgentId::as_str),
            Some("codex")
        );
        assert_eq!(
            deltas[0].delta,
            "completed: Route @codex: run focused tests\n"
        );

        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_snapshot_workers"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));
        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCompleted(payload)
                    if payload.worker_id == worker_id
                        && payload.status.as_deref() == Some("completed")
            )
        }));
    }

    #[test]
    fn worker_tail_rejects_unknown_worker() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_missing_worker_tail"),
            ClientId::from("cli_1"),
            ClientRequest::WorkerTail(WorkerTailRequest {
                worker_id: "worker_missing".to_owned(),
                lines: Some(10),
            }),
        ));

        assert_rejected(outcome, "worker_not_found");
    }

    #[test]
    fn orchestrator_worker_action_spawns_and_routes_child_agent() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "/worker Reviewer: inspect patch".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let spawned_agent = outcome.events.iter().find_map(|event| match &event.event {
            CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
            _ => None,
        });
        let spawned_agent = spawned_agent.expect("worker action should spawn an agent");
        assert!(spawned_agent.as_str().starts_with("reviewer_"));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSpawned(payload)
                    if payload.role.as_deref() == Some("Reviewer")
                        && payload.parent_agent_id.as_ref().map(AgentId::as_str) == Some("main")
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::OrchestratorRoute(payload)
                    if payload.target_agent_id == spawned_agent
                        && payload.reason == "orchestrator action: spawn worker"
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::MessageCompleted(payload)
                    if payload.agent_id.as_ref() == Some(&spawned_agent)
                        && payload
                            .content
                            .as_deref()
                            .is_some_and(|content| content.contains("User request:\ninspect patch"))
            )
        }));
    }

    #[test]
    fn orchestrator_worker_action_respects_spawn_limits() {
        let mut runtime = runtime_with_spawn_limits(AgentSpawnLimits {
            max_depth: 2,
            max_children_per_parent: 4,
            max_total_agents: 13,
        });
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "/worker inspect patch".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert_rejected(outcome, "agent_spawn_total_limit_exceeded");
    }

    #[test]
    fn orchestrator_worker_actions_can_be_disabled_without_blocking_mentions() {
        let mut runtime = runtime_with_options(
            AgentSpawnLimits::default(),
            OrchestratorConfig {
                worker_delegation_enabled: false,
                default_worker_role: "Worker".to_owned(),
            },
        );
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "/route @codex run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        assert_rejected(outcome, "orchestrator_worker_delegation_disabled");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_2"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "@codex run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
    }

    #[test]
    fn unknown_mention_is_rejected() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "@missing hello".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestRejected(ErrorPayload { code, .. }) if code == "agent_not_found"
        ));
        assert!(outcome.events.is_empty());
    }

    fn assert_rejected(outcome: RequestOutcome, code: &str) {
        match outcome.response.response {
            DaemonResponse::RequestRejected(error) => assert_eq!(error.code, code),
            other => panic!("unexpected response: {other:?}"),
        }
        assert!(outcome.events.is_empty());
    }
}
