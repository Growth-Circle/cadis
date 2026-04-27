//! Core CADIS request handling and event production.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration as StdDuration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use cadis_models::{
    provider_catalog_for_config, ModelCatalogConfig, ModelInvocation, ModelProvider, ModelRequest,
    ModelResponse, ModelStreamControl, ModelStreamEvent, ProviderCatalogEntry, ProviderReadiness,
};
use cadis_policy::{PolicyDecision, PolicyEngine};
use cadis_protocol::{
    AgentEventPayload, AgentId, AgentListPayload, AgentModelChangedPayload, AgentRenamedPayload,
    AgentSessionEventPayload, AgentSessionId, AgentSessionStatus, AgentSpawnRequest, AgentStatus,
    AgentStatusChangedPayload, ApprovalDecision, ApprovalId, ApprovalRequestPayload,
    ApprovalResolvedPayload, ApprovalResponseRequest, CadisEvent, ClientRequest, ContentKind,
    DaemonResponse, DaemonStatusPayload, ErrorPayload, EventEnvelope, EventId,
    MessageCompletedPayload, MessageDeltaPayload, MessageSendRequest, ModelDescriptor,
    ModelInvocationPayload, ModelReadiness, ModelsListPayload, OrchestratorRoutePayload,
    ProtocolVersion, RequestAcceptedPayload, RequestEnvelope, RequestId, ResponseEnvelope,
    SessionEventPayload, SessionId, Timestamp, ToolCallId, ToolCallRequest, ToolEventPayload,
    ToolFailedPayload, UiPreferencesPayload, VoiceDoctorCheck, VoiceDoctorPayload,
    VoicePreferences, VoicePreflightRequest, VoicePreflightSummary, VoicePreviewRequest,
    VoiceRuntimeState, VoiceStatusPayload, WorkerArtifactLocations, WorkerCleanupRequest,
    WorkerEventPayload, WorkerLogDeltaPayload, WorkerTailRequest, WorkerWorktreeCleanupPolicy,
    WorkerWorktreeIntent, WorkerWorktreeState, WorkspaceAccess, WorkspaceDoctorCheck,
    WorkspaceDoctorPayload, WorkspaceDoctorRequest, WorkspaceGrantId, WorkspaceGrantPayload,
    WorkspaceGrantRequest, WorkspaceId, WorkspaceKind, WorkspaceListPayload, WorkspaceListRequest,
    WorkspaceRecordPayload, WorkspaceRegisterRequest, WorkspaceRevokeRequest,
};
use cadis_store::{
    redact, AgentHomeDiagnostic, AgentHomeDoctorOptions, AgentHomeTemplate, ApprovalRecord,
    ApprovalState, ApprovalStore, CadisConfig, CadisHome, CheckpointPolicy,
    GrantSource as StoreGrantSource, ProfileHome, ProjectWorkerWorktreeMetadata,
    ProjectWorkerWorktreeState, ProjectWorkspaceStore, ProjectWorktreeDiagnostic,
    StateRecoveryDiagnostic, StateStore, WorkerArtifactPathSet,
    WorkspaceAccess as StoreWorkspaceAccess, WorkspaceAlias,
    WorkspaceGrantRecord as StoreWorkspaceGrantRecord, WorkspaceKind as StoreWorkspaceKind,
    WorkspaceMetadata, WorkspaceRegistry, WorkspaceVcs,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

const FILE_READ_LIMIT_BYTES: usize = 64 * 1024;
const FILE_SEARCH_LIMIT_BYTES: u64 = 1024 * 1024;
const FILE_SEARCH_DEFAULT_LIMIT: usize = 50;
const FILE_PATCH_MAX_OPERATIONS: usize = 64;
const FILE_PATCH_MAX_FILE_BYTES: usize = 1024 * 1024;
const FILE_PATCH_OUTPUT_MAX_FILES: usize = 64;
const GIT_DIFF_LIMIT_BYTES: usize = 128 * 1024;
const SHELL_OUTPUT_LIMIT_BYTES: usize = 16 * 1024;
const SHELL_POLL_INTERVAL_MS: u64 = 10;
const APPROVAL_TIMEOUT_MINUTES: i64 = 5;
const WORKER_TAIL_DEFAULT_LINES: usize = 64;
const WORKER_TAIL_MAX_LINES: usize = 1_000;
const WORKER_DEFAULT_COMMAND: &str = "git status --short";
const WORKER_COMMAND_TIMEOUT_MS: u64 = 5_000;
const WORKER_COMMAND_LOG_LIMIT_BYTES: usize = 4 * 1024;
const WORKER_COMMAND_SUMMARY_LIMIT_BYTES: usize = 512;

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
    /// Configured Ollama model used for catalog visibility.
    pub ollama_model: String,
    /// Configured OpenAI model used for catalog visibility.
    pub openai_model: String,
    /// Whether an OpenAI API key is present in the daemon environment.
    pub openai_api_key_configured: bool,
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

/// Text-to-speech provider contract owned by the daemon runtime.
pub trait TtsProvider: Send {
    /// Stable provider identifier.
    fn id(&self) -> &'static str;

    /// Human-readable provider label.
    fn label(&self) -> &'static str;

    /// Curated voice IDs this provider can report without external calls.
    fn supported_voices(&self) -> Vec<TtsVoice>;

    /// Speaks or queues short speakable text.
    fn speak(&mut self, request: TtsRequest<'_>) -> Result<TtsOutput, TtsError>;

    /// Stops current speech where the provider supports cancellation.
    fn stop(&mut self) -> Result<(), TtsError>;
}

/// Curated voice metadata exposed by TTS providers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TtsVoice {
    /// Provider voice identifier.
    pub id: &'static str,
    /// Display label.
    pub label: &'static str,
    /// BCP-47 locale.
    pub locale: &'static str,
    /// Display gender label from the provider catalog.
    pub gender: &'static str,
}

/// TTS request after daemon speech policy has allowed the text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TtsRequest<'a> {
    /// Redaction-checked speakable text.
    pub text: &'a str,
    /// Requested voice ID.
    pub voice_id: &'a str,
    /// Speaking rate adjustment.
    pub rate: i16,
    /// Pitch adjustment.
    pub pitch: i16,
    /// Volume adjustment.
    pub volume: i16,
}

/// Successful TTS provider outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TtsOutput {
    /// Provider that handled the request.
    pub provider: String,
    /// Voice selected for the request.
    pub voice_id: String,
    /// Character count accepted by the provider.
    pub spoken_chars: usize,
}

/// Structured TTS provider error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TtsError {
    /// Stable provider error code.
    pub code: String,
    /// Redacted human-readable message.
    pub message: String,
    /// Whether retrying may help.
    pub retryable: bool,
}

impl TtsError {
    fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
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

/// Daemon-owned model generation work prepared by the runtime.
pub struct PendingMessageGeneration {
    /// Immediate response for the client that sent `message.send`.
    pub response: ResponseEnvelope,
    /// Events that are ready before provider generation starts.
    pub initial_events: Vec<EventEnvelope>,
    /// Provider selected by the daemon runtime.
    pub provider: Arc<dyn ModelProvider>,
    /// Prompt text prepared by daemon-owned routing/orchestration.
    pub prompt: String,
    /// Optional provider/model selected on the routed agent.
    pub selected_model: Option<String>,
    context: MessageGenerationContext,
}

#[derive(Clone, Debug, PartialEq)]
struct MessageGenerationContext {
    session_id: SessionId,
    agent_session_id: AgentSessionId,
    content_kind: ContentKind,
    agent_id: AgentId,
    agent_name: String,
    worker: Option<WorkerDelegation>,
}

/// CADIS core runtime.
pub struct Runtime {
    options: RuntimeOptions,
    provider: Arc<dyn ModelProvider>,
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
    profile_home: ProfileHome,
    pending_approvals: HashMap<ApprovalId, PendingApproval>,
    workspaces: HashMap<WorkspaceId, WorkspaceRecord>,
    workspace_grants: HashMap<WorkspaceGrantId, WorkspaceGrantRecord>,
    last_voice_preflight: Option<VoicePreflightRecord>,
    recovery_diagnostics: Vec<ErrorPayload>,
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
        let RuntimeRecovery {
            records: sessions,
            mut diagnostics,
        } = recover_session_records(&state_store);
        let agent_session_recovery = recover_agent_session_records(&state_store);
        let agent_sessions = agent_session_recovery.records;
        diagnostics.extend(agent_session_recovery.diagnostics);
        let mut agents = default_agents(&options.model_provider);
        let agent_recovery = recover_agent_records(&state_store);
        diagnostics.extend(agent_recovery.diagnostics);
        for (agent_id, record) in agent_recovery.records {
            agents.insert(agent_id, record);
        }
        init_agent_homes(&profile_home, &agents);
        let worker_recovery = recover_worker_records(&state_store);
        diagnostics.extend(worker_recovery.diagnostics);
        let mut workers = worker_recovery.records;
        diagnostics.extend(reconcile_recovered_workers(&state_store, &mut workers));
        let approval_recovery = recover_approval_records(&state_store);
        diagnostics.extend(approval_recovery.diagnostics);
        let next_approval = next_approval_counter(&approval_recovery.records);
        let pending_approvals = pending_approval_records(approval_recovery.records);
        let next_session = next_session_counter(&sessions);
        let next_agent = next_agent_counter(&agents);
        let next_worker = next_worker_counter(&workers);
        let next_agent_session = next_agent_session_counter(&agent_sessions);
        let next_route = next_route_counter(&agent_sessions);

        Self {
            options,
            provider: Arc::from(provider),
            tools: ToolRegistry::builtin().expect("built-in tool registry should be valid"),
            started_at: Instant::now(),
            next_event: 1,
            next_session,
            next_agent,
            next_agent_session,
            next_route,
            next_worker,
            sessions,
            agents,
            agent_sessions,
            workers,
            orchestrator,
            ui_preferences,
            spawn_limits,
            agent_runtime,
            policy: PolicyEngine,
            approval_store,
            state_store,
            profile_home,
            pending_approvals,
            workspaces,
            workspace_grants,
            last_voice_preflight: None,
            recovery_diagnostics: diagnostics,
            next_tool: 1,
            next_approval,
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
                    let cancellation_requested_at = now_timestamp();
                    let mut events = self.cancel_agent_sessions_for_session(
                        &request.session_id,
                        cancellation_requested_at.clone(),
                    );
                    events.extend(self.cancel_workers_for_session(
                        &request.session_id,
                        cancellation_requested_at,
                    ));
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
                    let events = if request.include_snapshot {
                        let title = session.title.clone();
                        vec![self.session_event(
                            request.session_id.clone(),
                            CadisEvent::SessionUpdated(SessionEventPayload {
                                session_id: request.session_id,
                                title,
                            }),
                        )]
                    } else {
                        Vec::new()
                    };
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
                let models = provider_catalog_for_config(&self.model_catalog_config())
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
            ClientRequest::VoiceStatus(_) => {
                let event = self.event(None, CadisEvent::VoiceStatusUpdated(self.voice_status()));
                self.accept(request_id, vec![event])
            }
            ClientRequest::VoiceDoctor(request) => {
                let event = self.event(
                    None,
                    CadisEvent::VoiceDoctorResponse(
                        self.voice_doctor_payload(request.include_bridge),
                    ),
                );
                self.accept(request_id, vec![event])
            }
            ClientRequest::VoicePreflight(request) => {
                self.handle_voice_preflight(request_id, request)
            }
            ClientRequest::VoicePreview(request) => self.handle_voice_preview(request_id, request),
            ClientRequest::VoiceStop(_) => self.handle_voice_stop(request_id),
            ClientRequest::WorkerTail(request) => self.worker_tail(request_id, request),
            ClientRequest::WorkerCleanup(request) => self.worker_cleanup(request_id, request),
            ClientRequest::SessionUnsubscribe(_) => self.reject(
                request_id,
                "not_implemented",
                "this request is defined in the protocol but is not implemented in the desktop MVP",
                false,
            ),
        }
    }

    /// Prepares a `message.send` request without running model generation.
    ///
    /// The returned plan contains all daemon-authoritative routing/session state
    /// and enough provider context for the daemon to stream outside the runtime
    /// mutex. Non-message requests and invalid message requests return a normal
    /// request outcome.
    pub fn begin_message_request(
        &mut self,
        envelope: RequestEnvelope,
    ) -> Result<PendingMessageGeneration, Box<RequestOutcome>> {
        let request_id = envelope.request_id.clone();

        if let Err(error) = envelope.protocol_version.ensure_supported() {
            return Err(Box::new(self.reject(
                request_id,
                "unsupported_protocol_version",
                error.to_string(),
                false,
            )));
        }

        match envelope.request {
            ClientRequest::MessageSend(request) => self.begin_message(request_id, request),
            _ => Err(Box::new(self.reject(
                request_id,
                "invalid_request_type",
                "begin_message_request only accepts message.send",
                false,
            ))),
        }
    }

    fn model_catalog_config(&self) -> ModelCatalogConfig {
        ModelCatalogConfig::new(
            self.options.model_provider.clone(),
            self.options.ollama_model.clone(),
            self.options.openai_model.clone(),
            self.options.openai_api_key_configured,
        )
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
                    voice: self.voice_status(),
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
        let approval_summary = tool.approval_summary();

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

        if request.tool_name == "file.patch" {
            if let Err(error) = validate_file_patch_input(&workspace.root, &request.input) {
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
        }

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
                    summary: approval_summary,
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
                        request: Some(request),
                    },
                );

                events.push(self.session_event(
                    session_id,
                    CadisEvent::ApprovalRequested(approval_request_payload(&record)),
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
        let mut pending_request = None;
        let mut record = match self.pending_approvals.remove(&request.approval_id) {
            Some(pending) => {
                pending_request = pending.request;
                pending.record
            }
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

        match effective_decision {
            ApprovalDecision::Approved => {
                if let Some(pending_request) = pending_request {
                    events.extend(self.execute_approved_tool(&record, pending_request));
                } else {
                    events.push(self.session_event(
                        session_id,
                        CadisEvent::ToolFailed(ToolFailedPayload {
                            tool_call_id: record.tool_call_id,
                            tool_name: record.tool_name,
                            error: ErrorPayload {
                                code: "tool_execution_unavailable".to_owned(),
                                message: "approval was recovered without process-local execution context; resubmit the tool call".to_owned(),
                                retryable: false,
                            },
                            risk_class: Some(record.risk_class),
                        }),
                    ));
                }
            }
            ApprovalDecision::Denied => {
                events.push(self.session_event(
                    session_id,
                    CadisEvent::ToolFailed(ToolFailedPayload {
                        tool_call_id: record.tool_call_id,
                        tool_name: record.tool_name,
                        error: ErrorPayload {
                            code: if record.state == ApprovalState::Expired {
                                "approval_expired".to_owned()
                            } else {
                                "approval_denied".to_owned()
                            },
                            message: "approval did not authorize tool execution".to_owned(),
                            retryable: false,
                        },
                        risk_class: Some(record.risk_class),
                    }),
                ));
            }
        }

        self.accept(request_id, events)
    }

    fn handle_message(
        &mut self,
        request_id: RequestId,
        request: MessageSendRequest,
    ) -> RequestOutcome {
        match self.begin_message(request_id, request) {
            Ok(pending) => self.complete_pending_message_blocking(pending),
            Err(outcome) => *outcome,
        }
    }

    fn begin_message(
        &mut self,
        request_id: RequestId,
        request: MessageSendRequest,
    ) -> Result<PendingMessageGeneration, Box<RequestOutcome>> {
        let content_kind = request.content_kind;
        let content = request.content;
        let decision =
            match self
                .orchestrator
                .route_message(request.target_agent_id, &content, &self.agents)
            {
                Ok(decision) => decision,
                Err(error) => {
                    return Err(Box::new(self.reject(
                        request_id,
                        error.code,
                        error.message,
                        false,
                    )));
                }
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
                    Err(error) => {
                        return Err(Box::new(self.reject(
                            request_id,
                            error.code,
                            error.message,
                            false,
                        )));
                    }
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
        let session_workspace_id = session_workspace
            .as_deref()
            .and_then(|workspace| self.workspace_id_for_root(workspace));
        let worker = route.worker_summary.as_ref().map(|summary| {
            let worker_id = self.next_worker_id();
            WorkerDelegation {
                worktree: planned_worker_worktree(
                    &worker_id,
                    session_workspace.as_deref(),
                    session_workspace_id.as_deref(),
                    &route.content,
                ),
                artifacts: worker_artifact_locations(
                    &self.profile_home.worker_artifact_paths(&worker_id),
                ),
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
            return Err(Box::new(self.accept(request_id, events)));
        }

        let prompt = self.agent_prompt(&route.agent_id, &route.content);
        let selected_model = self
            .agents
            .get(&route.agent_id)
            .map(|agent| agent.model.clone())
            .filter(|model| !model.trim().is_empty());
        let response = self.accept(request_id, Vec::new()).response;

        Ok(PendingMessageGeneration {
            response,
            initial_events: events,
            provider: Arc::clone(&self.provider),
            prompt,
            selected_model,
            context: MessageGenerationContext {
                session_id,
                agent_session_id,
                content_kind,
                agent_id: route.agent_id,
                agent_name: route.agent_name,
                worker,
            },
        })
    }

    /// Creates a daemon event for a streamed model delta.
    pub fn message_delta_event(
        &mut self,
        pending: &PendingMessageGeneration,
        delta: String,
        invocation: Option<&ModelInvocation>,
    ) -> EventEnvelope {
        let model = invocation.map(model_invocation_payload);
        self.session_event(
            pending.context.session_id.clone(),
            CadisEvent::MessageDelta(MessageDeltaPayload {
                delta,
                content_kind: pending.context.content_kind,
                agent_id: Some(pending.context.agent_id.clone()),
                agent_name: Some(pending.context.agent_name.clone()),
                model,
            }),
        )
    }

    /// Returns whether a prepared model generation has been cancelled by the runtime.
    pub fn message_generation_cancelled(&self, pending: &PendingMessageGeneration) -> bool {
        self.agent_session_cancelled(&pending.context.agent_session_id)
    }

    /// Finalizes a successful streamed model generation.
    pub fn complete_message_generation(
        &mut self,
        pending: PendingMessageGeneration,
        response: ModelResponse,
        final_content: String,
        emitted_delta: bool,
    ) -> Vec<EventEnvelope> {
        if self.message_generation_cancelled(&pending) {
            return Vec::new();
        }

        let mut events = Vec::new();
        let model = Some(model_invocation_payload(&response.invocation));
        let mut content = final_content;

        if !emitted_delta {
            for delta in response.deltas {
                content.push_str(&delta);
                events.push(self.message_delta_event(&pending, delta, Some(&response.invocation)));
            }
        }

        let context = pending.context;
        if self.agent_session_timed_out(&context.agent_session_id) {
            let error_message = format!(
                "agent session exceeded default_timeout_sec={}",
                self.agent_runtime.default_timeout_sec
            );
            if let Some(event) = self.fail_agent_session(
                &context.agent_session_id,
                AgentSessionStatus::TimedOut,
                "agent_timeout",
                error_message.clone(),
            ) {
                events.push(event);
            }
            events.push(self.session_event(
                context.session_id.clone(),
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id: context.agent_id.clone(),
                    status: AgentStatus::Failed,
                    task: Some(error_message.clone()),
                }),
            ));
            if let Some(worker) = &context.worker {
                events.extend(self.fail_worker(
                    &worker.worker_id,
                    "agent_timeout",
                    error_message.clone(),
                ));
            }
            events.push(self.session_event(
                context.session_id,
                CadisEvent::SessionFailed(ErrorPayload {
                    code: "agent_timeout".to_owned(),
                    message: error_message,
                    retryable: true,
                }),
            ));
            return events;
        }

        events.push(self.session_event(
            context.session_id.clone(),
            CadisEvent::MessageCompleted(MessageCompletedPayload {
                content_kind: context.content_kind,
                content: Some(content.clone()),
                agent_id: Some(context.agent_id.clone()),
                agent_name: Some(context.agent_name.clone()),
                model,
            }),
        ));
        events.extend(self.auto_speech_events(&context.session_id, context.content_kind, &content));
        if let Some(event) = self.complete_agent_session(&context.agent_session_id, content) {
            events.push(event);
        }
        events.push(self.session_event(
            context.session_id.clone(),
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: context.agent_id.clone(),
                status: AgentStatus::Completed,
                task: None,
            }),
        ));
        if context.agent_id.as_str() != "main" {
            events.push(self.session_event(
                context.session_id.clone(),
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id: AgentId::from("main"),
                    status: AgentStatus::Completed,
                    task: None,
                }),
            ));
        }
        if let Some(worker) = &context.worker {
            events.extend(self.complete_worker(
                &worker.worker_id,
                "completed",
                worker.summary.clone(),
            ));
        }
        events.push(self.session_event(
            context.session_id.clone(),
            CadisEvent::SessionCompleted(SessionEventPayload {
                session_id: context.session_id,
                title: None,
            }),
        ));
        events
    }

    /// Finalizes a failed streamed model generation.
    pub fn fail_message_generation(
        &mut self,
        pending: PendingMessageGeneration,
        error: cadis_models::ModelError,
    ) -> Vec<EventEnvelope> {
        if error.is_cancelled() && self.message_generation_cancelled(&pending) {
            return Vec::new();
        }

        let context = pending.context;
        let error_message = error.message().to_owned();
        let mut events = Vec::new();
        if let Some(event) = self.fail_agent_session(
            &context.agent_session_id,
            AgentSessionStatus::Failed,
            error.code(),
            error_message.clone(),
        ) {
            events.push(event);
        }
        events.push(self.session_event(
            context.session_id.clone(),
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: context.agent_id.clone(),
                status: AgentStatus::Failed,
                task: Some(error_message.clone()),
            }),
        ));
        if let Some(worker) = &context.worker {
            events.extend(self.fail_worker(&worker.worker_id, error.code(), error_message.clone()));
        }
        events.push(self.session_event(
            context.session_id,
            CadisEvent::SessionFailed(ErrorPayload {
                code: error.code().to_owned(),
                message: error_message,
                retryable: error.retryable(),
            }),
        ));
        events
    }

    fn complete_pending_message_blocking(
        &mut self,
        pending: PendingMessageGeneration,
    ) -> RequestOutcome {
        let response = pending.response.clone();
        let mut events = pending.initial_events.clone();
        let provider = Arc::clone(&pending.provider);
        let mut invocation = None;
        let mut final_content = String::new();
        let mut emitted_delta = false;
        let stream_result = provider.stream_chat(
            ModelRequest::new(&pending.prompt)
                .with_selected_model(pending.selected_model.as_deref()),
            &mut |event| {
                match event {
                    ModelStreamEvent::Started(started) | ModelStreamEvent::Completed(started) => {
                        invocation = Some(started);
                    }
                    ModelStreamEvent::Delta(delta) => {
                        final_content.push_str(&delta);
                        emitted_delta = true;
                        events.push(self.message_delta_event(&pending, delta, invocation.as_ref()));
                    }
                    ModelStreamEvent::Failed(_) | ModelStreamEvent::Cancelled(_) => {}
                }
                Ok(ModelStreamControl::Continue)
            },
        );

        match stream_result {
            Ok(model_response) => {
                events.extend(self.complete_message_generation(
                    pending,
                    model_response,
                    final_content,
                    emitted_delta,
                ));
                RequestOutcome { response, events }
            }
            Err(error) => {
                events.extend(self.fail_message_generation(pending, error));
                RequestOutcome { response, events }
            }
        }
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

    fn worker_cleanup(
        &mut self,
        request_id: RequestId,
        request: WorkerCleanupRequest,
    ) -> RequestOutcome {
        let Some(worker) = self.workers.get(&request.worker_id) else {
            return self.reject(
                request_id,
                "worker_not_found",
                format!("worker '{}' was not found", request.worker_id),
                false,
            );
        };
        if !worker.is_terminal() {
            return self.reject(
                request_id,
                "worker_not_terminal",
                format!(
                    "worker '{}' is not terminal and cannot be cleaned up",
                    request.worker_id
                ),
                false,
            );
        }

        match self.plan_worker_worktree_cleanup(
            &request.worker_id,
            request.worktree_path.as_deref(),
            "explicit cleanup request",
        ) {
            Ok(events) => self.accept(request_id, events),
            Err(error) => self.reject(request_id, error.code, error.message, false),
        }
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
        let _ = self.profile_home.init_agent(&record.agent_home_template());
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

    fn handle_voice_preflight(
        &mut self,
        request_id: RequestId,
        request: VoicePreflightRequest,
    ) -> RequestOutcome {
        let checked_at = now_timestamp();
        let checks = normalize_voice_checks(request.checks);
        let status = voice_check_summary_status(&checks);
        let summary = request
            .summary
            .map(|summary| redact(&summary))
            .filter(|summary| !summary.trim().is_empty())
            .unwrap_or_else(|| voice_checks_summary(&checks));
        let surface = request
            .surface
            .map(|surface| redact(&surface))
            .filter(|surface| !surface.trim().is_empty())
            .unwrap_or_else(|| "local-bridge".to_owned());

        self.last_voice_preflight = Some(VoicePreflightRecord {
            surface,
            status,
            summary,
            checked_at,
            checks,
        });

        let status_event = self.event(None, CadisEvent::VoiceStatusUpdated(self.voice_status()));
        let response_event = self.event(
            None,
            CadisEvent::VoicePreflightResponse(self.voice_doctor_payload(true)),
        );
        self.accept(request_id, vec![status_event, response_event])
    }

    fn handle_voice_preview(
        &mut self,
        request_id: RequestId,
        request: VoicePreviewRequest,
    ) -> RequestOutcome {
        let prefs = VoiceRuntimePreferences::from_preview(&self.ui_preferences, request.prefs);
        match speech_decision(
            &prefs,
            ContentKind::Chat,
            &request.text,
            SpeechMode::Preview,
        ) {
            SpeechDecision::Speak => {
                let started = self.event(None, CadisEvent::VoicePreviewStarted(Default::default()));
                match self.speak_with_provider(&prefs, request.text.trim()) {
                    Ok(_) => {
                        let completed =
                            self.event(None, CadisEvent::VoicePreviewCompleted(Default::default()));
                        self.accept(request_id, vec![started, completed])
                    }
                    Err(error) => {
                        let failed = self.event(
                            None,
                            CadisEvent::VoicePreviewFailed(ErrorPayload {
                                code: error.code,
                                message: error.message,
                                retryable: error.retryable,
                            }),
                        );
                        self.accept(request_id, vec![started, failed])
                    }
                }
            }
            SpeechDecision::Blocked(reason) | SpeechDecision::RequiresSummary(reason) => {
                let failed = self.event(
                    None,
                    CadisEvent::VoicePreviewFailed(ErrorPayload {
                        code: reason.to_owned(),
                        message: "voice preview text is not speakable by daemon policy".to_owned(),
                        retryable: false,
                    }),
                );
                self.accept(request_id, vec![failed])
            }
        }
    }

    fn handle_voice_stop(&mut self, request_id: RequestId) -> RequestOutcome {
        let prefs = VoiceRuntimePreferences::from_options(&self.ui_preferences);
        let mut provider = tts_provider_from_config(&prefs.provider);
        let event = match provider.stop() {
            Ok(()) => CadisEvent::VoicePreviewCompleted(Default::default()),
            Err(error) => CadisEvent::VoicePreviewFailed(ErrorPayload {
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            }),
        };
        let event = self.event(None, event);
        self.accept(request_id, vec![event])
    }

    fn speak_with_provider(
        &self,
        prefs: &VoiceRuntimePreferences,
        text: &str,
    ) -> Result<TtsOutput, TtsError> {
        let mut provider = tts_provider_from_config(&prefs.provider);
        provider.speak(TtsRequest {
            text,
            voice_id: &prefs.voice_id,
            rate: prefs.rate,
            pitch: prefs.pitch,
            volume: prefs.volume,
        })
    }

    fn auto_speech_events(
        &mut self,
        session_id: &SessionId,
        content_kind: ContentKind,
        text: &str,
    ) -> Vec<EventEnvelope> {
        let prefs = VoiceRuntimePreferences::from_options(&self.ui_preferences);
        if speech_decision(&prefs, content_kind, text, SpeechMode::AutoSpeak)
            != SpeechDecision::Speak
        {
            return Vec::new();
        }

        match self.speak_with_provider(&prefs, text.trim()) {
            Ok(_) => vec![
                self.session_event(
                    session_id.clone(),
                    CadisEvent::VoiceStarted(Default::default()),
                ),
                self.session_event(
                    session_id.clone(),
                    CadisEvent::VoiceCompleted(Default::default()),
                ),
            ],
            Err(_) => Vec::new(),
        }
    }

    fn voice_status(&self) -> VoiceStatusPayload {
        let prefs = VoiceRuntimePreferences::from_options(&self.ui_preferences);
        let checks = self.voice_doctor_checks(true);
        let state = if !prefs.enabled {
            VoiceRuntimeState::Disabled
        } else {
            voice_runtime_state(&checks)
        };

        VoiceStatusPayload {
            enabled: prefs.enabled,
            state,
            provider: prefs.provider,
            voice_id: prefs.voice_id,
            stt_language: prefs.stt_language,
            max_spoken_chars: prefs.max_spoken_chars,
            bridge: "hud-local".to_owned(),
            last_preflight: self.last_voice_preflight.as_ref().map(|preflight| {
                VoicePreflightSummary {
                    surface: preflight.surface.clone(),
                    status: preflight.status.clone(),
                    summary: preflight.summary.clone(),
                    checked_at: preflight.checked_at.clone(),
                }
            }),
        }
    }

    fn voice_doctor_payload(&self, include_bridge: bool) -> VoiceDoctorPayload {
        VoiceDoctorPayload {
            status: self.voice_status(),
            checks: self.voice_doctor_checks(include_bridge),
        }
    }

    fn voice_doctor_checks(&self, include_bridge: bool) -> Vec<VoiceDoctorCheck> {
        let prefs = VoiceRuntimePreferences::from_options(&self.ui_preferences);
        let provider = tts_provider_from_config(&prefs.provider);
        let voice_count = provider.supported_voices().len();
        let mut checks = Vec::new();

        checks.push(VoiceDoctorCheck {
            name: "voice.config".to_owned(),
            status: "ok".to_owned(),
            message: if prefs.enabled {
                "voice output is enabled".to_owned()
            } else {
                "voice output is disabled".to_owned()
            },
        });

        checks.push(VoiceDoctorCheck {
            name: "voice.provider".to_owned(),
            status: if is_supported_voice_provider(&prefs.provider) {
                "ok"
            } else {
                "error"
            }
            .to_owned(),
            message: if is_supported_voice_provider(&prefs.provider) {
                format!(
                    "configured provider {} ({}, {} curated voices)",
                    provider.id(),
                    provider.label(),
                    voice_count
                )
            } else {
                format!(
                    "unsupported provider {}; expected edge, openai, system, or stub",
                    prefs.provider
                )
            },
        });

        checks.push(VoiceDoctorCheck {
            name: "voice.tts_voice".to_owned(),
            status: if prefs.voice_id.trim().is_empty() {
                "error"
            } else {
                "ok"
            }
            .to_owned(),
            message: if prefs.voice_id.trim().is_empty() {
                "voice_id is empty".to_owned()
            } else {
                format!("configured voice {}", prefs.voice_id)
            },
        });

        checks.push(VoiceDoctorCheck {
            name: "voice.stt_language".to_owned(),
            status: "ok".to_owned(),
            message: format!("STT language {}", prefs.stt_language),
        });

        checks.push(VoiceDoctorCheck {
            name: "voice.bridge".to_owned(),
            status: "ok".to_owned(),
            message: "HUD remains the local capture/playback bridge for microphone, MediaRecorder, WebAudio PCM fallback, and native audio playback".to_owned(),
        });

        if include_bridge {
            if let Some(preflight) = &self.last_voice_preflight {
                checks.push(VoiceDoctorCheck {
                    name: "voice.preflight".to_owned(),
                    status: preflight.status.clone(),
                    message: format!(
                        "{} from {} at {}",
                        preflight.summary, preflight.surface, preflight.checked_at
                    ),
                });
                checks.extend(preflight.checks.clone());
            } else {
                checks.push(VoiceDoctorCheck {
                    name: "voice.preflight".to_owned(),
                    status: "warn".to_owned(),
                    message: "no local bridge preflight has been reported; run HUD voice doctor"
                        .to_owned(),
                });
            }
        }

        checks
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

    fn pending_approval_records_sorted(&self) -> Vec<ApprovalRecord> {
        let mut records = self
            .pending_approvals
            .values()
            .map(|pending| pending.record.clone())
            .filter(|record| !approval_is_expired(record))
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
        records
    }

    fn snapshot_events(&mut self) -> Vec<EventEnvelope> {
        let agents = self
            .agent_records_sorted()
            .into_iter()
            .map(AgentRecord::event_payload)
            .collect();
        let diagnostics = self.recovery_diagnostics.clone();
        let mut events = diagnostics
            .into_iter()
            .map(|diagnostic| self.event(None, CadisEvent::DaemonError(diagnostic)))
            .collect::<Vec<_>>();

        events.extend([
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
            self.event(None, CadisEvent::VoiceStatusUpdated(self.voice_status())),
        ]);

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
            let session_id = worker.session_id.clone();
            let payload = worker.event_payload();
            let event = worker_lifecycle_event(payload);
            events.push(self.session_event(session_id, event));
        }

        for record in self.pending_approval_records_sorted() {
            events.push(self.session_event(
                record.session_id.clone(),
                CadisEvent::ApprovalRequested(approval_request_payload(&record)),
            ));
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

    fn agent_session_cancelled(&self, agent_session_id: &AgentSessionId) -> bool {
        self.agent_sessions
            .get(agent_session_id)
            .is_some_and(|record| record.status == AgentSessionStatus::Cancelled)
    }

    fn cancel_agent_sessions_for_session(
        &mut self,
        session_id: &SessionId,
        cancellation_requested_at: Timestamp,
    ) -> Vec<EventEnvelope> {
        let mut agent_session_ids = self
            .agent_sessions
            .iter()
            .filter(|(_, record)| {
                &record.session_id == session_id && !agent_session_is_terminal(record.status)
            })
            .map(|(agent_session_id, _)| agent_session_id.clone())
            .collect::<Vec<_>>();
        agent_session_ids.sort();

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

    fn cancel_workers_for_session(
        &mut self,
        session_id: &SessionId,
        cancellation_requested_at: Timestamp,
    ) -> Vec<EventEnvelope> {
        let mut worker_ids = self
            .workers
            .iter()
            .filter(|(_, record)| &record.session_id == session_id && !record.is_terminal())
            .map(|(worker_id, _)| worker_id.clone())
            .collect::<Vec<_>>();
        worker_ids.sort();

        let mut events = Vec::new();
        for worker_id in worker_ids {
            events.extend(self.cancel_worker(&worker_id, cancellation_requested_at.clone()));
        }
        events
    }

    fn start_worker(
        &mut self,
        session_id: SessionId,
        agent_id: AgentId,
        worker: &WorkerDelegation,
    ) -> Vec<EventEnvelope> {
        let mut record = WorkerRecord::from_delegation(session_id, Some(agent_id), worker);
        let worker_id = record.worker_id.clone();
        let preparation_logs = prepare_worker_execution(&mut record);
        let mut events = vec![self.session_event(
            record.session_id.clone(),
            CadisEvent::WorkerStarted(record.event_payload()),
        )];
        self.workers.insert(worker_id.clone(), record);
        let _ = self.persist_worker_record(&worker_id);
        if let Some(event) =
            self.append_worker_log(&worker_id, format!("started: {}\n", worker.summary))
        {
            events.push(event);
        }
        for line in preparation_logs {
            if let Some(event) = self.append_worker_log(&worker_id, line) {
                events.push(event);
            }
        }
        events
    }

    fn complete_worker(
        &mut self,
        worker_id: &str,
        status: &str,
        summary: String,
    ) -> Vec<EventEnvelope> {
        if status == "completed" {
            let mut events = Vec::new();
            let command_result = self.execute_worker_command(worker_id);
            for line in command_result.logs {
                if let Some(event) = self.append_worker_log(worker_id, line) {
                    events.push(event);
                }
            }
            if let Some(failure) = command_result.failure {
                events.extend(self.finish_worker(
                    worker_id,
                    "failed",
                    failure.message.clone(),
                    WorkerFinishOptions {
                        error_code: Some(failure.code),
                        error: Some(failure.message),
                        cancellation_requested_at: None,
                        write_artifacts: true,
                    },
                ));
                return events;
            }
            events.extend(self.finish_worker(
                worker_id,
                status,
                summary,
                WorkerFinishOptions {
                    error_code: None,
                    error: None,
                    cancellation_requested_at: None,
                    write_artifacts: true,
                },
            ));
            return events;
        }

        self.finish_worker(
            worker_id,
            status,
            summary,
            WorkerFinishOptions {
                error_code: None,
                error: None,
                cancellation_requested_at: None,
                write_artifacts: true,
            },
        )
    }

    fn fail_worker(
        &mut self,
        worker_id: &str,
        error_code: &str,
        error_message: String,
    ) -> Vec<EventEnvelope> {
        self.finish_worker(
            worker_id,
            "failed",
            error_message.clone(),
            WorkerFinishOptions {
                error_code: Some(error_code.to_owned()),
                error: Some(error_message),
                cancellation_requested_at: None,
                write_artifacts: true,
            },
        )
    }

    fn cancel_worker(
        &mut self,
        worker_id: &str,
        cancellation_requested_at: Timestamp,
    ) -> Vec<EventEnvelope> {
        let reason = "session was cancelled".to_owned();
        self.finish_worker(
            worker_id,
            "cancelled",
            reason.clone(),
            WorkerFinishOptions {
                error_code: Some("session_cancelled".to_owned()),
                error: Some(reason),
                cancellation_requested_at: Some(cancellation_requested_at),
                write_artifacts: false,
            },
        )
    }

    fn finish_worker(
        &mut self,
        worker_id: &str,
        status: &str,
        summary: String,
        options: WorkerFinishOptions,
    ) -> Vec<EventEnvelope> {
        let mut events = Vec::new();
        if let Some(event) = self.append_worker_log(worker_id, format!("{status}: {summary}\n")) {
            events.push(event);
        }
        if options.write_artifacts {
            for line in self.write_worker_artifacts(worker_id, status, &summary) {
                if let Some(event) = self.append_worker_log(worker_id, line) {
                    events.push(event);
                }
            }
        }
        events.extend(self.plan_worker_terminal_cleanup(worker_id, status));
        if let Some(event) = self.update_worker_status(
            worker_id,
            status,
            Some(summary),
            options.error_code,
            options.error,
            options.cancellation_requested_at,
        ) {
            events.push(event);
        }
        events
    }

    fn write_worker_artifacts(
        &mut self,
        worker_id: &str,
        status: &str,
        summary: &str,
    ) -> Vec<String> {
        let Some(worker) = self.workers.get_mut(worker_id) else {
            return Vec::new();
        };
        write_worker_artifacts(worker, status, summary)
    }

    fn execute_worker_command(&mut self, worker_id: &str) -> WorkerCommandExecution {
        let Some(worker) = self.workers.get_mut(worker_id) else {
            return WorkerCommandExecution::default();
        };
        execute_worker_command(worker)
    }

    fn plan_worker_terminal_cleanup(
        &mut self,
        worker_id: &str,
        status: &str,
    ) -> Vec<EventEnvelope> {
        if !worker_status_is_terminal(status) {
            return Vec::new();
        }
        let Some(target_state) = self
            .workers
            .get(worker_id)
            .and_then(|worker| worker_terminal_worktree_state(worker, status))
        else {
            return Vec::new();
        };

        match target_state {
            WorkerWorktreeState::CleanupPending => {
                match self.transition_worker_worktree_state(
                    worker_id,
                    None,
                    WorkerWorktreeState::CleanupPending,
                ) {
                    Ok(()) => self
                        .append_worker_log(
                            worker_id,
                            "cleanup pending: terminal worker state; files were not removed\n",
                        )
                        .into_iter()
                        .collect(),
                    Err(error) => self.worker_cleanup_failed_events(worker_id, error),
                }
            }
            WorkerWorktreeState::ReviewPending => {
                match self.transition_worker_worktree_state(
                    worker_id,
                    None,
                    WorkerWorktreeState::ReviewPending,
                ) {
                    Ok(()) => Vec::new(),
                    Err(error) => self.worker_cleanup_failed_events(worker_id, error),
                }
            }
            _ => Vec::new(),
        }
    }

    fn plan_worker_worktree_cleanup(
        &mut self,
        worker_id: &str,
        requested_path: Option<&str>,
        reason: &str,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        self.transition_worker_worktree_state(
            worker_id,
            requested_path,
            WorkerWorktreeState::CleanupPending,
        )?;

        let mut events = Vec::new();
        if let Some(event) = self.append_worker_log(
            worker_id,
            format!("cleanup requested: {reason}; files were not removed\n"),
        ) {
            events.push(event);
        }
        if let Some((session_id, payload)) = self
            .workers
            .get(worker_id)
            .map(|worker| (worker.session_id.clone(), worker.event_payload()))
        {
            events
                .push(self.session_event(session_id, CadisEvent::WorkerCleanupRequested(payload)));
        }
        Ok(events)
    }

    fn transition_worker_worktree_state(
        &mut self,
        worker_id: &str,
        requested_path: Option<&str>,
        target_state: WorkerWorktreeState,
    ) -> Result<(), RuntimeError> {
        let verified = self.verify_cadis_worker_worktree(worker_id, requested_path)?;
        let project_state = project_worker_worktree_state_for_worker_state(target_state);

        let mut metadata = verified.metadata;
        metadata.state = project_state;
        verified
            .store
            .save_worker_worktree_metadata(&metadata)
            .map_err(|error| RuntimeError {
                code: "worker_worktree_metadata_persist_failed",
                message: format!(
                    "worker '{}' worktree metadata could not be updated: {error}",
                    worker_id
                ),
            })?;

        if let Some(worker) = self.workers.get_mut(worker_id) {
            if let Some(worktree) = &mut worker.worktree {
                worktree.state = target_state;
            }
            worker.updated_at = now_timestamp();
        }
        self.persist_worker_record(worker_id)
            .map_err(|error| RuntimeError {
                code: "worker_metadata_persist_failed",
                message: format!("worker '{worker_id}' metadata could not be updated: {error}"),
            })?;

        Ok(())
    }

    fn verify_cadis_worker_worktree(
        &self,
        worker_id: &str,
        requested_path: Option<&str>,
    ) -> Result<VerifiedWorkerWorktree, RuntimeError> {
        let worker = self.workers.get(worker_id).ok_or(RuntimeError {
            code: "worker_not_found",
            message: format!("worker '{worker_id}' was not found"),
        })?;
        let worktree = worker.worktree.as_ref().ok_or(RuntimeError {
            code: "worker_worktree_not_owned",
            message: format!("worker '{worker_id}' has no daemon-owned worktree metadata"),
        })?;
        let Some(project_root) = worktree.project_root.as_deref() else {
            return Err(RuntimeError {
                code: "worker_worktree_not_owned",
                message: format!("worker '{worker_id}' is not bound to a CADIS project worktree"),
            });
        };

        let project_root = fs::canonicalize(project_root).map_err(|error| RuntimeError {
            code: "worker_workspace_missing",
            message: format!(
                "worker '{worker_id}' project root '{}' is unavailable: {error}",
                project_root
            ),
        })?;
        let store = ProjectWorkspaceStore::new(&project_root);
        let workspace_metadata = store.load().map_err(|error| RuntimeError {
            code: "worker_worktree_metadata_unreadable",
            message: format!(
                "worker '{worker_id}' project workspace metadata could not be read: {error}"
            ),
        })?;
        let paths = store.worker_worktree_paths(worker_id, workspace_metadata.as_ref());
        let cadis_worktree_root =
            fs::canonicalize(project_root.join(".cadis/worktrees")).map_err(|error| {
                RuntimeError {
                    code: "worker_worktree_not_owned",
                    message: format!(
                        "worker '{worker_id}' CADIS worktree root is unavailable: {error}"
                    ),
                }
            })?;
        let expected_path =
            fs::canonicalize(&paths.worktree_path).map_err(|error| RuntimeError {
                code: "worker_worktree_missing",
                message: format!(
                    "worker '{worker_id}' worktree '{}' is unavailable: {error}",
                    paths.worktree_path.display()
                ),
            })?;
        if expected_path == cadis_worktree_root || !expected_path.starts_with(&cadis_worktree_root)
        {
            return Err(RuntimeError {
                code: "worker_worktree_not_owned",
                message: format!(
                    "worker '{worker_id}' worktree '{}' is outside the CADIS worktree root",
                    expected_path.display()
                ),
            });
        }

        let record_path = resolve_project_path(&project_root, &worktree.worktree_path);
        let record_path = fs::canonicalize(&record_path).map_err(|error| RuntimeError {
            code: "worker_worktree_missing",
            message: format!(
                "worker '{worker_id}' recorded worktree '{}' is unavailable: {error}",
                record_path.display()
            ),
        })?;
        if record_path != expected_path {
            return Err(RuntimeError {
                code: "worker_worktree_not_owned",
                message: format!(
                    "worker '{worker_id}' recorded worktree '{}' does not match CADIS path '{}'",
                    record_path.display(),
                    expected_path.display()
                ),
            });
        }

        if let Some(requested_path) = requested_path {
            let requested_path = resolve_project_path(&project_root, requested_path);
            let requested_path =
                fs::canonicalize(&requested_path).map_err(|error| RuntimeError {
                    code: "worker_worktree_missing",
                    message: format!(
                        "worker '{worker_id}' requested worktree '{}' is unavailable: {error}",
                        requested_path.display()
                    ),
                })?;
            if requested_path != expected_path {
                return Err(RuntimeError {
                    code: "worker_worktree_not_owned",
                    message: format!(
                        "worker '{worker_id}' requested worktree '{}' does not match CADIS path '{}'",
                        requested_path.display(),
                        expected_path.display()
                    ),
                });
            }
        }

        let metadata = store
            .load_worker_worktree_metadata(worker_id)
            .map_err(|error| RuntimeError {
                code: "worker_worktree_metadata_unreadable",
                message: format!(
                    "worker '{worker_id}' worktree metadata could not be read: {error}"
                ),
            })?
            .ok_or(RuntimeError {
                code: "worker_worktree_metadata_missing",
                message: format!("worker '{worker_id}' has no project-local worktree metadata"),
            })?;
        if metadata.worker_id != worker_id {
            return Err(RuntimeError {
                code: "worker_worktree_not_owned",
                message: format!(
                    "worker '{worker_id}' metadata belongs to worker '{}'",
                    metadata.worker_id
                ),
            });
        }
        let metadata_path = resolve_project_path(&project_root, &metadata.worktree_path);
        let metadata_path = fs::canonicalize(&metadata_path).map_err(|error| RuntimeError {
            code: "worker_worktree_missing",
            message: format!(
                "worker '{worker_id}' metadata worktree '{}' is unavailable: {error}",
                metadata_path.display()
            ),
        })?;
        if metadata_path != expected_path {
            return Err(RuntimeError {
                code: "worker_worktree_not_owned",
                message: format!(
                    "worker '{worker_id}' metadata worktree '{}' does not match CADIS path '{}'",
                    metadata_path.display(),
                    expected_path.display()
                ),
            });
        }
        if metadata.state == ProjectWorkerWorktreeState::Removed {
            return Err(RuntimeError {
                code: "worker_worktree_not_owned",
                message: format!("worker '{worker_id}' worktree metadata is already removed"),
            });
        }

        Ok(VerifiedWorkerWorktree { store, metadata })
    }

    fn worker_cleanup_failed_events(
        &mut self,
        worker_id: &str,
        error: RuntimeError,
    ) -> Vec<EventEnvelope> {
        self.append_worker_log(
            worker_id,
            format!(
                "cleanup planning failed closed: {}: {}\n",
                error.code, error.message
            ),
        )
        .into_iter()
        .collect()
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
        error_code: Option<String>,
        error: Option<String>,
        cancellation_requested_at: Option<Timestamp>,
    ) -> Option<EventEnvelope> {
        let (session_id, payload) = {
            let worker = self.workers.get_mut(worker_id)?;
            worker.status = status.to_owned();
            if let Some(summary) = summary {
                worker.summary = Some(redact(&summary));
            }
            worker.error_code = error_code;
            worker.error = error.map(|error| redact(&error));
            worker.cancellation_requested_at = cancellation_requested_at;
            worker.updated_at = now_timestamp();
            (worker.session_id.clone(), worker.event_payload())
        };
        let _ = self.persist_worker_record(worker_id);

        Some(self.session_event(session_id, worker_lifecycle_event(payload)))
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
            checks.extend(self.workspace_duplicate_root_checks());
        }

        checks.extend(self.profile_agent_doctor_checks());

        if let Some(workspace_id) = request.workspace_id {
            match self.workspaces.get(&workspace_id) {
                Some(workspace) => {
                    checks.push(root_check("workspace.root", &workspace.root));
                    checks.extend(project_workspace_metadata_checks(workspace));
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
                Ok(root) => {
                    checks.push(root_check("request.root", &root));
                    checks.extend(project_workspace_metadata_checks_for_root(&root));
                    checks.extend(project_worker_worktree_checks_for_root(&root));
                }
                Err(error) => checks.push(WorkspaceDoctorCheck {
                    name: "request.root".to_owned(),
                    status: "error".to_owned(),
                    message: error.to_string(),
                }),
            }
        }

        checks
    }

    fn profile_agent_doctor_checks(&self) -> Vec<WorkspaceDoctorCheck> {
        match self
            .profile_home
            .agent_doctor_diagnostics(AgentHomeDoctorOptions::default())
        {
            Ok(diagnostics) => diagnostics
                .into_iter()
                .map(agent_home_diagnostic_check)
                .collect(),
            Err(error) => vec![WorkspaceDoctorCheck {
                name: "profile.agents".to_owned(),
                status: "error".to_owned(),
                message: format!("could not inspect agent homes: {error}"),
            }],
        }
    }

    fn workspace_duplicate_root_checks(&self) -> Vec<WorkspaceDoctorCheck> {
        let mut roots: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for workspace in self.workspaces.values() {
            roots
                .entry(workspace.root.clone())
                .or_default()
                .push(workspace.id.to_string());
        }

        roots
            .into_iter()
            .filter_map(|(root, mut ids)| {
                if ids.len() <= 1 {
                    return None;
                }
                ids.sort();
                Some(WorkspaceDoctorCheck {
                    name: "registry.duplicate_root".to_owned(),
                    status: "warn".to_owned(),
                    message: format!("{} is registered by {}", root.display(), ids.join(", ")),
                })
            })
            .collect()
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

    fn workspace_id_for_root(&self, root: &str) -> Option<String> {
        let root = canonical_workspace_root(root).ok()?;
        self.workspaces
            .iter()
            .find(|(_, record)| record.root == root)
            .map(|(workspace_id, _)| workspace_id.to_string())
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
                "git.diff" => self.execute_git_diff(workspace, &request.input),
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

    fn execute_approved_tool(
        &mut self,
        record: &ApprovalRecord,
        request: ToolCallRequest,
    ) -> Vec<EventEnvelope> {
        if request.tool_name != record.tool_name {
            return vec![self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolFailed(ToolFailedPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: record.tool_name.clone(),
                    error: tool_error(
                        "approval_request_mismatch",
                        "approved tool request did not match the approval record",
                        false,
                    ),
                    risk_class: Some(record.risk_class),
                }),
            )];
        }

        let tool_timeout_secs = match self.tools.get(&request.tool_name) {
            Some(tool) if tool.execution == ToolExecutionMode::ApprovalPlaceholder => {
                tool.timeout_secs
            }
            Some(_) => {
                return vec![self.session_event(
                    record.session_id.clone(),
                    CadisEvent::ToolFailed(ToolFailedPayload {
                        tool_call_id: record.tool_call_id.clone(),
                        tool_name: request.tool_name,
                        error: tool_error(
                            "tool_execution_blocked",
                            "approved execution is only available for approval-gated tools",
                            false,
                        ),
                        risk_class: Some(record.risk_class),
                    }),
                )]
            }
            None => {
                return vec![self.session_event(
                    record.session_id.clone(),
                    CadisEvent::ToolFailed(ToolFailedPayload {
                        tool_call_id: record.tool_call_id.clone(),
                        tool_name: request.tool_name,
                        error: tool_error(
                            "tool_not_found",
                            "approved tool is not registered",
                            false,
                        ),
                        risk_class: Some(record.risk_class),
                    }),
                )]
            }
        };

        let workspace = match self.resolved_granted_workspace(
            &record.session_id,
            request.agent_id.as_ref(),
            &request.input,
            required_tool_access(&request.tool_name),
        ) {
            Ok(workspace) => workspace,
            Err(error) => {
                return vec![self.session_event(
                    record.session_id.clone(),
                    CadisEvent::ToolFailed(ToolFailedPayload {
                        tool_call_id: record.tool_call_id.clone(),
                        tool_name: request.tool_name,
                        error,
                        risk_class: Some(record.risk_class),
                    }),
                )]
            }
        };

        let mut events = vec![self.session_event(
            record.session_id.clone(),
            CadisEvent::ToolStarted(ToolEventPayload {
                tool_call_id: record.tool_call_id.clone(),
                tool_name: request.tool_name.clone(),
                summary: Some("approved tool execution started".to_owned()),
                risk_class: Some(record.risk_class),
                output: None,
            }),
        )];

        match self.execute_approved_tool_backend(&workspace.root, &request, tool_timeout_secs) {
            Ok(result) => events.push(self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolCompleted(ToolEventPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: request.tool_name,
                    summary: Some(result.summary),
                    risk_class: Some(record.risk_class),
                    output: Some(result.output),
                }),
            )),
            Err(error) => events.push(self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolFailed(ToolFailedPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: request.tool_name,
                    error,
                    risk_class: Some(record.risk_class),
                }),
            )),
        }

        events
    }

    fn execute_approved_tool_backend(
        &self,
        workspace: &Path,
        request: &ToolCallRequest,
        tool_timeout_secs: u64,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        match request.tool_name.as_str() {
            "shell.run" => self.execute_shell_run(workspace, &request.input, tool_timeout_secs),
            "file.patch" => self.execute_file_patch(workspace, &request.input),
            tool_name => Err(tool_error(
                "tool_not_implemented",
                format!("{tool_name} has no approved execution backend"),
                false,
            )),
        }
    }

    fn execute_file_patch(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let operations = parse_file_patch_operations(input)?;
        let prepared = prepare_file_patch(workspace, &operations)?;

        for change in &prepared {
            fs::write(&change.path, change.content.as_bytes()).map_err(|error| {
                tool_error(
                    "file_patch_write_failed",
                    format!("could not write {}: {error}", change.display_path),
                    false,
                )
            })?;
        }

        let output_files = prepared
            .iter()
            .take(FILE_PATCH_OUTPUT_MAX_FILES)
            .map(|change| {
                serde_json::json!({
                    "path": redact(&change.display_path),
                    "action": change.action,
                })
            })
            .collect::<Vec<_>>();
        let truncated = prepared.len() > FILE_PATCH_OUTPUT_MAX_FILES;
        let summary = format!("patched {} file{}", prepared.len(), plural(prepared.len()));

        Ok(ToolExecutionResult {
            summary,
            output: serde_json::json!({
                "schema": "structured_replace_write_v1",
                "files": output_files,
                "truncated": truncated
            }),
        })
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

    fn execute_git_diff(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let cwd = input_string(input, "path")
            .or_else(|| input_string(input, "cwd"))
            .unwrap_or_else(|| ".".to_owned());
        let cwd = resolve_inside_workspace(workspace, &cwd)?;
        let pathspec = input_string(input, "pathspec")
            .or_else(|| input_string(input, "target"))
            .map(|value| validate_git_pathspec(&value))
            .transpose()?;

        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&cwd)
            .args(["diff", "--no-ext-diff", "--no-color", "--"]);
        if let Some(pathspec) = &pathspec {
            command.arg(pathspec);
        }

        let output = command
            .output()
            .map_err(|error| tool_error("git_diff_failed", error.to_string(), false))?;
        if !output.status.success() {
            let stderr = redact(&String::from_utf8_lossy(&output.stderr));
            return Err(tool_error(
                "git_diff_failed",
                if stderr.trim().is_empty() {
                    "git diff failed".to_owned()
                } else {
                    stderr
                },
                false,
            ));
        }

        let truncated = output.stdout.len() > GIT_DIFF_LIMIT_BYTES;
        let visible = if truncated {
            &output.stdout[..GIT_DIFF_LIMIT_BYTES]
        } else {
            &output.stdout
        };
        let diff = redact(&String::from_utf8_lossy(visible));
        let summary = if diff.trim().is_empty() {
            "no diff".to_owned()
        } else {
            diff.clone()
        };

        Ok(ToolExecutionResult {
            summary,
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "pathspec": pathspec,
                "diff": diff,
                "truncated": truncated
            }),
        })
    }

    fn execute_shell_run(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
        tool_timeout_secs: u64,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let command = input_string(input, "command")
            .ok_or_else(|| tool_error("invalid_tool_input", "shell.run requires command", false))?;
        if command.contains('\0') {
            return Err(tool_error(
                "invalid_tool_input",
                "shell.run command cannot contain NUL bytes",
                false,
            ));
        }

        let cwd = input_string(input, "cwd")
            .or_else(|| input_string(input, "path"))
            .unwrap_or_else(|| ".".to_owned());
        let cwd = resolve_inside_workspace(workspace, &cwd)?;
        validate_shell_cwd(&cwd, &self.options.cadis_home)?;

        let timeout = shell_timeout(input, tool_timeout_secs)?;
        let result = run_shell_command(&cwd, &command, timeout)?;
        let stdout = redact(&String::from_utf8_lossy(&result.stdout.bytes));
        let stderr = redact(&String::from_utf8_lossy(&result.stderr.bytes));
        let timeout_ms = duration_millis(timeout);

        if result.timed_out {
            return Err(tool_error(
                "tool_timeout",
                format!("shell.run exceeded timeout_ms={timeout_ms}"),
                false,
            ));
        }

        if !result.status_success {
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_owned()
            } else if !stdout.trim().is_empty() {
                stdout.trim().to_owned()
            } else {
                "command exited without output".to_owned()
            };
            return Err(tool_error(
                "shell_command_failed",
                format!(
                    "shell.run exited with code {:?}: {}",
                    result.exit_code, detail
                ),
                false,
            ));
        }

        Ok(ToolExecutionResult {
            summary: shell_summary(
                &stdout,
                &stderr,
                result.stdout.truncated,
                result.stderr.truncated,
            ),
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "exit_code": result.exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "stdout_truncated": result.stdout.truncated,
                "stderr_truncated": result.stderr.truncated,
                "timeout_ms": timeout_ms
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

    fn persist_worker_record(&self, worker_id: &str) -> Result<(), cadis_store::StoreError> {
        if let Some(record) = self.workers.get(worker_id) {
            self.state_store
                .write_worker_metadata(worker_id, &WorkerMetadata::from_record(record))?;
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
    fn agent_home_template(&self) -> AgentHomeTemplate {
        AgentHomeTemplate::new(
            self.id.clone(),
            self.display_name.clone(),
            self.role.clone(),
            self.parent_agent_id.clone(),
            self.model.clone(),
        )
    }

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

#[derive(Clone, Debug, PartialEq)]
struct AgentSessionRecovery {
    records: HashMap<AgentSessionId, AgentSessionRecord>,
    diagnostics: Vec<ErrorPayload>,
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
    error_code: Option<String>,
    error: Option<String>,
    cancellation_requested_at: Option<Timestamp>,
    worktree: Option<WorkerWorktreeIntent>,
    artifacts: Option<WorkerArtifactLocations>,
    updated_at: Timestamp,
    log_lines: Vec<String>,
    command_report: Option<WorkerCommandReport>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerFinishOptions {
    error_code: Option<String>,
    error: Option<String>,
    cancellation_requested_at: Option<Timestamp>,
    write_artifacts: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerCommandReport {
    command: String,
    cwd: String,
    status: String,
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    timeout_ms: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct WorkerCommandExecution {
    logs: Vec<String>,
    failure: Option<WorkerCommandFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerCommandFailure {
    code: String,
    message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct WorkerMetadata {
    worker_id: String,
    session_id: SessionId,
    agent_id: Option<AgentId>,
    parent_agent_id: Option<AgentId>,
    status: String,
    cli: Option<String>,
    cwd: Option<String>,
    summary: Option<String>,
    error_code: Option<String>,
    error: Option<String>,
    cancellation_requested_at: Option<Timestamp>,
    worktree: Option<WorkerWorktreeIntent>,
    artifacts: Option<WorkerArtifactLocations>,
    updated_at: Timestamp,
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
            error_code: None,
            error: None,
            cancellation_requested_at: None,
            worktree: Some(worker.worktree.clone()),
            artifacts: Some(worker.artifacts.clone()),
            updated_at: now_timestamp(),
            log_lines: Vec::new(),
            command_report: None,
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
            error_code: self.error_code.clone(),
            error: self.error.clone(),
            cancellation_requested_at: self.cancellation_requested_at.clone(),
            worktree: self.worktree.clone(),
            artifacts: self.artifacts.clone(),
        }
    }

    fn is_terminal(&self) -> bool {
        worker_status_is_terminal(&self.status)
    }
}

impl WorkerMetadata {
    fn from_record(record: &WorkerRecord) -> Self {
        Self {
            worker_id: record.worker_id.clone(),
            session_id: record.session_id.clone(),
            agent_id: record.agent_id.clone(),
            parent_agent_id: record.parent_agent_id.clone(),
            status: record.status.clone(),
            cli: record.cli.clone(),
            cwd: record.cwd.clone(),
            summary: record.summary.clone(),
            error_code: record.error_code.clone(),
            error: record.error.clone(),
            cancellation_requested_at: record.cancellation_requested_at.clone(),
            worktree: record.worktree.clone(),
            artifacts: record.artifacts.clone(),
            updated_at: record.updated_at.clone(),
        }
    }

    fn into_record(self) -> (String, WorkerRecord) {
        let worker_id = self.worker_id;
        (
            worker_id.clone(),
            WorkerRecord {
                worker_id,
                session_id: self.session_id,
                agent_id: self.agent_id,
                parent_agent_id: self.parent_agent_id,
                status: self.status,
                cli: self.cli,
                cwd: self.cwd,
                summary: self.summary,
                error_code: self.error_code,
                error: self.error,
                cancellation_requested_at: self.cancellation_requested_at,
                worktree: self.worktree,
                artifacts: self.artifacts,
                updated_at: self.updated_at,
                log_lines: Vec::new(),
                command_report: None,
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PendingApproval {
    record: ApprovalRecord,
    request: Option<ToolCallRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VoicePreflightRecord {
    surface: String,
    status: String,
    summary: String,
    checked_at: Timestamp,
    checks: Vec<VoiceDoctorCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VoiceRuntimePreferences {
    enabled: bool,
    provider: String,
    voice_id: String,
    stt_language: String,
    rate: i16,
    pitch: i16,
    volume: i16,
    auto_speak: bool,
    max_spoken_chars: usize,
}

impl VoiceRuntimePreferences {
    fn from_options(options: &serde_json::Value) -> Self {
        let voice = options.get("voice").and_then(serde_json::Value::as_object);

        Self {
            enabled: voice
                .and_then(|voice| voice.get("enabled"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            provider: voice
                .and_then(|voice| voice.get("provider"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_voice_config_string)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "edge".to_owned()),
            voice_id: voice
                .and_then(|voice| voice.get("voice_id"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_voice_config_string)
                .unwrap_or_else(|| "id-ID-GadisNeural".to_owned()),
            stt_language: voice
                .and_then(|voice| voice.get("stt_language"))
                .and_then(serde_json::Value::as_str)
                .map(normalize_voice_config_string)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "auto".to_owned()),
            rate: voice
                .and_then(|voice| voice.get("rate"))
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i16::try_from(value).ok())
                .map(clamp_voice_adjustment)
                .unwrap_or_default(),
            pitch: voice
                .and_then(|voice| voice.get("pitch"))
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i16::try_from(value).ok())
                .map(clamp_voice_adjustment)
                .unwrap_or_default(),
            volume: voice
                .and_then(|voice| voice.get("volume"))
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i16::try_from(value).ok())
                .map(clamp_voice_adjustment)
                .unwrap_or_default(),
            auto_speak: voice
                .and_then(|voice| voice.get("auto_speak"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            max_spoken_chars: voice
                .and_then(|voice| voice.get("max_spoken_chars"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .unwrap_or(800),
        }
    }

    fn from_preview(options: &serde_json::Value, prefs: Option<VoicePreferences>) -> Self {
        let mut runtime_prefs = Self::from_options(options);
        if let Some(prefs) = prefs {
            runtime_prefs.voice_id = normalize_voice_config_string(&prefs.voice_id);
            runtime_prefs.rate = clamp_voice_adjustment(prefs.rate);
            runtime_prefs.pitch = clamp_voice_adjustment(prefs.pitch);
            runtime_prefs.volume = clamp_voice_adjustment(prefs.volume);
        }
        runtime_prefs
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TtsProviderKind {
    Edge,
    OpenAi,
    System,
    Stub,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StubbedTtsProvider {
    kind: TtsProviderKind,
    configured_id: String,
}

impl StubbedTtsProvider {
    fn new(provider: &str) -> Self {
        Self {
            kind: tts_provider_kind(provider),
            configured_id: normalize_voice_config_string(provider),
        }
    }
}

impl TtsProvider for StubbedTtsProvider {
    fn id(&self) -> &'static str {
        match self.kind {
            TtsProviderKind::Edge => "edge",
            TtsProviderKind::OpenAi => "openai",
            TtsProviderKind::System => "system",
            TtsProviderKind::Stub => "stub",
            TtsProviderKind::Unsupported => "unsupported",
        }
    }

    fn label(&self) -> &'static str {
        match self.kind {
            TtsProviderKind::Edge => "Edge TTS daemon stub",
            TtsProviderKind::OpenAi => "OpenAI TTS daemon stub",
            TtsProviderKind::System => "System speech daemon stub",
            TtsProviderKind::Stub => "Deterministic test TTS stub",
            TtsProviderKind::Unsupported => "Unsupported TTS provider",
        }
    }

    fn supported_voices(&self) -> Vec<TtsVoice> {
        curated_tts_voices()
    }

    fn speak(&mut self, request: TtsRequest<'_>) -> Result<TtsOutput, TtsError> {
        if self.kind == TtsProviderKind::Unsupported {
            return Err(TtsError::new(
                "unsupported_tts_provider",
                format!("unsupported TTS provider '{}'", self.configured_id),
                false,
            ));
        }
        Ok(TtsOutput {
            provider: self.id().to_owned(),
            voice_id: request.voice_id.to_owned(),
            spoken_chars: request.text.chars().count(),
        })
    }

    fn stop(&mut self) -> Result<(), TtsError> {
        if self.kind == TtsProviderKind::Unsupported {
            return Err(TtsError::new(
                "unsupported_tts_provider",
                format!("unsupported TTS provider '{}'", self.configured_id),
                false,
            ));
        }
        Ok(())
    }
}

fn tts_provider_from_config(provider: &str) -> Box<dyn TtsProvider> {
    Box::new(StubbedTtsProvider::new(provider))
}

fn tts_provider_kind(provider: &str) -> TtsProviderKind {
    match provider {
        "edge" => TtsProviderKind::Edge,
        "openai" => TtsProviderKind::OpenAi,
        "system" => TtsProviderKind::System,
        "stub" => TtsProviderKind::Stub,
        _ => TtsProviderKind::Unsupported,
    }
}

fn curated_tts_voices() -> Vec<TtsVoice> {
    vec![
        TtsVoice {
            id: "id-ID-ArdiNeural",
            label: "Ardi (Indonesian, Male)",
            locale: "id-ID",
            gender: "Male",
        },
        TtsVoice {
            id: "id-ID-GadisNeural",
            label: "Gadis (Indonesian, Female)",
            locale: "id-ID",
            gender: "Female",
        },
        TtsVoice {
            id: "ms-MY-OsmanNeural",
            label: "Osman (Malay, Male)",
            locale: "ms-MY",
            gender: "Male",
        },
        TtsVoice {
            id: "ms-MY-YasminNeural",
            label: "Yasmin (Malay, Female)",
            locale: "ms-MY",
            gender: "Female",
        },
        TtsVoice {
            id: "en-US-AvaNeural",
            label: "Ava (US, Female)",
            locale: "en-US",
            gender: "Female",
        },
        TtsVoice {
            id: "en-US-AndrewNeural",
            label: "Andrew (US, Male)",
            locale: "en-US",
            gender: "Male",
        },
        TtsVoice {
            id: "en-US-EmmaNeural",
            label: "Emma (US, Female)",
            locale: "en-US",
            gender: "Female",
        },
        TtsVoice {
            id: "en-US-BrianNeural",
            label: "Brian (US, Male)",
            locale: "en-US",
            gender: "Male",
        },
        TtsVoice {
            id: "en-GB-SoniaNeural",
            label: "Sonia (GB, Female)",
            locale: "en-GB",
            gender: "Female",
        },
        TtsVoice {
            id: "en-GB-RyanNeural",
            label: "Ryan (GB, Male)",
            locale: "en-GB",
            gender: "Male",
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpeechMode {
    AutoSpeak,
    Preview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SpeechDecision {
    Speak,
    Blocked(&'static str),
    RequiresSummary(&'static str),
}

fn speech_decision(
    prefs: &VoiceRuntimePreferences,
    content_kind: ContentKind,
    text: &str,
    mode: SpeechMode,
) -> SpeechDecision {
    let text = text.trim();
    if text.is_empty() {
        return SpeechDecision::Blocked("empty_text");
    }

    if mode == SpeechMode::AutoSpeak {
        if !prefs.enabled {
            return SpeechDecision::Blocked("voice_disabled");
        }
        if !prefs.auto_speak {
            return SpeechDecision::Blocked("auto_speak_disabled");
        }
    }

    match content_kind {
        ContentKind::Code => return SpeechDecision::Blocked("code_not_speakable"),
        ContentKind::Diff => return SpeechDecision::Blocked("diff_not_speakable"),
        ContentKind::TerminalLog => return SpeechDecision::Blocked("terminal_log_not_speakable"),
        ContentKind::TestResult if text.chars().count() > prefs.max_spoken_chars => {
            return SpeechDecision::Blocked("long_tool_output_not_speakable");
        }
        ContentKind::TestResult if looks_like_raw_tool_output(text) => {
            return SpeechDecision::Blocked("raw_tool_output_not_speakable");
        }
        _ => {}
    }

    if text.chars().count() > prefs.max_spoken_chars {
        return SpeechDecision::RequiresSummary("content_exceeds_max_spoken_chars");
    }

    SpeechDecision::Speak
}

fn looks_like_raw_tool_output(text: &str) -> bool {
    let line_count = text.lines().count();
    line_count > 12
        || text.contains("```")
        || text.contains("diff --git")
        || text.contains("thread '")
        || text.contains("panicked at")
        || text.contains("error[E")
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

    fn builtin() -> Result<Self, RuntimeError> {
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
    description: &'static str,
    risk_class: cadis_protocol::RiskClass,
    input_schema: ToolInputSchema,
    execution: ToolExecutionMode,
    side_effects: &'static [ToolSideEffect],
    timeout_secs: u64,
    timeout_behavior: ToolTimeoutBehavior,
    cancellation_behavior: ToolCancellationBehavior,
    workspace_behavior: ToolWorkspaceBehavior,
    needs_network: bool,
    may_read_secrets: bool,
}

impl ToolDefinition {
    fn safe_read(
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
    fn approval_placeholder(
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

    fn policy_reason(&self) -> String {
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

    fn approval_summary(&self) -> String {
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
enum ToolInputSchema {
    FileRead,
    FileSearch,
    GitStatus,
    GitDiff,
    ShellRun,
    WorkspaceMutation,
    GitMutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolExecutionMode {
    AutoExecute,
    ApprovalPlaceholder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSideEffect {
    ReadFiles,
    SearchFiles,
    ReadGitMetadata,
    EditWorkspace,
    ReadGitDiff,
    CreateWorktree,
    RemoveWorktree,
    RunSubprocess,
}

impl ToolSideEffect {
    fn label(self) -> &'static str {
        match self {
            Self::ReadFiles => "read_files",
            Self::SearchFiles => "search_files",
            Self::ReadGitMetadata => "read_git_metadata",
            Self::EditWorkspace => "edit_workspace",
            Self::ReadGitDiff => "read_git_diff",
            Self::CreateWorktree => "create_worktree",
            Self::RemoveWorktree => "remove_worktree",
            Self::RunSubprocess => "run_subprocess",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolTimeoutBehavior {
    FailClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolCancellationBehavior {
    NotSupported,
    Cooperative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolWorkspaceBehavior {
    PathScoped,
    RequiresWorkspace,
    CadisManagedWorktree,
}

#[derive(Clone, Debug, PartialEq)]
struct ToolExecutionResult {
    summary: String,
    output: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FilePatchOperation {
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
    fn path(&self) -> &str {
        match self {
            Self::Write { path, .. } | Self::Replace { path, .. } => path,
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::Write { .. } => "write",
            Self::Replace { .. } => "replace",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedFilePatch {
    path: PathBuf,
    display_path: String,
    action: &'static str,
    content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ShellRunResult {
    status_success: bool,
    exit_code: Option<i32>,
    stdout: BoundedOutput,
    stderr: BoundedOutput,
    timed_out: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedWorkerWorktree {
    store: ProjectWorkspaceStore,
    metadata: ProjectWorkerWorktreeMetadata,
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

#[derive(Debug)]
struct RuntimeRecovery<K, V> {
    records: HashMap<K, V>,
    diagnostics: Vec<ErrorPayload>,
}

fn recover_session_records(state_store: &StateStore) -> RuntimeRecovery<SessionId, SessionRecord> {
    match state_store.recover_session_metadata::<SessionMetadata>() {
        Ok(recovery) => RuntimeRecovery {
            records: recovery
                .records
                .into_iter()
                .map(|record| record.metadata.into_record())
                .collect(),
            diagnostics: recovery_diagnostics("session", recovery.diagnostics),
        },
        Err(error) => RuntimeRecovery {
            records: HashMap::new(),
            diagnostics: vec![recovery_error(
                "session_recovery_failed",
                error.to_string(),
                true,
            )],
        },
    }
}

fn recover_agent_records(state_store: &StateStore) -> RuntimeRecovery<AgentId, AgentRecord> {
    match state_store.recover_agent_metadata::<AgentMetadata>() {
        Ok(recovery) => RuntimeRecovery {
            records: recovery
                .records
                .into_iter()
                .map(|record| record.metadata.into_record())
                .collect(),
            diagnostics: recovery_diagnostics("agent", recovery.diagnostics),
        },
        Err(error) => RuntimeRecovery {
            records: HashMap::new(),
            diagnostics: vec![recovery_error(
                "agent_recovery_failed",
                error.to_string(),
                true,
            )],
        },
    }
}

fn recover_worker_records(state_store: &StateStore) -> RuntimeRecovery<String, WorkerRecord> {
    match state_store.recover_worker_metadata::<WorkerMetadata>() {
        Ok(recovery) => RuntimeRecovery {
            records: recovery
                .records
                .into_iter()
                .map(|record| record.metadata.into_record())
                .collect(),
            diagnostics: recovery_diagnostics("worker", recovery.diagnostics),
        },
        Err(error) => RuntimeRecovery {
            records: HashMap::new(),
            diagnostics: vec![recovery_error(
                "worker_recovery_failed",
                error.to_string(),
                true,
            )],
        },
    }
}

fn recover_approval_records(
    state_store: &StateStore,
) -> RuntimeRecovery<ApprovalId, ApprovalRecord> {
    match state_store.recover_approval_metadata::<ApprovalRecord>() {
        Ok(recovery) => RuntimeRecovery {
            records: recovery
                .records
                .into_iter()
                .map(|record| (record.metadata.approval_id.clone(), record.metadata))
                .collect(),
            diagnostics: recovery_diagnostics("approval", recovery.diagnostics),
        },
        Err(error) => RuntimeRecovery {
            records: HashMap::new(),
            diagnostics: vec![recovery_error(
                "approval_recovery_failed",
                error.to_string(),
                true,
            )],
        },
    }
}

fn pending_approval_records(
    records: HashMap<ApprovalId, ApprovalRecord>,
) -> HashMap<ApprovalId, PendingApproval> {
    records
        .into_iter()
        .filter(|(_, record)| record.state == ApprovalState::Pending)
        .filter(|(_, record)| !approval_is_expired(record))
        .map(|(approval_id, record)| {
            (
                approval_id,
                PendingApproval {
                    record,
                    request: None,
                },
            )
        })
        .collect()
}

fn reconcile_recovered_workers(
    state_store: &StateStore,
    workers: &mut HashMap<String, WorkerRecord>,
) -> Vec<ErrorPayload> {
    let mut diagnostics = Vec::new();

    for worker in workers.values_mut() {
        if worker.is_terminal() {
            continue;
        }

        worker.status = "failed".to_owned();
        let summary = match worker.summary.take() {
            Some(summary) if !summary.trim().is_empty() => {
                format!("{summary} (marked failed during daemon recovery)")
            }
            _ => "Worker was marked failed during daemon recovery".to_owned(),
        };
        worker.summary = Some(summary.clone());
        worker.error_code = Some("worker_recovered_stale".to_owned());
        worker.error = Some(summary);
        plan_worker_terminal_worktree(worker, "failed");
        worker.updated_at = now_timestamp();

        match state_store.write_worker_metadata(
            &worker.worker_id,
            &WorkerMetadata::from_record(worker),
        ) {
            Ok(()) => diagnostics.push(recovery_error(
                "worker_recovered_stale",
                format!(
                    "worker '{}' had non-terminal status on daemon startup and was marked failed",
                    worker.worker_id
                ),
                false,
            )),
            Err(error) => diagnostics.push(recovery_error(
                "worker_recovery_persist_failed",
                format!(
                    "worker '{}' was marked failed in memory, but recovery state could not be persisted: {error}",
                    worker.worker_id
                ),
                true,
            )),
        }
    }

    diagnostics
}

fn recovery_diagnostics(
    kind: &'static str,
    diagnostics: Vec<StateRecoveryDiagnostic>,
) -> Vec<ErrorPayload> {
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            recovery_error(
                format!("{kind}_metadata_recovery_skipped"),
                format!(
                    "skipped invalid {kind} metadata '{}': {}",
                    diagnostic.path.display(),
                    diagnostic.reason
                ),
                false,
            )
        })
        .collect()
}

fn recovery_error(
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> ErrorPayload {
    ErrorPayload {
        code: code.into(),
        message: message.into(),
        retryable,
    }
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
            diagnostics: vec![recovery_error(
                "agent_session_recovery_failed",
                format!("could not scan durable AgentSession metadata: {error}"),
                true,
            )],
        },
    }
}

fn agent_session_recovery_diagnostic(diagnostic: StateRecoveryDiagnostic) -> ErrorPayload {
    let file_name = diagnostic
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    recovery_error(
        "agent_session_recovery_skipped",
        format!(
            "skipped durable AgentSession metadata state/agent-sessions/{file_name}: {}",
            diagnostic.reason
        ),
        false,
    )
}

fn init_agent_homes(profile_home: &ProfileHome, agents: &HashMap<AgentId, AgentRecord>) {
    for record in agents.values() {
        let _ = profile_home.init_agent(&record.agent_home_template());
    }
}

fn agent_home_diagnostic_check(diagnostic: AgentHomeDiagnostic) -> WorkspaceDoctorCheck {
    WorkspaceDoctorCheck {
        name: diagnostic.name,
        status: diagnostic.status,
        message: diagnostic.message,
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

fn next_worker_counter(workers: &HashMap<String, WorkerRecord>) -> u64 {
    workers
        .keys()
        .filter_map(|worker_id| worker_id.strip_prefix("worker_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn next_approval_counter(approvals: &HashMap<ApprovalId, ApprovalRecord>) -> u64 {
    approvals
        .keys()
        .filter_map(|approval_id| approval_id.as_str().strip_prefix("apr_"))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn worker_status_is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "canceled" | "expired"
    )
}

fn worker_lifecycle_event(payload: WorkerEventPayload) -> CadisEvent {
    match payload.status.as_deref().map(worker_lifecycle_event_kind) {
        Some(WorkerLifecycleEventKind::Completed) => CadisEvent::WorkerCompleted(payload),
        Some(WorkerLifecycleEventKind::Failed) => CadisEvent::WorkerFailed(payload),
        Some(WorkerLifecycleEventKind::Cancelled) => CadisEvent::WorkerCancelled(payload),
        Some(WorkerLifecycleEventKind::Started) | None => CadisEvent::WorkerStarted(payload),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerLifecycleEventKind {
    Started,
    Completed,
    Failed,
    Cancelled,
}

fn worker_lifecycle_event_kind(status: &str) -> WorkerLifecycleEventKind {
    match status {
        "completed" => WorkerLifecycleEventKind::Completed,
        "cancelled" | "canceled" => WorkerLifecycleEventKind::Cancelled,
        status if worker_status_is_terminal(status) => WorkerLifecycleEventKind::Failed,
        _ => WorkerLifecycleEventKind::Started,
    }
}

fn worker_terminal_worktree_state(
    record: &WorkerRecord,
    status: &str,
) -> Option<WorkerWorktreeState> {
    if !worker_status_is_terminal(status) {
        return None;
    }
    let worktree = record.worktree.as_ref()?;
    if worktree.state != WorkerWorktreeState::Active {
        return None;
    }
    if worker_lifecycle_event_kind(status) == WorkerLifecycleEventKind::Cancelled {
        return Some(WorkerWorktreeState::CleanupPending);
    }

    Some(match worktree.cleanup_policy {
        WorkerWorktreeCleanupPolicy::OnCompletion => WorkerWorktreeState::CleanupPending,
        WorkerWorktreeCleanupPolicy::Explicit | WorkerWorktreeCleanupPolicy::AfterApply => {
            WorkerWorktreeState::ReviewPending
        }
    })
}

fn plan_worker_terminal_worktree(record: &mut WorkerRecord, status: &str) {
    let Some(state) = worker_terminal_worktree_state(record, status) else {
        return;
    };
    if let Some(worktree) = &mut record.worktree {
        worktree.state = state;
    }
}

fn project_worker_worktree_state_for_worker_state(
    state: WorkerWorktreeState,
) -> ProjectWorkerWorktreeState {
    match state {
        WorkerWorktreeState::Planned => ProjectWorkerWorktreeState::Planned,
        WorkerWorktreeState::Active => ProjectWorkerWorktreeState::Ready,
        WorkerWorktreeState::ReviewPending => ProjectWorkerWorktreeState::ReviewPending,
        WorkerWorktreeState::CleanupPending => ProjectWorkerWorktreeState::CleanupPending,
        WorkerWorktreeState::Removed => ProjectWorkerWorktreeState::Removed,
    }
}

fn resolve_project_path(project_root: &Path, path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
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

fn approval_request_payload(record: &ApprovalRecord) -> ApprovalRequestPayload {
    ApprovalRequestPayload {
        approval_id: record.approval_id.clone(),
        session_id: record.session_id.clone(),
        tool_call_id: record.tool_call_id.clone(),
        risk_class: record.risk_class,
        title: record.title.clone(),
        summary: record.summary.clone(),
        command: record.command.clone(),
        workspace: record.workspace.clone(),
        expires_at: record.expires_at.clone(),
    }
}

fn input_string(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn input_raw_string(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn input_usize(input: &serde_json::Value, key: &str) -> Option<usize> {
    input
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn input_u64(input: &serde_json::Value, key: &str) -> Option<u64> {
    input.get(key).and_then(serde_json::Value::as_u64)
}

fn shell_timeout(
    input: &serde_json::Value,
    tool_timeout_secs: u64,
) -> Result<StdDuration, ErrorPayload> {
    let max_ms = tool_timeout_secs.saturating_mul(1_000);
    let requested_ms = input_u64(input, "timeout_ms")
        .or_else(|| input_u64(input, "timeout_secs").map(|seconds| seconds.saturating_mul(1_000)))
        .unwrap_or(max_ms);

    if requested_ms == 0 {
        return Err(tool_error(
            "invalid_tool_input",
            "shell.run timeout must be positive",
            false,
        ));
    }

    Ok(StdDuration::from_millis(requested_ms.min(max_ms).max(1)))
}

fn duration_millis(duration: StdDuration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn shell_summary(
    stdout: &str,
    stderr: &str,
    stdout_truncated: bool,
    stderr_truncated: bool,
) -> String {
    let mut summary = String::new();
    if !stdout.is_empty() {
        summary.push_str(stdout);
        if stdout_truncated {
            if !summary.ends_with('\n') {
                summary.push('\n');
            }
            summary.push_str("[stdout truncated]");
        }
    }
    if !stderr.is_empty() {
        if !summary.is_empty() && !summary.ends_with('\n') {
            summary.push('\n');
        }
        summary.push_str(stderr);
        if stderr_truncated {
            if !summary.ends_with('\n') {
                summary.push('\n');
            }
            summary.push_str("[stderr truncated]");
        }
    }
    if summary.is_empty() {
        "command completed with no output".to_owned()
    } else {
        summary
    }
}

fn normalize_voice_checks(checks: Vec<VoiceDoctorCheck>) -> Vec<VoiceDoctorCheck> {
    checks
        .into_iter()
        .filter_map(|check| {
            let name = check.name.trim();
            if name.is_empty() {
                return None;
            }
            Some(VoiceDoctorCheck {
                name: redact(name),
                status: normalize_voice_check_status(&check.status),
                message: redact(check.message.trim()),
            })
        })
        .collect()
}

fn normalize_voice_check_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "ok" | "pass" | "passed" | "ready" => "ok",
        "warn" | "warning" | "degraded" | "unknown" => "warn",
        "error" | "fail" | "failed" | "blocked" => "error",
        _ => "warn",
    }
    .to_owned()
}

fn voice_check_summary_status(checks: &[VoiceDoctorCheck]) -> String {
    if checks.iter().any(|check| check.status == "error") {
        "error".to_owned()
    } else if checks.is_empty() || checks.iter().any(|check| check.status == "warn") {
        "warn".to_owned()
    } else {
        "ok".to_owned()
    }
}

fn voice_checks_summary(checks: &[VoiceDoctorCheck]) -> String {
    let errors = checks
        .iter()
        .filter(|check| check.status == "error")
        .count();
    let warnings = checks.iter().filter(|check| check.status == "warn").count();
    if errors > 0 {
        format!("{errors} blocking voice issue{}", plural(errors))
    } else if warnings > 0 {
        format!("{warnings} voice warning{}", plural(warnings))
    } else if checks.is_empty() {
        "no bridge checks reported".to_owned()
    } else {
        "voice bridge ready".to_owned()
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn voice_runtime_state(checks: &[VoiceDoctorCheck]) -> VoiceRuntimeState {
    if checks.iter().any(|check| check.status == "error") {
        VoiceRuntimeState::Blocked
    } else if checks.iter().any(|check| check.status == "warn") {
        VoiceRuntimeState::Degraded
    } else {
        VoiceRuntimeState::Ready
    }
}

fn normalize_voice_config_string(value: &str) -> String {
    value.trim().to_owned()
}

fn is_supported_voice_provider(provider: &str) -> bool {
    matches!(provider, "edge" | "openai" | "system" | "stub")
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

fn validate_shell_cwd(cwd: &Path, cadis_home: &Path) -> Result<(), ErrorPayload> {
    if cwd.parent().is_none() {
        return Err(tool_error(
            "shell_cwd_denied",
            "shell.run cwd cannot be the filesystem root",
            false,
        ));
    }

    for denied in ["/etc", "/dev", "/proc", "/sys", "/run"] {
        let denied = Path::new(denied);
        if cwd == denied || cwd.starts_with(denied) {
            return Err(tool_error(
                "shell_cwd_denied",
                format!("shell.run cwd {} is a protected system path", cwd.display()),
                false,
            ));
        }
    }

    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok())
    {
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

fn project_workspace_metadata_checks(workspace: &WorkspaceRecord) -> Vec<WorkspaceDoctorCheck> {
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

fn project_workspace_metadata_checks_for_root(root: &Path) -> Vec<WorkspaceDoctorCheck> {
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

fn project_worker_worktree_checks_for_root(root: &Path) -> Vec<WorkspaceDoctorCheck> {
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

fn project_worktree_diagnostic_check(
    diagnostic: ProjectWorktreeDiagnostic,
) -> WorkspaceDoctorCheck {
    WorkspaceDoctorCheck {
        name: diagnostic.name,
        status: diagnostic.status,
        message: diagnostic.message,
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
        "file.patch" => file_patch_path_summary(input),
        _ => input_string(input, "path"),
    }
    .map(|value| redact(&value))
}

fn file_patch_path_summary(input: &serde_json::Value) -> Option<String> {
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

fn parse_file_patch_operations(
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

fn parse_file_patch_operation(
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

fn parse_file_patch_replace(
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

fn validate_patch_text_size(label: &str, value: &str) -> Result<(), ErrorPayload> {
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

fn validate_file_patch_input(
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

fn validate_file_patch_target(
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

fn prepare_file_patch(
    workspace: &Path,
    operations: &[FilePatchOperation],
) -> Result<Vec<PreparedFilePatch>, ErrorPayload> {
    let mut staged = HashMap::<PathBuf, String>::new();
    let mut prepared = Vec::new();

    for operation in operations {
        let path = resolve_file_patch_target(workspace, operation.path())?;
        validate_file_patch_target(operation, &path)?;
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
        });
    }

    Ok(prepared)
}

fn read_patch_target(path: &Path) -> Result<String, ErrorPayload> {
    fs::read_to_string(path).map_err(|error| {
        tool_error(
            "file_patch_read_failed",
            format!("could not read patch target: {error}"),
            false,
        )
    })
}

fn replace_once(content: &str, old: &str, new: &str) -> Result<String, ErrorPayload> {
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

fn resolve_file_patch_target(workspace: &Path, user_path: &str) -> Result<PathBuf, ErrorPayload> {
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

fn validate_file_patch_relative_path(path: &Path) -> Result<(), ErrorPayload> {
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

fn path_has_parent_or_root(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn file_patch_path_is_protected(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => matches!(value.to_str(), Some(".git" | ".cadis")),
        _ => false,
    })
}

fn file_patch_path_is_secret_like(path: &Path) -> bool {
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

fn validate_git_pathspec(pathspec: &str) -> Result<String, ErrorPayload> {
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
    workspace_id: Option<&str>,
    task: &str,
) -> WorkerWorktreeIntent {
    let worktree_root = workspace
        .map(|workspace| {
            let store = ProjectWorkspaceStore::new(workspace);
            let metadata = store.load().ok().flatten();
            store
                .worker_worktree_paths(worker_id, metadata.as_ref())
                .worktree_root
                .display()
                .to_string()
        })
        .unwrap_or_else(|| ".cadis/worktrees".to_owned());
    let worktree_path = Path::new(&worktree_root)
        .join(worker_id)
        .display()
        .to_string();

    WorkerWorktreeIntent {
        workspace_id: workspace_id.map(ToOwned::to_owned),
        project_root: workspace.map(ToOwned::to_owned),
        worktree_root,
        worktree_path,
        branch_name: format!("cadis/{worker_id}/{}", branch_slug(task)),
        base_ref: Some("HEAD".to_owned()),
        state: WorkerWorktreeState::Planned,
        cleanup_policy: WorkerWorktreeCleanupPolicy::Explicit,
    }
}

fn worker_artifact_locations(paths: &WorkerArtifactPathSet) -> WorkerArtifactLocations {
    WorkerArtifactLocations {
        root: paths.root.display().to_string(),
        patch: paths.patch.display().to_string(),
        test_report: paths.test_report.display().to_string(),
        summary: paths.summary.display().to_string(),
        changed_files: paths.changed_files.display().to_string(),
        memory_candidates: paths.memory_candidates.display().to_string(),
    }
}

fn prepare_worker_execution(record: &mut WorkerRecord) -> Vec<String> {
    let mut logs = Vec::new();
    if let Some(artifacts) = &record.artifacts {
        if let Err(error) = fs::create_dir_all(&artifacts.root) {
            logs.push(format!("artifact layout failed: {error}\n"));
        }
    }

    if let Some(worktree) = &mut record.worktree {
        logs.extend(prepare_worker_worktree(
            &record.worker_id,
            worktree,
            record.artifacts.as_ref(),
        ));
    }

    logs.into_iter().map(|line| redact(&line)).collect()
}

fn prepare_worker_worktree(
    worker_id: &str,
    worktree: &mut WorkerWorktreeIntent,
    artifacts: Option<&WorkerArtifactLocations>,
) -> Vec<String> {
    let mut logs = Vec::new();
    let Some(project_root) = worktree.project_root.clone() else {
        return logs;
    };
    let project_root = PathBuf::from(project_root);
    let store = ProjectWorkspaceStore::new(&project_root);
    let metadata = store.load().ok().flatten();
    let paths = store.worker_worktree_paths(worker_id, metadata.as_ref());
    let artifact_root = artifacts
        .map(|artifacts| PathBuf::from(&artifacts.root))
        .unwrap_or_else(|| PathBuf::from(format!("artifacts/workers/{worker_id}")));

    if let Err(error) = store.ensure_layout() {
        logs.push(format!("worktree layout failed: {error}\n"));
        return logs;
    }

    if !is_git_work_tree(&project_root) {
        let _ = store.save_worker_worktree_metadata(&project_worker_metadata(
            worker_id,
            worktree,
            &paths.worktree_path,
            &artifact_root,
            ProjectWorkerWorktreeState::Planned,
        ));
        logs.push("worktree skipped: session workspace is not a git worktree\n".to_owned());
        return logs;
    }

    if let Some(parent) = paths.worktree_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            logs.push(format!("worktree parent creation failed: {error}\n"));
            return logs;
        }
    }

    if paths.worktree_path.exists() {
        worktree.state = WorkerWorktreeState::Active;
        worktree.worktree_path = paths.worktree_path.display().to_string();
        let _ = store.save_worker_worktree_metadata(&project_worker_metadata(
            worker_id,
            worktree,
            &paths.worktree_path,
            &artifact_root,
            ProjectWorkerWorktreeState::Ready,
        ));
        logs.push(format!(
            "worktree ready: {}\n",
            paths.worktree_path.display()
        ));
        return logs;
    }

    let base_ref = worktree.base_ref.as_deref().unwrap_or("HEAD");
    match create_git_worktree(
        &project_root,
        &paths.worktree_path,
        &worktree.branch_name,
        base_ref,
    ) {
        Ok(()) => {
            worktree.state = WorkerWorktreeState::Active;
            worktree.worktree_path = paths.worktree_path.display().to_string();
            let _ = store.save_worker_worktree_metadata(&project_worker_metadata(
                worker_id,
                worktree,
                &paths.worktree_path,
                &artifact_root,
                ProjectWorkerWorktreeState::Ready,
            ));
            logs.push(format!(
                "worktree ready: {} ({})\n",
                paths.worktree_path.display(),
                worktree.branch_name
            ));
        }
        Err(error) => {
            let _ = store.save_worker_worktree_metadata(&project_worker_metadata(
                worker_id,
                worktree,
                &paths.worktree_path,
                &artifact_root,
                ProjectWorkerWorktreeState::Planned,
            ));
            logs.push(format!("worktree creation failed: {error}\n"));
        }
    }

    logs
}

fn project_worker_metadata(
    worker_id: &str,
    worktree: &WorkerWorktreeIntent,
    worktree_path: &Path,
    artifact_root: &Path,
    state: ProjectWorkerWorktreeState,
) -> ProjectWorkerWorktreeMetadata {
    ProjectWorkerWorktreeMetadata {
        worker_id: worker_id.to_owned(),
        workspace_id: worktree
            .workspace_id
            .clone()
            .unwrap_or_else(|| "session".to_owned()),
        worktree_path: worktree_path.to_path_buf(),
        branch_name: worktree.branch_name.clone(),
        base_ref: worktree.base_ref.clone(),
        state,
        artifact_root: artifact_root.to_path_buf(),
    }
}

fn is_git_work_tree(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn create_git_worktree(
    project_root: &Path,
    worktree_path: &Path,
    branch_name: &str,
    base_ref: &str,
) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["worktree", "add", "-b"])
        .arg(branch_name)
        .arg(worktree_path)
        .arg(base_ref)
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = redact(&String::from_utf8_lossy(&output.stderr));
        Err(if stderr.trim().is_empty() {
            "git worktree add failed".to_owned()
        } else {
            stderr.trim().to_owned()
        })
    }
}

fn execute_worker_command(record: &mut WorkerRecord) -> WorkerCommandExecution {
    let Some(worktree) = record.worktree.clone() else {
        return WorkerCommandExecution::default();
    };
    if worktree.state != WorkerWorktreeState::Active {
        return WorkerCommandExecution::default();
    }

    let mut execution = WorkerCommandExecution::default();
    let cwd = match worker_command_cwd(&record.worker_id, &worktree) {
        Ok(cwd) => cwd,
        Err(message) => {
            let message = redact(&message);
            record.command_report = Some(WorkerCommandReport {
                command: WORKER_DEFAULT_COMMAND.to_owned(),
                cwd: worktree.worktree_path,
                status: "failed".to_owned(),
                exit_code: None,
                timed_out: false,
                stdout: String::new(),
                stderr: message.clone(),
                stdout_truncated: false,
                stderr_truncated: false,
                timeout_ms: WORKER_COMMAND_TIMEOUT_MS,
            });
            execution.failure = Some(WorkerCommandFailure {
                code: "worker_command_refused".to_owned(),
                message,
            });
            return execution;
        }
    };

    let cwd_display = cwd.display().to_string();
    record.cli = Some("cadisd-worker-command".to_owned());
    record.cwd = Some(cwd_display.clone());
    execution
        .logs
        .push(format!("command started: {WORKER_DEFAULT_COMMAND}\n"));

    let report = match run_worker_validation_command(&cwd) {
        Ok(result) => worker_command_report(&cwd_display, result),
        Err(error) => WorkerCommandReport {
            command: WORKER_DEFAULT_COMMAND.to_owned(),
            cwd: cwd_display,
            status: "failed".to_owned(),
            exit_code: None,
            timed_out: false,
            stdout: String::new(),
            stderr: redact(&error.message),
            stdout_truncated: false,
            stderr_truncated: false,
            timeout_ms: WORKER_COMMAND_TIMEOUT_MS,
        },
    };

    execution.logs.extend(worker_command_logs(&report));
    if report.status != "passed" {
        execution.failure = Some(worker_command_failure(&report));
    }
    record.command_report = Some(report);
    execution
}

fn worker_command_cwd(worker_id: &str, worktree: &WorkerWorktreeIntent) -> Result<PathBuf, String> {
    let cwd = PathBuf::from(&worktree.worktree_path)
        .canonicalize()
        .map_err(|error| format!("worker command cwd is unavailable: {error}"))?;
    if !cwd.is_dir() {
        return Err(format!(
            "worker command cwd {} is not a directory",
            cwd.display()
        ));
    }
    if cwd.file_name().and_then(|value| value.to_str()) != Some(worker_id) {
        return Err("worker command refused: cwd is not the assigned worker directory".to_owned());
    }

    let worktree_root = PathBuf::from(&worktree.worktree_root)
        .canonicalize()
        .map_err(|error| format!("worker command root is unavailable: {error}"))?;
    if !worktree_root.ends_with(Path::new(".cadis/worktrees")) {
        return Err(
            "worker command refused: worktree root must be project .cadis/worktrees".to_owned(),
        );
    }
    if cwd.parent() != Some(worktree_root.as_path()) {
        return Err("worker command refused: cwd is outside the worker worktree root".to_owned());
    }

    Ok(cwd)
}

fn run_worker_validation_command(cwd: &Path) -> Result<ShellRunResult, ErrorPayload> {
    let mut command = Command::new("git");
    command
        .args(["status", "--short"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    run_bounded_command(command, StdDuration::from_millis(WORKER_COMMAND_TIMEOUT_MS))
}

fn worker_command_report(cwd: &str, result: ShellRunResult) -> WorkerCommandReport {
    let stdout = redact(&String::from_utf8_lossy(&result.stdout.bytes));
    let stderr = redact(&String::from_utf8_lossy(&result.stderr.bytes));
    let status = if result.timed_out {
        "timed_out"
    } else if result.status_success {
        "passed"
    } else {
        "failed"
    };

    WorkerCommandReport {
        command: WORKER_DEFAULT_COMMAND.to_owned(),
        cwd: cwd.to_owned(),
        status: status.to_owned(),
        exit_code: result.exit_code,
        timed_out: result.timed_out,
        stdout,
        stderr,
        stdout_truncated: result.stdout.truncated,
        stderr_truncated: result.stderr.truncated,
        timeout_ms: WORKER_COMMAND_TIMEOUT_MS,
    }
}

fn worker_command_failure(report: &WorkerCommandReport) -> WorkerCommandFailure {
    if report.timed_out {
        return WorkerCommandFailure {
            code: "worker_command_timeout".to_owned(),
            message: format!(
                "worker command timed out after timeout_ms={}: {}",
                report.timeout_ms, report.command
            ),
        };
    }

    let detail = if !report.stderr.trim().is_empty() {
        report.stderr.trim()
    } else if !report.stdout.trim().is_empty() {
        report.stdout.trim()
    } else {
        "command exited without output"
    };
    WorkerCommandFailure {
        code: "worker_command_failed".to_owned(),
        message: format!(
            "worker command exited with code {:?}: {}",
            report.exit_code,
            truncate_redacted_text(detail, WORKER_COMMAND_SUMMARY_LIMIT_BYTES)
        ),
    }
}

fn worker_command_logs(report: &WorkerCommandReport) -> Vec<String> {
    let mut logs = Vec::new();
    if !report.stdout.is_empty() || report.stdout_truncated {
        logs.push(bounded_worker_command_log(
            "stdout",
            &report.stdout,
            report.stdout_truncated,
        ));
    }
    if !report.stderr.is_empty() || report.stderr_truncated {
        logs.push(bounded_worker_command_log(
            "stderr",
            &report.stderr,
            report.stderr_truncated,
        ));
    }
    logs.push(format!(
        "command finished: status={} exit_code={:?}\n",
        report.status, report.exit_code
    ));
    logs
}

fn bounded_worker_command_log(label: &str, content: &str, source_truncated: bool) -> String {
    let header = format!("command {label}:\n");
    let marker = format!("\n[{label} truncated]\n");
    let redacted = redact(content);
    let limit = WORKER_COMMAND_LOG_LIMIT_BYTES
        .saturating_sub(header.len())
        .saturating_sub(marker.len())
        .max(1);
    let (content, locally_truncated) = truncate_to_utf8_boundary(&redacted, limit);
    let mut log = header;
    log.push_str(content);
    if source_truncated || locally_truncated {
        log.push_str(&marker);
    } else if !log.ends_with('\n') {
        log.push('\n');
    }
    log
}

fn truncate_redacted_text(content: &str, limit: usize) -> String {
    let redacted = redact(content);
    let (content, truncated) = truncate_to_utf8_boundary(&redacted, limit);
    if truncated {
        format!("{content}...")
    } else {
        content.to_owned()
    }
}

fn truncate_to_utf8_boundary(content: &str, limit: usize) -> (&str, bool) {
    if content.len() <= limit {
        return (content, false);
    }

    let mut end = limit;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    (&content[..end], true)
}

fn worker_command_summary_markdown(report: &WorkerCommandReport) -> String {
    let mut content = format!(
        "\n## Daemon Validation\n\nCommand: `{}`\n\nStatus: {}\n\nExit code: {:?}\n\n",
        redact(&report.command),
        report.status,
        report.exit_code
    );
    if !report.stdout.is_empty() || report.stdout_truncated {
        content.push_str("Stdout:\n\n```text\n");
        content.push_str(&report.stdout);
        if report.stdout_truncated {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("[stdout truncated]\n");
        }
        content.push_str("```\n\n");
    }
    if !report.stderr.is_empty() || report.stderr_truncated {
        content.push_str("Stderr:\n\n```text\n");
        content.push_str(&report.stderr);
        if report.stderr_truncated {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("[stderr truncated]\n");
        }
        content.push_str("```\n\n");
    }
    content
}

fn worker_command_report_json(report: &WorkerCommandReport) -> serde_json::Value {
    serde_json::json!({
        "command": report.command.clone(),
        "cwd": report.cwd.clone(),
        "status": report.status.clone(),
        "exit_code": report.exit_code,
        "timed_out": report.timed_out,
        "stdout": report.stdout.clone(),
        "stderr": report.stderr.clone(),
        "stdout_truncated": report.stdout_truncated,
        "stderr_truncated": report.stderr_truncated,
        "timeout_ms": report.timeout_ms,
    })
}

fn write_worker_artifacts(record: &mut WorkerRecord, status: &str, summary: &str) -> Vec<String> {
    let mut logs = Vec::new();
    let Some(artifacts) = record.artifacts.clone() else {
        return logs;
    };
    if let Err(error) = fs::create_dir_all(&artifacts.root) {
        logs.push(format!("artifact write failed: {error}\n"));
        return logs;
    }

    let mut summary_content = format!(
        "# Worker {}\n\nStatus: {}\n\n{}\n",
        record.worker_id,
        status,
        redact(summary)
    );
    if let Some(command_report) = &record.command_report {
        summary_content.push_str(&worker_command_summary_markdown(command_report));
    }
    if let Err(error) = write_artifact(&artifacts.summary, &summary_content) {
        logs.push(format!("summary artifact failed: {error}\n"));
    }

    let worktree_path = record
        .worktree
        .as_ref()
        .filter(|worktree| worktree.state == WorkerWorktreeState::Active)
        .map(|worktree| PathBuf::from(&worktree.worktree_path));

    let patch = worktree_path
        .as_deref()
        .and_then(|path| git_stdout(path, ["diff", "--binary", "HEAD"]).ok())
        .unwrap_or_default();
    if let Err(error) = write_artifact(&artifacts.patch, &patch) {
        logs.push(format!("patch artifact failed: {error}\n"));
    }

    let status_lines = worktree_path
        .as_deref()
        .and_then(|path| git_stdout(path, ["status", "--short"]).ok())
        .unwrap_or_default()
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let changed_files = serde_json::json!({
        "worker_id": record.worker_id.clone(),
        "status": status,
        "files": status_lines,
    });
    if let Err(error) = write_json_artifact(&artifacts.changed_files, &changed_files) {
        logs.push(format!("changed-files artifact failed: {error}\n"));
    }

    let test_report = serde_json::json!({
        "worker_id": record.worker_id.clone(),
        "status": status,
        "summary": redact(summary),
        "generated_by": "cadisd",
        "generated_at": now_timestamp(),
        "validation_command": record.command_report.as_ref().map(worker_command_report_json),
    });
    if let Err(error) = write_json_artifact(&artifacts.test_report, &test_report) {
        logs.push(format!("test-report artifact failed: {error}\n"));
    }

    if let Err(error) = write_artifact(&artifacts.memory_candidates, "") {
        logs.push(format!("memory-candidates artifact failed: {error}\n"));
    }

    logs.into_iter().map(|line| redact(&line)).collect()
}

fn write_artifact(path: &str, content: &str) -> io::Result<()> {
    let path = Path::new(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, redact(content))
}

fn write_json_artifact(path: &str, value: &serde_json::Value) -> io::Result<()> {
    let mut content = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned());
    content.push('\n');
    write_artifact(path, &content)
}

fn git_stdout<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(redact(&String::from_utf8_lossy(&output.stdout)))
    } else {
        Err(redact(&String::from_utf8_lossy(&output.stderr)))
    }
}

fn run_shell_command(
    cwd: &Path,
    command: &str,
    timeout: StdDuration,
) -> Result<ShellRunResult, ErrorPayload> {
    let mut child_command = Command::new("sh");
    child_command
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    child_command.process_group(0);

    run_bounded_command(child_command, timeout)
}

fn run_bounded_command(
    mut child_command: Command,
    timeout: StdDuration,
) -> Result<ShellRunResult, ErrorPayload> {
    let mut child = child_command
        .spawn()
        .map_err(|error| tool_error("shell_spawn_failed", error.to_string(), false))?;

    let stdout = child
        .stdout
        .take()
        .map(|stdout| thread::spawn(move || read_bounded_output(stdout, SHELL_OUTPUT_LIMIT_BYTES)));
    let stderr = child
        .stderr
        .take()
        .map(|stderr| thread::spawn(move || read_bounded_output(stderr, SHELL_OUTPUT_LIMIT_BYTES)));

    let started_at = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started_at.elapsed() >= timeout => {
                timed_out = true;
                terminate_child(&mut child);
                break child
                    .wait()
                    .map_err(|error| tool_error("shell_wait_failed", error.to_string(), false))?;
            }
            Ok(None) => thread::sleep(StdDuration::from_millis(SHELL_POLL_INTERVAL_MS)),
            Err(error) => return Err(tool_error("shell_wait_failed", error.to_string(), false)),
        }
    };

    Ok(ShellRunResult {
        status_success: !timed_out && status.success(),
        exit_code: status.code(),
        stdout: join_bounded_output(stdout),
        stderr: join_bounded_output(stderr),
        timed_out,
    })
}

fn read_bounded_output(mut reader: impl Read, limit: usize) -> BoundedOutput {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let remaining = limit.saturating_sub(bytes.len());
                if remaining > 0 {
                    let visible = read.min(remaining);
                    bytes.extend_from_slice(&buffer[..visible]);
                    if visible < read {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => {
                truncated = true;
                break;
            }
        }
    }

    BoundedOutput { bytes, truncated }
}

fn join_bounded_output(handle: Option<thread::JoinHandle<BoundedOutput>>) -> BoundedOutput {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or(BoundedOutput {
            bytes: Vec::new(),
            truncated: false,
        })
}

fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(&process_group)
            .status();
        thread::sleep(StdDuration::from_millis(20));
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(&process_group)
            .status();
    }

    let _ = child.kill();
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

fn clamp_voice_adjustment(value: i16) -> i16 {
    value.clamp(-50, 50)
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
        MessageSendRequest, RequestId, ServerFrame, SessionCreateRequest,
        SessionSubscriptionRequest, SessionTargetRequest, ToolCallRequest, VoiceDoctorCheck,
        VoiceDoctorRequest, VoicePreflightRequest, WorkerTailRequest, WorkspaceAccess,
        WorkspaceDoctorRequest, WorkspaceGrantRequest, WorkspaceId, WorkspaceKind,
        WorkspaceRegisterRequest, WorkspaceRevokeRequest,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
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

    fn runtime_with_voice(enabled: bool, auto_speak: bool, max_spoken_chars: usize) -> Runtime {
        Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cadis-home"),
                profile_id: "default".to_owned(),
                socket_path: Some(PathBuf::from("/tmp/cadis-test.sock")),
                model_provider: "echo".to_owned(),
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
                ui_preferences: serde_json::json!({
                    "voice": {
                        "enabled": enabled,
                        "provider": "stub",
                        "voice_id": "id-ID-GadisNeural",
                        "stt_language": "auto",
                        "rate": 0,
                        "pitch": 0,
                        "volume": 0,
                        "auto_speak": auto_speak,
                        "max_spoken_chars": max_spoken_chars
                    },
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
            Box::<EchoProvider>::default(),
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
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
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
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
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

    #[derive(Clone, Debug)]
    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    impl ModelProvider for CountingProvider {
        fn name(&self) -> &str {
            "counting"
        }

        fn chat(&self, _prompt: &str) -> Result<Vec<String>, cadis_models::ModelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec!["counted".to_owned()])
        }
    }

    fn runtime_with_agent_runtime_config(agent_runtime: AgentRuntimeConfig) -> Runtime {
        Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cadis-home"),
                profile_id: "default".to_owned(),
                socket_path: Some(PathBuf::from("/tmp/cadis-test.sock")),
                model_provider: "echo".to_owned(),
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
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

    fn init_git_workspace(root: &Path) {
        run_git(root, &["init"]);
        fs::write(root.join("README.md"), "CADIS worker fixture\n")
            .expect("fixture file should write");
        run_git(root, &["add", "README.md"]);
        run_git(
            root,
            &[
                "-c",
                "user.name=CADIS Test",
                "-c",
                "user.email=cadis@example.invalid",
                "commit",
                "-m",
                "initial",
            ],
        );
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn create_workspace_session(runtime: &mut Runtime, workspace: &Path, title: &str) -> SessionId {
        runtime
            .handle_request(RequestEnvelope::new(
                RequestId::from(format!("req_session_{}", slugify(title))),
                ClientId::from("cli_1"),
                ClientRequest::SessionCreate(SessionCreateRequest {
                    title: Some(title.to_owned()),
                    cwd: Some(workspace.display().to_string()),
                }),
            ))
            .events
            .into_iter()
            .find_map(|event| match event.event {
                CadisEvent::SessionStarted(payload) => Some(payload.session_id),
                _ => None,
            })
            .expect("session.started should be emitted")
    }

    fn begin_workspace_worker_message(
        runtime: &mut Runtime,
        workspace: &Path,
        request_id: &str,
        content: &str,
    ) -> PendingMessageGeneration {
        let session_id = create_workspace_session(runtime, workspace, request_id);
        match runtime.begin_message_request(RequestEnvelope::new(
            RequestId::from(request_id),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: Some(session_id),
                target_agent_id: None,
                content: content.to_owned(),
                content_kind: ContentKind::Chat,
            }),
        )) {
            Ok(pending) => pending,
            Err(outcome) => panic!("worker message should begin: {:?}", outcome.response),
        }
    }

    fn test_model_response() -> cadis_models::ModelResponse {
        cadis_models::ModelResponse {
            deltas: Vec::new(),
            invocation: cadis_models::ModelInvocation {
                requested_model: None,
                effective_provider: "echo".to_owned(),
                effective_model: "echo".to_owned(),
                fallback: false,
                fallback_reason: None,
            },
        }
    }

    fn worker_started_payload(events: &[EventEnvelope]) -> WorkerEventPayload {
        events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerStarted(payload) => Some(payload.clone()),
                _ => None,
            })
            .expect("worker.started should be emitted")
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

    fn complete_worker_in_workspace(
        runtime: &mut Runtime,
        workspace: &Path,
        title: &str,
    ) -> (SessionId, String, String) {
        let session_id = runtime
            .handle_request(RequestEnvelope::new(
                RequestId::from(format!("req_worker_session_{title}")),
                ClientId::from("cli_1"),
                ClientRequest::SessionCreate(SessionCreateRequest {
                    title: Some(title.to_owned()),
                    cwd: Some(workspace.display().to_string()),
                }),
            ))
            .events
            .into_iter()
            .find_map(|event| match event.event {
                CadisEvent::SessionStarted(payload) => Some(payload.session_id),
                _ => None,
            })
            .expect("session.started should be emitted");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from(format!("req_worker_execution_{title}")),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: Some(session_id.clone()),
                target_agent_id: None,
                content: "/route @codex run focused tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let completed = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("worker.completed should be emitted");
        let worktree_path = completed
            .worktree
            .as_ref()
            .expect("worker.completed should include worktree metadata")
            .worktree_path
            .clone();

        (session_id, completed.worker_id.clone(), worktree_path)
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

    fn approval_id_from(outcome: &RequestOutcome) -> ApprovalId {
        outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::ApprovalRequested(payload) => Some(payload.approval_id.clone()),
                _ => None,
            })
            .expect("approval.requested should be emitted")
    }

    fn approve(runtime: &mut Runtime, approval_id: ApprovalId) -> RequestOutcome {
        runtime.handle_request(RequestEnvelope::new(
            RequestId::from(format!("req_approve_{approval_id}")),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id,
                decision: ApprovalDecision::Approved,
                reason: Some("approved by test".to_owned()),
            }),
        ))
    }

    fn send_message_with_kind(
        runtime: &mut Runtime,
        content_kind: ContentKind,
        content: &str,
    ) -> RequestOutcome {
        runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_message"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: content.to_owned(),
                content_kind,
            }),
        ))
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
                assert_eq!(status.voice.provider, "edge");
                assert_eq!(status.voice.state, VoiceRuntimeState::Disabled);
            }
            other => panic!("unexpected response: {other:?}"),
        }
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn voice_doctor_reports_missing_local_bridge_preflight() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_voice_doctor"),
            ClientId::from("cli_1"),
            ClientRequest::VoiceDoctor(VoiceDoctorRequest::default()),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let doctor = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::VoiceDoctorResponse(payload) => Some(payload),
                _ => None,
            })
            .expect("voice doctor event should be emitted");
        assert!(doctor.checks.iter().any(|check| {
            check.name == "voice.preflight"
                && check.status == "warn"
                && check.message.contains("no local bridge preflight")
        }));
    }

    #[test]
    fn voice_preflight_promotes_hud_checks_into_status() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_voice_preflight"),
            ClientId::from("hud_1"),
            ClientRequest::VoicePreflight(VoicePreflightRequest {
                surface: Some("cadis-hud".to_owned()),
                summary: Some("ready".to_owned()),
                checks: vec![VoiceDoctorCheck {
                    name: "microphone".to_owned(),
                    status: "pass".to_owned(),
                    message: "1 input visible".to_owned(),
                }],
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let status = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::VoiceStatusUpdated(payload) => Some(payload),
                _ => None,
            })
            .expect("voice status event should be emitted");
        assert_eq!(status.last_preflight.as_ref().unwrap().surface, "cadis-hud");
        assert_eq!(status.last_preflight.as_ref().unwrap().status, "ok");

        let doctor = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::VoicePreflightResponse(payload) => Some(payload),
                _ => None,
            })
            .expect("voice preflight response should be emitted");
        assert!(doctor.checks.iter().any(|check| {
            check.name == "microphone" && check.status == "ok" && check.message == "1 input visible"
        }));
    }

    #[test]
    fn voice_provider_stubs_report_curated_catalog_without_external_calls() {
        for provider_id in ["edge", "openai", "system", "stub"] {
            let provider = tts_provider_from_config(provider_id);
            assert_eq!(provider.id(), provider_id);
            assert!(provider
                .supported_voices()
                .iter()
                .any(|voice| voice.id == "id-ID-GadisNeural"));
        }
    }

    #[test]
    fn voice_status_reports_daemon_visible_preferences() {
        let mut runtime = runtime_with_voice(true, true, 420);
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_voice_status"),
            ClientId::from("hud_1"),
            ClientRequest::VoiceStatus(EmptyPayload::default()),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        let status = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::VoiceStatusUpdated(payload) => Some(payload),
                _ => None,
            })
            .expect("voice status should be emitted");
        assert!(status.enabled);
        assert_eq!(status.provider, "stub");
        assert_eq!(status.voice_id, "id-ID-GadisNeural");
        assert_eq!(status.stt_language, "auto");
        assert_eq!(status.max_spoken_chars, 420);
        assert_eq!(status.bridge, "hud-local");
    }

    #[test]
    fn voice_preview_uses_daemon_provider_stub() {
        let mut runtime = runtime_with_voice(false, false, 800);
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_voice_preview"),
            ClientId::from("hud_1"),
            ClientRequest::VoicePreview(VoicePreviewRequest {
                text: "Halo, saya CADIS.".to_owned(),
                prefs: Some(VoicePreferences {
                    voice_id: "id-ID-GadisNeural".to_owned(),
                    rate: 5,
                    pitch: 0,
                    volume: 0,
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
            .any(|event| matches!(event.event, CadisEvent::VoicePreviewStarted(_))));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoicePreviewCompleted(_))));
    }

    #[test]
    fn voice_preview_blocks_unspeakable_text_by_policy() {
        let mut runtime = runtime_with_voice(false, false, 800);
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_voice_preview_empty"),
            ClientId::from("hud_1"),
            ClientRequest::VoicePreview(VoicePreviewRequest {
                text: "   ".to_owned(),
                prefs: None,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::VoicePreviewFailed(error)
                    if error.code == "empty_text"
                        && error.message.contains("not speakable")
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoicePreviewStarted(_))));
    }

    #[test]
    fn voice_stop_uses_daemon_provider_stop_contract() {
        let mut runtime = runtime_with_voice(true, true, 800);
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_voice_stop"),
            ClientId::from("hud_1"),
            ClientRequest::VoiceStop(EmptyPayload::default()),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoicePreviewCompleted(_))));
    }

    #[test]
    fn auto_speak_speaks_short_final_chat_response() {
        let mut runtime = runtime_with_voice(true, true, 10_000);
        let outcome = send_message_with_kind(&mut runtime, ContentKind::Chat, "hello");

        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::MessageCompleted(_))));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoiceStarted(_))));
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoiceCompleted(_))));
    }

    #[test]
    fn speech_policy_blocks_code_diff_and_terminal_log() {
        for content_kind in [
            ContentKind::Code,
            ContentKind::Diff,
            ContentKind::TerminalLog,
        ] {
            let mut runtime = runtime_with_voice(true, true, 10_000);
            let outcome = send_message_with_kind(&mut runtime, content_kind, "unsafe to speak");

            assert!(outcome
                .events
                .iter()
                .any(|event| matches!(event.event, CadisEvent::MessageCompleted(_))));
            assert!(!outcome
                .events
                .iter()
                .any(|event| matches!(event.event, CadisEvent::VoiceStarted(_))));
            assert!(!outcome
                .events
                .iter()
                .any(|event| matches!(event.event, CadisEvent::VoiceCompleted(_))));
        }
    }

    #[test]
    fn speech_policy_blocks_long_tool_output() {
        let prefs = VoiceRuntimePreferences {
            enabled: true,
            provider: "stub".to_owned(),
            voice_id: "id-ID-GadisNeural".to_owned(),
            stt_language: "auto".to_owned(),
            rate: 0,
            pitch: 0,
            volume: 0,
            auto_speak: true,
            max_spoken_chars: 40,
        };
        let output = "running 42 tests\n".repeat(12);

        assert_eq!(
            speech_decision(
                &prefs,
                ContentKind::TestResult,
                &output,
                SpeechMode::AutoSpeak
            ),
            SpeechDecision::Blocked("long_tool_output_not_speakable")
        );

        let mut runtime = runtime_with_voice(true, true, 40);
        let outcome = send_message_with_kind(&mut runtime, ContentKind::TestResult, &output);
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoiceStarted(_))));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::VoiceCompleted(_))));
    }

    #[test]
    fn builtin_tool_registry_contains_safe_and_gated_tools() {
        let registry = ToolRegistry::builtin().expect("registry should build");

        assert!(registry.is_auto_executable_safe_read("file.read"));
        assert!(registry.is_auto_executable_safe_read("file.search"));
        assert!(registry.is_auto_executable_safe_read("git.status"));
        assert!(registry.is_auto_executable_safe_read("git.diff"));
        assert!(!registry.is_auto_executable_safe_read("shell.run"));

        let shell = registry.get("shell.run").expect("shell tool exists");
        assert_eq!(shell.risk_class, cadis_protocol::RiskClass::SystemChange);
        assert_eq!(shell.execution, ToolExecutionMode::ApprovalPlaceholder);
    }

    #[test]
    fn builtin_tool_registry_declares_runtime_contract_metadata() {
        let registry = ToolRegistry::builtin().expect("registry should build");

        for definition in &registry.definitions {
            assert!(!definition.description.trim().is_empty());
            assert!(!definition.side_effects.is_empty());
            assert!(definition.timeout_secs > 0);
            assert_eq!(definition.timeout_behavior, ToolTimeoutBehavior::FailClosed);
        }

        let file_read = registry.get("file.read").expect("file.read should exist");
        assert_eq!(
            file_read.workspace_behavior,
            ToolWorkspaceBehavior::PathScoped
        );
        assert_eq!(
            file_read.cancellation_behavior,
            ToolCancellationBehavior::NotSupported
        );
        assert!(!file_read.needs_network);
        assert!(!file_read.may_read_secrets);

        let shell = registry.get("shell.run").expect("shell.run should exist");
        assert_eq!(
            shell.workspace_behavior,
            ToolWorkspaceBehavior::RequiresWorkspace
        );
        assert_eq!(
            shell.cancellation_behavior,
            ToolCancellationBehavior::Cooperative
        );
        assert!(shell.may_read_secrets);
    }

    #[test]
    fn tool_registry_rejects_duplicate_names() {
        let result = ToolRegistry::new(vec![
            ToolDefinition::safe_read(
                "file.read",
                "Read one file",
                ToolInputSchema::FileRead,
                &[ToolSideEffect::ReadFiles],
                5,
                ToolWorkspaceBehavior::PathScoped,
            ),
            ToolDefinition::safe_read(
                "file.read",
                "Search files",
                ToolInputSchema::FileSearch,
                &[ToolSideEffect::SearchFiles],
                5,
                ToolWorkspaceBehavior::PathScoped,
            ),
        ]);

        let error = result.expect_err("duplicate tool names should be rejected");
        assert_eq!(error.code, "duplicate_tool_name");
        assert!(error.message.contains("file.read"));
    }

    #[test]
    fn tool_registry_rejects_missing_runtime_contract_fields() {
        let missing_description = ToolRegistry::new(vec![ToolDefinition::safe_read(
            "file.read",
            "",
            ToolInputSchema::FileRead,
            &[ToolSideEffect::ReadFiles],
            5,
            ToolWorkspaceBehavior::PathScoped,
        )])
        .expect_err("empty descriptions should be rejected");
        assert_eq!(missing_description.code, "invalid_tool_description");

        let missing_side_effects = ToolRegistry::new(vec![ToolDefinition::safe_read(
            "file.read",
            "Read one file",
            ToolInputSchema::FileRead,
            &[],
            5,
            ToolWorkspaceBehavior::PathScoped,
        )])
        .expect_err("empty side effects should be rejected");
        assert_eq!(missing_side_effects.code, "invalid_tool_side_effects");

        let invalid_timeout = ToolRegistry::new(vec![ToolDefinition::safe_read(
            "file.read",
            "Read one file",
            ToolInputSchema::FileRead,
            &[ToolSideEffect::ReadFiles],
            0,
            ToolWorkspaceBehavior::PathScoped,
        )])
        .expect_err("zero timeout should be rejected");
        assert_eq!(invalid_timeout.code, "invalid_tool_timeout");
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
                CadisEvent::ToolRequested(payload)
                    if payload.summary.as_deref().is_some_and(|summary|
                        summary.contains("timeout=5s")
                            && summary.contains("workspace=PathScoped")
                    )
            )
        }));
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
    fn safe_git_diff_tool_runs_without_approval() {
        let workspace = test_workspace("git-diff");
        init_git_workspace(&workspace);
        fs::write(
            workspace.join("README.md"),
            "CADIS worker fixture\nupdated line\n",
        )
        .expect("tracked file should update");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "git-diff", &workspace);
        grant_workspace(&mut runtime, "git-diff", vec![WorkspaceAccess::Read]);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_git_diff"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "git.diff".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "git-diff",
                    "path": ".",
                    "pathspec": "README.md"
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
                    if payload.tool_name == "git.diff"
                        && payload.summary.as_deref().is_some_and(|summary| {
                            summary.contains("+updated line")
                                && summary.contains("diff --git")
                        })
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
    }

    #[test]
    fn git_diff_rejects_parent_pathspec() {
        let workspace = test_workspace("git-diff-pathspec");
        init_git_workspace(&workspace);
        let mut runtime = runtime();
        register_workspace(&mut runtime, "git-diff-pathspec", &workspace);
        grant_workspace(
            &mut runtime,
            "git-diff-pathspec",
            vec![WorkspaceAccess::Read],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_git_diff_pathspec"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "git.diff".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "git-diff-pathspec",
                    "path": ".",
                    "pathspec": "../README.md"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "git.diff"
                        && payload.error.code == "outside_workspace"
            )
        }));
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
    fn file_patch_requests_approval_without_execution() {
        let workspace = test_workspace("file-patch-approval");
        fs::write(workspace.join("README.md"), "hello\n").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-approval", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-approval",
            vec![WorkspaceAccess::Write],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_approval"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-approval",
                    "path": "README.md",
                    "old": "hello\n",
                    "new": "hello cadis\n"
                }),
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalRequested(payload)
                    if payload.tool_call_id.as_str() == "tool_000001"
                        && payload.summary.contains("edit_workspace")
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
        assert_eq!(
            fs::read_to_string(workspace.join("README.md")).expect("file should read"),
            "hello\n"
        );
    }

    #[test]
    fn file_patch_denial_does_not_apply_patch() {
        let workspace = test_workspace("file-patch-denial");
        fs::write(workspace.join("README.md"), "hello\n").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-denial", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-denial",
            vec![WorkspaceAccess::Write],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_denial"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-denial",
                    "path": "README.md",
                    "content": "patched\n"
                }),
            }),
        ));
        let approval_id = approval_id_from(&request);

        let denial = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_deny"),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id,
                decision: ApprovalDecision::Denied,
                reason: Some("not this patch".to_owned()),
            }),
        ));

        assert!(denial.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Denied
            )
        }));
        assert!(denial.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "file.patch"
                        && payload.error.code == "approval_denied"
            )
        }));
        assert_eq!(
            fs::read_to_string(workspace.join("README.md")).expect("file should read"),
            "hello\n"
        );
    }

    #[test]
    fn approved_file_patch_applies_inside_workspace() {
        let workspace = test_workspace("file-patch-approved");
        fs::write(workspace.join("README.md"), "hello\n").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-approved", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-approved",
            vec![WorkspaceAccess::Write],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_apply"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-approved",
                    "operations": [
                        {
                            "op": "replace",
                            "path": "README.md",
                            "old": "hello\n",
                            "new": "hello cadis\n"
                        }
                    ]
                }),
            }),
        ));
        let approved = approve(&mut runtime, approval_id_from(&request));

        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Approved
            )
        }));
        assert!(approved
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolCompleted(payload)
                    if payload.tool_name == "file.patch"
                        && payload.summary.as_deref() == Some("patched 1 file")
                        && payload.output.as_ref().is_some_and(|output| {
                            output["schema"] == "structured_replace_write_v1"
                                && output["truncated"] == false
                        })
            )
        }));
        assert_eq!(
            fs::read_to_string(workspace.join("README.md")).expect("file should read"),
            "hello cadis\n"
        );
    }

    #[test]
    fn file_patch_requires_workspace_write_grant() {
        let workspace = test_workspace("file-patch-grant");
        fs::write(workspace.join("README.md"), "hello\n").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-grant", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-grant",
            vec![WorkspaceAccess::Read],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_grant"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-grant",
                    "path": "README.md",
                    "content": "patched\n"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "file.patch"
                        && payload.error.code == "workspace_grant_required"
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
    }

    #[test]
    fn file_patch_rejects_outside_workspace_path() {
        let workspace = test_workspace("file-patch-outside");
        fs::write(workspace.join("README.md"), "hello\n").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-outside", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-outside",
            vec![WorkspaceAccess::Write],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_outside"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-outside",
                    "path": "../README.md",
                    "content": "patched\n"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "file.patch"
                        && payload.error.code == "outside_workspace"
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
    }

    #[test]
    fn file_patch_rejects_protected_path() {
        let workspace = test_workspace("file-patch-protected");
        fs::create_dir_all(workspace.join(".cadis")).expect("metadata dir should write");
        fs::write(
            workspace.join(".cadis/workspace.toml"),
            "workspace_id = 'x'\n",
        )
        .expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-protected", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-protected",
            vec![WorkspaceAccess::Write],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_protected"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-protected",
                    "path": ".cadis/workspace.toml",
                    "content": "patched\n"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "file.patch"
                        && payload.error.code == "protected_path"
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
    }

    #[test]
    fn file_patch_rejects_secret_like_path() {
        let workspace = test_workspace("file-patch-secret");
        fs::write(workspace.join(".env"), "TOKEN=secret\n").expect("test file should write");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "file-patch-secret", &workspace);
        grant_workspace(
            &mut runtime,
            "file-patch-secret",
            vec![WorkspaceAccess::Write],
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_file_patch_secret"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "file.patch".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "file-patch-secret",
                    "path": ".env",
                    "content": "TOKEN=patched\n"
                }),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "file.patch"
                        && payload.error.code == "secret_path_rejected"
            )
        }));
        assert!(!outcome
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
        assert_eq!(
            fs::read_to_string(workspace.join(".env")).expect("file should read"),
            "TOKEN=secret\n"
        );
    }

    #[test]
    fn recovered_file_patch_approval_fails_closed_on_approve() {
        let cadis_home = test_workspace("file-patch-recovery-home");
        let workspace = test_workspace("file-patch-recovery-workspace");
        fs::write(workspace.join("README.md"), "hello\n").expect("test file should write");
        let approval_id = {
            let mut runtime = runtime_with_home(cadis_home.clone());
            register_workspace(&mut runtime, "file-patch-recovery", &workspace);
            grant_workspace(
                &mut runtime,
                "file-patch-recovery",
                vec![WorkspaceAccess::Write],
            );

            let request = runtime.handle_request(RequestEnvelope::new(
                RequestId::from("req_file_patch_recovery"),
                ClientId::from("cli_1"),
                ClientRequest::ToolCall(ToolCallRequest {
                    session_id: None,
                    agent_id: None,
                    tool_name: "file.patch".to_owned(),
                    input: serde_json::json!({
                        "workspace_id": "file-patch-recovery",
                        "path": "README.md",
                        "content": "patched\n"
                    }),
                }),
            ));
            approval_id_from(&request)
        };

        let mut restarted = runtime_with_home(cadis_home);
        let approved = approve(&mut restarted, approval_id);

        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Approved
            )
        }));
        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.tool_name == "file.patch"
                        && payload.error.code == "tool_execution_unavailable"
            )
        }));
        assert_eq!(
            fs::read_to_string(workspace.join("README.md")).expect("file should read"),
            "hello\n"
        );
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
    fn workspace_doctor_reports_project_metadata_mismatch_and_duplicate_roots() {
        let workspace = test_workspace("doctor-metadata");
        cadis_store::ProjectWorkspaceStore::new(&workspace)
            .save(&cadis_store::ProjectWorkspaceMetadata {
                workspace_id: "wrong-id".to_owned(),
                kind: cadis_store::WorkspaceKind::Project,
                vcs: cadis_store::WorkspaceVcs::Git,
                worktree_root: PathBuf::from(".cadis/worktrees"),
                artifact_root: PathBuf::from(".cadis/artifacts"),
                media_root: PathBuf::from(".cadis/media"),
            })
            .expect("project workspace metadata should save");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "doctor-a", &workspace);
        register_workspace(&mut runtime, "doctor-b", &workspace);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_workspace_doctor"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceDoctor(WorkspaceDoctorRequest {
                workspace_id: Some(WorkspaceId::from("doctor-a")),
                root: None,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkspaceDoctorResponse(payload)
                    if payload.checks.iter().any(|check| check.name == "registry.duplicate_root"
                        && check.status == "warn")
                        && payload.checks.iter().any(|check| check.name == "workspace.metadata.id"
                            && check.status == "error")
            )
        }));
    }

    #[test]
    fn workspace_doctor_warns_when_project_metadata_is_missing() {
        let workspace = test_workspace("doctor-missing-metadata");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "doctor-missing", &workspace);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_workspace_doctor"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceDoctor(WorkspaceDoctorRequest {
                workspace_id: Some(WorkspaceId::from("doctor-missing")),
                root: None,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkspaceDoctorResponse(payload)
                    if payload.checks.iter().any(|check| check.name == "workspace.metadata"
                        && check.status == "warn")
            )
        }));
    }

    #[test]
    fn workspace_doctor_reports_stale_worker_worktree_metadata() {
        let workspace = test_workspace("doctor-stale-worktree");
        let mut runtime = runtime();
        let store = cadis_store::ProjectWorkspaceStore::new(&workspace);
        store
            .save(&cadis_store::ProjectWorkspaceMetadata {
                workspace_id: "doctor-stale".to_owned(),
                kind: cadis_store::WorkspaceKind::Project,
                vcs: cadis_store::WorkspaceVcs::Git,
                worktree_root: PathBuf::from(".cadis/worktrees"),
                artifact_root: PathBuf::from(".cadis/artifacts"),
                media_root: PathBuf::from(".cadis/media"),
            })
            .expect("project workspace metadata should save");
        store
            .save_worker_worktree_metadata(&cadis_store::ProjectWorkerWorktreeMetadata {
                worker_id: "worker_000001".to_owned(),
                workspace_id: "doctor-stale".to_owned(),
                worktree_path: PathBuf::from(".cadis/worktrees/worker_000001"),
                branch_name: "cadis/worker_000001/example".to_owned(),
                base_ref: Some("HEAD".to_owned()),
                state: cadis_store::ProjectWorkerWorktreeState::Planned,
                artifact_root: runtime
                    .profile_home
                    .root()
                    .join("artifacts/workers/worker_000001"),
            })
            .expect("worker worktree metadata should save");

        register_workspace(&mut runtime, "doctor-stale", &workspace);

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_workspace_doctor"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceDoctor(WorkspaceDoctorRequest {
                workspace_id: Some(WorkspaceId::from("doctor-stale")),
                root: None,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkspaceDoctorResponse(payload)
                    if payload.checks.iter().any(|check| check.name == "workspace.worktrees.worker_000001.path"
                        && check.status == "warn")
                        && payload.checks.iter().any(|check| check.name == "workspace.worktrees.worker_000001.artifacts"
                            && check.status == "warn")
            )
        }));
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
            ClientRequest::SessionSubscribe(SessionSubscriptionRequest {
                session_id: session_id.clone(),
                since_event_id: None,
                replay_limit: None,
                include_snapshot: true,
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
            ClientRequest::SessionSubscribe(SessionSubscriptionRequest {
                session_id,
                since_event_id: None,
                replay_limit: None,
                include_snapshot: true,
            }),
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
    fn worker_metadata_survives_runtime_restart_and_snapshot_replays_worker() {
        let cadis_home = test_workspace("persistent-worker-home");
        let worker_id = {
            let mut runtime = runtime_with_home(cadis_home.clone());
            let outcome = runtime.handle_request(RequestEnvelope::new(
                RequestId::from("req_worker"),
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
            outcome
                .events
                .iter()
                .find_map(|event| match &event.event {
                    CadisEvent::WorkerCompleted(payload) => Some(payload.worker_id.clone()),
                    _ => None,
                })
                .expect("worker.completed should be emitted")
        };

        assert_eq!(worker_id, "worker_000001");

        let mut runtime = runtime_with_home(cadis_home);
        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCompleted(payload)
                    if payload.worker_id == worker_id
                        && payload.agent_id.as_ref().map(AgentId::as_str) == Some("codex")
                        && payload.status.as_deref() == Some("completed")
            )
        }));

        let next = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_next_worker"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "/route @codex inspect next task".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(next.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerStarted(payload)
                    if payload.worker_id == "worker_000002"
                        && payload.status.as_deref() == Some("running")
            )
        }));
    }

    #[test]
    fn stale_running_worker_metadata_is_failed_on_runtime_recovery() {
        let cadis_home = test_workspace("stale-worker-home");
        let state_store = StateStore::new(&CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        });
        state_store
            .ensure_layout()
            .expect("state layout should initialize");
        state_store
            .write_worker_metadata(
                "worker_000007",
                &WorkerMetadata {
                    worker_id: "worker_000007".to_owned(),
                    session_id: SessionId::from("ses_stale"),
                    agent_id: Some(AgentId::from("codex")),
                    parent_agent_id: Some(AgentId::from("main")),
                    status: "running".to_owned(),
                    cli: None,
                    cwd: None,
                    summary: Some("stale worker".to_owned()),
                    error_code: None,
                    error: None,
                    cancellation_requested_at: None,
                    worktree: None,
                    artifacts: None,
                    updated_at: now_timestamp(),
                },
            )
            .expect("stale worker metadata should write");

        let mut runtime = runtime_with_home(cadis_home);
        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::DaemonError(payload)
                    if payload.code == "worker_recovered_stale"
                        && payload.message.contains("worker_000007")
            )
        }));
        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerFailed(payload)
                    if payload.worker_id == "worker_000007"
                        && payload.status.as_deref() == Some("failed")
                        && payload.error_code.as_deref() == Some("worker_recovered_stale")
                        && payload.summary.as_deref().is_some_and(|summary| {
                            summary.contains("marked failed during daemon recovery")
                        })
            )
        }));

        let recovered = state_store
            .recover_worker_metadata::<WorkerMetadata>()
            .expect("worker metadata should recover");
        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].metadata.status, "failed");
        assert_eq!(
            recovered.records[0].metadata.error_code.as_deref(),
            Some("worker_recovered_stale")
        );
    }

    #[test]
    fn failed_worker_generation_emits_worker_failed_event() {
        let mut runtime = runtime();
        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_worker_failure"),
                ClientId::from("hud_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content: "/route @codex run focused tests".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ))
            .expect("worker route should prepare model generation");
        let worker_id = pending
            .context
            .worker
            .as_ref()
            .expect("route should create a worker")
            .worker_id
            .clone();

        let events = runtime.fail_message_generation(
            pending,
            cadis_models::ModelError::with_code(
                "provider_client_error",
                "provider request failed",
                true,
            ),
        );

        assert!(events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerFailed(payload)
                    if payload.worker_id == worker_id
                        && payload.status.as_deref() == Some("failed")
                        && payload.error_code.as_deref() == Some("provider_client_error")
                        && payload.error.as_deref() == Some("provider request failed")
            )
        }));
    }

    #[test]
    fn session_cancel_marks_running_worker_cancelled_and_recovers_metadata() {
        let cadis_home = test_workspace("cancelled-worker-home");
        let mut runtime = runtime_with_home(cadis_home.clone());
        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_worker_cancel"),
                ClientId::from("hud_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content: "/route @codex wait for cancellation".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ))
            .expect("worker route should prepare model generation");
        let session_id = pending.context.session_id.clone();
        let worker_id = pending
            .context
            .worker
            .as_ref()
            .expect("route should create a worker")
            .worker_id
            .clone();

        let cancel = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancel_worker_session"),
            ClientId::from("hud_1"),
            ClientRequest::SessionCancel(SessionTargetRequest { session_id }),
        ));

        assert!(matches!(
            cancel.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(cancel.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCancelled(payload)
                    if payload.worker_id == worker_id
                        && payload.status.as_deref() == Some("cancelled")
                        && payload.error_code.as_deref() == Some("session_cancelled")
                        && payload.cancellation_requested_at.is_some()
            )
        }));

        let final_events = runtime.fail_message_generation(
            pending,
            cadis_models::ModelError::cancelled("model request was cancelled"),
        );
        assert!(final_events.is_empty());

        let state_store = StateStore::new(&CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        });
        let recovered = state_store
            .recover_worker_metadata::<WorkerMetadata>()
            .expect("worker metadata should recover");
        assert_eq!(recovered.records.len(), 1);
        assert_eq!(recovered.records[0].metadata.status, "cancelled");
        assert!(recovered.records[0]
            .metadata
            .cancellation_requested_at
            .is_some());

        let mut restarted = runtime_with_home(cadis_home);
        let snapshot = restarted.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancelled_worker_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));
        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCancelled(payload)
                    if payload.worker_id == worker_id
                        && payload.status.as_deref() == Some("cancelled")
                        && payload.cancellation_requested_at.is_some()
            )
        }));
    }

    #[test]
    fn worker_execution_creates_git_worktree_and_artifacts() {
        let cadis_home = test_workspace("worker-execution-home");
        let workspace = test_workspace("worker-execution-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        register_workspace(&mut runtime, "worker-git", &workspace);
        let session_id = runtime
            .handle_request(RequestEnvelope::new(
                RequestId::from("req_worker_session"),
                ClientId::from("cli_1"),
                ClientRequest::SessionCreate(SessionCreateRequest {
                    title: Some("Worker execution".to_owned()),
                    cwd: Some(workspace.display().to_string()),
                }),
            ))
            .events
            .into_iter()
            .find_map(|event| match event.event {
                CadisEvent::SessionStarted(payload) => Some(payload.session_id),
                _ => None,
            })
            .expect("session.started should be emitted");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_worker_execution"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: Some(session_id),
                target_agent_id: None,
                content: "/route @codex run focused tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let started = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerStarted(payload) => Some(payload),
                _ => None,
            })
            .expect("worker.started should be emitted");
        let worktree = started
            .worktree
            .as_ref()
            .expect("worker.started should include worktree metadata");
        assert_eq!(worktree.workspace_id.as_deref(), Some("worker-git"));
        assert_eq!(worktree.state, WorkerWorktreeState::Active);
        assert!(Path::new(&worktree.worktree_path).is_dir());
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerLogDelta(payload)
                    if payload.delta.contains("worktree ready")
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerLogDelta(payload)
                    if payload.delta == format!("command started: {WORKER_DEFAULT_COMMAND}\n")
            )
        }));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerLogDelta(payload)
                    if payload.delta == "command finished: status=passed exit_code=Some(0)\n"
            )
        }));

        let completed = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("worker.completed should be emitted");
        let completed_worktree = completed
            .worktree
            .as_ref()
            .expect("worker.completed should include worktree metadata");
        assert_eq!(completed_worktree.state, WorkerWorktreeState::ReviewPending);

        let artifacts = completed
            .artifacts
            .as_ref()
            .expect("worker.completed should include artifact paths");
        assert!(Path::new(&artifacts.summary).is_file());
        assert!(Path::new(&artifacts.test_report).is_file());
        assert!(Path::new(&artifacts.changed_files).is_file());
        assert!(Path::new(&artifacts.patch).is_file());

        let test_report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&artifacts.test_report).expect("test report should read"),
        )
        .expect("test report should be JSON");
        assert_eq!(
            test_report["validation_command"]["command"],
            WORKER_DEFAULT_COMMAND
        );
        assert_eq!(test_report["validation_command"]["status"], "passed");
        let reported_cwd = test_report["validation_command"]["cwd"]
            .as_str()
            .expect("validation command cwd should be a string");
        assert_eq!(
            Path::new(reported_cwd)
                .canonicalize()
                .expect("reported cwd should canonicalize"),
            Path::new(&worktree.worktree_path)
                .canonicalize()
                .expect("worker worktree path should canonicalize")
        );
        let summary = fs::read_to_string(&artifacts.summary).expect("summary should read");
        assert!(summary.contains("## Daemon Validation"));
        assert!(summary.contains("Status: completed"));

        let project_metadata = ProjectWorkspaceStore::new(&workspace)
            .load_worker_worktree_metadata("worker_000001")
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(project_metadata.workspace_id, "worker-git");
        assert_eq!(
            project_metadata.state,
            ProjectWorkerWorktreeState::ReviewPending
        );
    }

    #[test]
    fn worker_cleanup_request_marks_review_worktree_cleanup_pending_without_deleting() {
        let cadis_home = test_workspace("worker-cleanup-home");
        let workspace = test_workspace("worker-cleanup-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home.clone());
        register_workspace(&mut runtime, "worker-cleanup", &workspace);
        let (_session_id, worker_id, worktree_path) =
            complete_worker_in_workspace(&mut runtime, &workspace, "cleanup_request");

        let cleanup = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_worker_cleanup"),
            ClientId::from("hud_1"),
            ClientRequest::WorkerCleanup(WorkerCleanupRequest {
                worker_id: worker_id.clone(),
                worktree_path: Some(worktree_path.clone()),
            }),
        ));

        assert!(matches!(
            cleanup.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(Path::new(&worktree_path).is_dir());
        assert!(!cleanup
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
        assert!(cleanup.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCleanupRequested(payload)
                    if payload.worker_id == worker_id
                        && payload.worktree.as_ref().is_some_and(|worktree|
                            worktree.state == WorkerWorktreeState::CleanupPending
                        )
            )
        }));

        let project_metadata = ProjectWorkspaceStore::new(&workspace)
            .load_worker_worktree_metadata(&worker_id)
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(
            project_metadata.state,
            ProjectWorkerWorktreeState::CleanupPending
        );

        let state_store = StateStore::new(&CadisConfig {
            cadis_home,
            ..CadisConfig::default()
        });
        let recovered = state_store
            .recover_worker_metadata::<WorkerMetadata>()
            .expect("worker metadata should recover");
        let worker = recovered
            .records
            .iter()
            .find(|record| record.metadata.worker_id == worker_id)
            .expect("worker metadata should exist");
        assert_eq!(
            worker
                .metadata
                .worktree
                .as_ref()
                .expect("worker metadata should include worktree")
                .state,
            WorkerWorktreeState::CleanupPending
        );
    }

    #[test]
    fn session_cancel_marks_cadis_worktree_cleanup_pending_metadata() {
        let cadis_home = test_workspace("cancelled-worker-worktree-home");
        let workspace = test_workspace("cancelled-worker-worktree-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home.clone());
        register_workspace(&mut runtime, "cancel-cleanup", &workspace);
        let session_id = runtime
            .handle_request(RequestEnvelope::new(
                RequestId::from("req_cancel_cleanup_session"),
                ClientId::from("cli_1"),
                ClientRequest::SessionCreate(SessionCreateRequest {
                    title: Some("Worker cancellation cleanup".to_owned()),
                    cwd: Some(workspace.display().to_string()),
                }),
            ))
            .events
            .into_iter()
            .find_map(|event| match event.event {
                CadisEvent::SessionStarted(payload) => Some(payload.session_id),
                _ => None,
            })
            .expect("session.started should be emitted");
        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_cancel_cleanup_worker"),
                ClientId::from("hud_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: Some(session_id.clone()),
                    target_agent_id: None,
                    content: "/route @codex wait for cancellation".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ))
            .expect("worker route should prepare model generation");
        let worker_id = pending
            .context
            .worker
            .as_ref()
            .expect("route should create a worker")
            .worker_id
            .clone();

        let cancel = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancel_cleanup"),
            ClientId::from("hud_1"),
            ClientRequest::SessionCancel(SessionTargetRequest { session_id }),
        ));

        assert!(matches!(
            cancel.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(cancel.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCancelled(payload)
                    if payload.worker_id == worker_id
                        && payload.worktree.as_ref().is_some_and(|worktree|
                            worktree.state == WorkerWorktreeState::CleanupPending
                        )
            )
        }));

        let project_metadata = ProjectWorkspaceStore::new(&workspace)
            .load_worker_worktree_metadata(&worker_id)
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(
            project_metadata.state,
            ProjectWorkerWorktreeState::CleanupPending
        );

        let state_store = StateStore::new(&CadisConfig {
            cadis_home,
            ..CadisConfig::default()
        });
        let recovered = state_store
            .recover_worker_metadata::<WorkerMetadata>()
            .expect("worker metadata should recover");
        let worker = recovered
            .records
            .iter()
            .find(|record| record.metadata.worker_id == worker_id)
            .expect("worker metadata should exist");
        assert_eq!(worker.metadata.status, "cancelled");
        assert_eq!(
            worker
                .metadata
                .worktree
                .as_ref()
                .expect("worker metadata should include worktree")
                .state,
            WorkerWorktreeState::CleanupPending
        );
    }

    #[test]
    fn worker_cleanup_fails_closed_for_unknown_missing_and_non_owned_worktrees() {
        let cadis_home = test_workspace("worker-cleanup-fail-closed-home");
        let workspace = test_workspace("worker-cleanup-fail-closed-workspace");
        let external = test_workspace("worker-cleanup-external");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        let unknown = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_unknown_worker_cleanup"),
            ClientId::from("hud_1"),
            ClientRequest::WorkerCleanup(WorkerCleanupRequest {
                worker_id: "worker_missing".to_owned(),
                worktree_path: None,
            }),
        ));
        assert_rejected(unknown, "worker_not_found");

        register_workspace(&mut runtime, "worker-cleanup-fail", &workspace);
        let (_session_id, worker_id, worktree_path) =
            complete_worker_in_workspace(&mut runtime, &workspace, "cleanup_fail_closed");

        let non_owned = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_non_owned_worker_cleanup"),
            ClientId::from("hud_1"),
            ClientRequest::WorkerCleanup(WorkerCleanupRequest {
                worker_id: worker_id.clone(),
                worktree_path: Some(external.display().to_string()),
            }),
        ));
        assert_rejected(non_owned, "worker_worktree_not_owned");
        let project_metadata = ProjectWorkspaceStore::new(&workspace)
            .load_worker_worktree_metadata(&worker_id)
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(
            project_metadata.state,
            ProjectWorkerWorktreeState::ReviewPending
        );

        fs::remove_dir_all(&worktree_path).expect("worker worktree should be removable in test");
        let missing = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_missing_worker_worktree_cleanup"),
            ClientId::from("hud_1"),
            ClientRequest::WorkerCleanup(WorkerCleanupRequest {
                worker_id: worker_id.clone(),
                worktree_path: Some(worktree_path),
            }),
        ));
        assert_rejected(missing, "worker_worktree_missing");
        let project_metadata = ProjectWorkspaceStore::new(&workspace)
            .load_worker_worktree_metadata(&worker_id)
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(
            project_metadata.state,
            ProjectWorkerWorktreeState::ReviewPending
        );
    }

    #[test]
    fn worker_command_logs_are_bounded_and_redacted() {
        let cadis_home = test_workspace("worker-command-redaction-home");
        let workspace = test_workspace("worker-command-redaction-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        register_workspace(&mut runtime, "worker-redaction", &workspace);
        let pending = begin_workspace_worker_message(
            &mut runtime,
            &workspace,
            "req_worker_command_redaction",
            "/worker run focused tests",
        );
        let started = worker_started_payload(&pending.initial_events);
        let worktree = started
            .worktree
            .as_ref()
            .expect("worker.started should include worktree");
        let worktree_path = PathBuf::from(&worktree.worktree_path);

        fs::write(
            worktree_path.join("000-secret=secret-value.txt"),
            "redact me",
        )
        .expect("secret-looking fixture should write");
        let long_component = "a".repeat(80);
        for index in 0..300 {
            fs::write(
                worktree_path.join(format!("file-{index:03}-{long_component}.txt")),
                "x",
            )
            .expect("large status fixture should write");
        }

        let events = runtime.complete_message_generation(
            pending,
            test_model_response(),
            "worker done".to_owned(),
            false,
        );
        let stdout_logs = events
            .iter()
            .filter_map(|event| match &event.event {
                CadisEvent::WorkerLogDelta(payload)
                    if payload.delta.starts_with("command stdout:\n") =>
                {
                    Some(payload.delta.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!stdout_logs.is_empty());
        assert!(stdout_logs
            .iter()
            .all(|delta| delta.len() <= WORKER_COMMAND_LOG_LIMIT_BYTES));

        let joined_logs = stdout_logs.join("");
        assert!(joined_logs.contains("secret=[REDACTED]"));
        assert!(!joined_logs.contains("secret-value"));
        assert!(joined_logs.contains("[stdout truncated]"));

        let completed = events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("worker should complete after passing validation");
        let artifacts = completed
            .artifacts
            .as_ref()
            .expect("worker.completed should include artifacts");
        let test_report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&artifacts.test_report).expect("test report should read"),
        )
        .expect("test report should be JSON");
        let stdout = test_report["validation_command"]["stdout"]
            .as_str()
            .expect("stdout should be a string");
        assert!(!stdout.contains("secret-value"));
        assert_eq!(test_report["validation_command"]["stdout_truncated"], true);
    }

    #[test]
    fn worker_command_nonzero_exit_marks_worker_failed_and_updates_report() {
        let cadis_home = test_workspace("worker-command-failure-home");
        let workspace = test_workspace("worker-command-failure-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        register_workspace(&mut runtime, "worker-command-failure", &workspace);
        let pending = begin_workspace_worker_message(
            &mut runtime,
            &workspace,
            "req_worker_command_failure",
            "/worker run focused tests",
        );
        let started = worker_started_payload(&pending.initial_events);
        let worker_id = started.worker_id.clone();
        let worktree = started
            .worktree
            .as_ref()
            .expect("worker.started should include worktree");
        let worktree_path = PathBuf::from(&worktree.worktree_path);
        fs::write(
            worktree_path.join(".git"),
            "gitdir: /cadis/missing/gitdir\n",
        )
        .expect("worktree gitdir should be made invalid");

        let events = runtime.complete_message_generation(
            pending,
            test_model_response(),
            "worker done".to_owned(),
            false,
        );
        assert!(!events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCompleted(payload) if payload.worker_id == worker_id
            )
        }));
        let failed = events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerFailed(payload) if payload.worker_id == worker_id => {
                    Some(payload)
                }
                _ => None,
            })
            .expect("worker.failed should be emitted");
        assert_eq!(failed.status.as_deref(), Some("failed"));
        assert_eq!(failed.error_code.as_deref(), Some("worker_command_failed"));
        assert!(failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("worker command exited with code")));

        let artifacts = failed
            .artifacts
            .as_ref()
            .expect("worker.failed should include artifacts");
        let test_report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&artifacts.test_report).expect("test report should read"),
        )
        .expect("test report should be JSON");
        assert_eq!(test_report["status"], "failed");
        assert_eq!(test_report["validation_command"]["status"], "failed");
        assert_ne!(
            test_report["validation_command"]["exit_code"].as_i64(),
            Some(0)
        );
        let summary = fs::read_to_string(&artifacts.summary).expect("summary should read");
        assert!(summary.contains("Status: failed"));
        assert!(summary.contains("## Daemon Validation"));
    }

    #[test]
    fn invalid_session_metadata_surfaces_recovery_diagnostic_in_snapshot() {
        let cadis_home = test_workspace("invalid-session-recovery");
        let state_store = StateStore::new(&CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        });
        state_store
            .ensure_layout()
            .expect("state layout should initialize");
        fs::write(state_store.state_dir().join("sessions/broken.json"), "{")
            .expect("corrupt session metadata should write");

        let mut runtime = runtime_with_home(cadis_home);
        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::DaemonError(payload)
                    if payload.code == "session_metadata_recovery_skipped"
                        && payload.message.contains("broken.json")
            )
        }));
    }

    #[test]
    fn runtime_initializes_agent_home_templates() {
        let cadis_home = test_workspace("runtime-agent-home");
        let mut runtime = runtime_with_home(cadis_home.clone());
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn_agent_home"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Review".to_owned(),
                parent_agent_id: Some(AgentId::from("main")),
                display_name: Some("Review Scout".to_owned()),
                model: Some("echo".to_owned()),
            }),
        ));
        let spawned_id = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                _ => None,
            })
            .expect("agent.spawned should be emitted");

        let profile = CadisHome::new(cadis_home).profile("default");
        assert!(profile.agent("main").agent_toml_path().is_file());
        let spawned_home = profile.agent(spawned_id.as_str());
        assert!(spawned_home.agent_toml_path().is_file());
        assert!(spawned_home.policy_toml_path().is_file());
        let metadata = spawned_home
            .load_metadata()
            .expect("spawned AGENT.toml should parse");
        assert_eq!(metadata.agent.display_name, "Review Scout");
        assert_eq!(metadata.agent.role, "Review");
        assert_eq!(metadata.agent.parent_agent_id.as_deref(), Some("main"));
    }

    #[test]
    fn workspace_doctor_reports_agent_home_diagnostics() {
        let cadis_home = test_workspace("runtime-agent-doctor");
        let mut runtime = runtime_with_home(cadis_home.clone());
        let policy_path = CadisHome::new(cadis_home)
            .profile("default")
            .agent("main")
            .policy_toml_path();
        fs::write(policy_path, "[policy\n").expect("corrupt policy should write");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_agent_doctor"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceDoctor(WorkspaceDoctorRequest {
                workspace_id: None,
                root: None,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkspaceDoctorResponse(payload)
                    if payload.checks.iter().any(|check| check.name == "main/agent.POLICY.toml"
                        && check.status == "error")
            )
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
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalRequested(payload)
                    if payload.summary.contains("side effects: run_subprocess")
                        && payload.summary.contains("cancellation: Cooperative")
                        && payload.summary.contains("secrets: true")
            )
        }));
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
    fn risky_shell_tool_runs_simple_command_after_approval() {
        let workspace = test_workspace("shell-approved");
        fs::create_dir_all(workspace.join("subdir")).expect("subdir should be created");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "shell-approved", &workspace);
        grant_workspace(&mut runtime, "shell-approved", vec![WorkspaceAccess::Exec]);

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_shell_approved"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "shell-approved",
                    "cwd": "subdir",
                    "command": "printf cadis-shell-ok"
                }),
            }),
        ));

        assert!(request.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalRequested(payload)
                    if payload.command.as_deref() == Some("printf cadis-shell-ok")
            )
        }));
        assert!(!request
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));

        let approval_id = approval_id_from(&request);
        let approved = approve(&mut runtime, approval_id);

        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Approved
            )
        }));
        assert!(approved
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
        let completed = approved
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::ToolCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("approved shell.run should complete");
        assert_eq!(completed.summary.as_deref(), Some("cadis-shell-ok"));
        let output = completed
            .output
            .as_ref()
            .expect("shell output should exist");
        assert_eq!(output["cwd"], "subdir");
        assert_eq!(output["stdout"], "cadis-shell-ok");
        assert_eq!(output["exit_code"], 0);
    }

    #[test]
    fn risky_shell_tool_rechecks_exec_grant_at_approval_time() {
        let workspace = test_workspace("shell-grant-recheck");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "shell-grant-recheck", &workspace);
        grant_workspace(
            &mut runtime,
            "shell-grant-recheck",
            vec![WorkspaceAccess::Exec],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_shell_recheck"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "shell-grant-recheck",
                    "command": "printf should-not-run"
                }),
            }),
        ));
        let approval_id = approval_id_from(&request);
        let grant_id = runtime
            .workspace_grants
            .keys()
            .next()
            .cloned()
            .expect("workspace grant should exist");

        let revoke = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_revoke_before_approval"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceRevoke(WorkspaceRevokeRequest {
                grant_id: Some(grant_id),
                workspace_id: None,
                agent_id: None,
            }),
        ));
        assert!(matches!(
            revoke.response.response,
            DaemonResponse::RequestAccepted(_)
        ));

        let approved = approve(&mut runtime, approval_id);
        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "workspace_grant_required"
            )
        }));
        assert!(!approved
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
    }

    #[test]
    fn risky_shell_tool_timeout_fails_closed() {
        let workspace = test_workspace("shell-timeout");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "shell-timeout", &workspace);
        grant_workspace(&mut runtime, "shell-timeout", vec![WorkspaceAccess::Exec]);

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_shell_timeout"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "shell-timeout",
                    "command": "sleep 2",
                    "timeout_ms": 50
                }),
            }),
        ));

        let approved = approve(&mut runtime, approval_id_from(&request));
        assert!(approved
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "tool_timeout"
                        && payload.error.message.contains("timeout_ms=50")
            )
        }));
        assert!(!approved
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolCompleted(_))));
    }

    #[test]
    fn risky_shell_tool_output_is_bounded_and_redacted() {
        let workspace = test_workspace("shell-output-redaction");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "shell-output-redaction", &workspace);
        grant_workspace(
            &mut runtime,
            "shell-output-redaction",
            vec![WorkspaceAccess::Exec],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_shell_output_redaction"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "shell-output-redaction",
                    "command": "printf 'token=secret-value\\n'; printf '%*s' 20000 '' | tr ' ' A"
                }),
            }),
        ));

        let approved = approve(&mut runtime, approval_id_from(&request));
        let completed = approved
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::ToolCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("approved shell.run should complete");
        let output = completed
            .output
            .as_ref()
            .expect("shell output should exist");
        let stdout = output["stdout"]
            .as_str()
            .expect("stdout should be a string");

        assert!(stdout.contains("token=[REDACTED]"));
        assert!(!stdout.contains("secret-value"));
        assert!(stdout.len() <= SHELL_OUTPUT_LIMIT_BYTES);
        assert_eq!(output["stdout_truncated"], true);
        assert!(completed
            .summary
            .as_deref()
            .is_some_and(|summary| !summary.contains("secret-value")));
    }

    #[test]
    fn pending_approval_survives_runtime_restart_snapshot_and_denial() {
        let cadis_home = test_workspace("approval-recovery-home");
        let workspace = test_workspace("approval-recovery-workspace");
        let approval_id = {
            let mut runtime = runtime_with_home(cadis_home.clone());
            register_workspace(&mut runtime, "approval-recovery", &workspace);
            grant_workspace(
                &mut runtime,
                "approval-recovery",
                vec![WorkspaceAccess::Exec],
            );

            let outcome = runtime.handle_request(RequestEnvelope::new(
                RequestId::from("req_shell_approval"),
                ClientId::from("cli_1"),
                ClientRequest::ToolCall(ToolCallRequest {
                    session_id: None,
                    agent_id: None,
                    tool_name: "shell.run".to_owned(),
                    input: serde_json::json!({
                        "workspace_id": "approval-recovery",
                        "command": "echo hello"
                    }),
                }),
            ));

            outcome
                .events
                .iter()
                .find_map(|event| match &event.event {
                    CadisEvent::ApprovalRequested(payload) => Some(payload.approval_id.clone()),
                    _ => None,
                })
                .expect("approval.requested should be emitted")
        };

        let mut runtime = runtime_with_home(cadis_home.clone());
        let snapshot = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_approval_snapshot"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
        ));

        assert!(snapshot.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalRequested(payload)
                    if payload.approval_id == approval_id
                        && payload.tool_call_id.as_str() == "tool_000001"
                        && payload.summary.contains("run_subprocess")
            )
        }));

        let next = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_second_shell_approval"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "approval-recovery",
                    "command": "echo next"
                }),
            }),
        ));
        assert!(next.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalRequested(payload)
                    if payload.approval_id.as_str() == "apr_000002"
            )
        }));

        let deny = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_deny_recovered"),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id: approval_id.clone(),
                decision: ApprovalDecision::Denied,
                reason: Some("not needed".to_owned()),
            }),
        ));

        assert!(deny.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.approval_id == approval_id
                        && payload.decision == ApprovalDecision::Denied
            )
        }));
        assert!(deny.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "approval_denied"
            )
        }));

        let mut restarted = runtime_with_home(cadis_home);
        let repeated = restarted.handle_request(RequestEnvelope::new(
            RequestId::from("req_deny_recovered_again"),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id,
                decision: ApprovalDecision::Denied,
                reason: Some("again".to_owned()),
            }),
        ));

        assert!(matches!(
            repeated.response.response,
            DaemonResponse::RequestRejected(error)
                if error.code == "approval_already_resolved"
        ));
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
    fn begin_message_request_prepares_progress_without_calling_provider() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingProvider {
            calls: Arc::clone(&calls),
        };
        let mut runtime = runtime_with_provider(Box::new(provider), "counting");

        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_1"),
                ClientId::from("cli_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content: "hello".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ))
            .expect("message request should be prepared");

        assert!(matches!(
            pending.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(pending
            .initial_events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::SessionStarted(_))));
        assert!(pending.initial_events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentStatusChanged(payload)
                    if payload.status == AgentStatus::Running
            )
        }));
        assert!(!pending
            .initial_events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::MessageCompleted(_))));
    }

    #[test]
    fn pending_message_generation_observes_session_cancel() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingProvider {
            calls: Arc::clone(&calls),
        };
        let mut runtime = runtime_with_provider(Box::new(provider), "counting");

        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_1"),
                ClientId::from("cli_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content: "cancel me".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ))
            .expect("message request should be prepared");
        let session_id = pending.context.session_id.clone();

        assert!(!runtime.message_generation_cancelled(&pending));

        let cancel = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancel"),
            ClientId::from("cli_2"),
            ClientRequest::SessionCancel(SessionTargetRequest {
                session_id: session_id.clone(),
            }),
        ));

        assert!(matches!(
            cancel.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(runtime.message_generation_cancelled(&pending));

        let final_events = runtime.fail_message_generation(
            pending,
            cadis_models::ModelError::cancelled("model request was cancelled"),
        );
        assert!(final_events.is_empty());
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
    fn session_subscribe_can_skip_initial_snapshot() {
        let mut runtime = runtime();
        let create = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_create"),
            ClientId::from("cli_1"),
            ClientRequest::SessionCreate(SessionCreateRequest {
                title: Some("Quiet session stream".to_owned()),
                cwd: None,
            }),
        ));
        let session_id = create
            .events
            .into_iter()
            .find_map(|event| match event.event {
                CadisEvent::SessionStarted(payload) => Some(payload.session_id),
                _ => None,
            })
            .expect("session.started should be emitted");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_subscribe"),
            ClientId::from("cli_1"),
            ClientRequest::SessionSubscribe(SessionSubscriptionRequest {
                session_id,
                since_event_id: None,
                replay_limit: None,
                include_snapshot: false,
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
    fn message_send_surfaces_structured_provider_error() {
        let provider = provider_from_config(
            "openai",
            "http://127.0.0.1:11434",
            "llama3.2",
            "https://api.openai.com/v1",
            "gpt-5.2",
            None,
        );
        let mut runtime = runtime_with_provider(provider, "openai");

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_chat"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("main")),
                content: "hello".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::SessionFailed(error)
                    if error.code == "model_auth_missing"
                        && !error.retryable
                        && error.message.contains("OpenAI provider requires")
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
    fn models_list_uses_runtime_model_config() {
        let mut runtime = Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cadis-home-model-config"),
                profile_id: "default".to_owned(),
                socket_path: Some(PathBuf::from("/tmp/cadis-test.sock")),
                model_provider: "openai".to_owned(),
                ollama_model: "qwen2.5-coder".to_owned(),
                openai_model: "gpt-5.4".to_owned(),
                openai_api_key_configured: true,
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
            Box::<EchoProvider>::default(),
        );

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::ModelsList(EmptyPayload::default()),
        ));
        let models = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::ModelsListResponse(payload) => Some(&payload.models),
                _ => None,
            })
            .expect("models.list.response should be emitted");

        let ollama = models
            .iter()
            .find(|model| model.provider == "ollama")
            .expect("ollama should be listed");
        assert_eq!(ollama.model, "qwen2.5-coder");
        assert_eq!(ollama.effective_model.as_deref(), Some("qwen2.5-coder"));

        let openai = models
            .iter()
            .find(|model| model.provider == "openai")
            .expect("openai should be listed");
        assert_eq!(openai.model, "gpt-5.4");
        assert_eq!(openai.effective_model.as_deref(), Some("gpt-5.4"));
        assert_eq!(openai.readiness, Some(ModelReadiness::Ready));
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
            .profile_home
            .root()
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
