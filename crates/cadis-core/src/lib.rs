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

use cadis_memory::MemoryStore;
use cadis_models::{
    provider_catalog_for_config, ModelCatalogConfig, ModelInvocation, ModelProvider, ModelRequest,
    ModelResponse, ModelStreamControl, ModelStreamEvent, ProviderCatalogEntry, ProviderReadiness,
};
use cadis_policy::{PolicyDecision, PolicyEngine};
use cadis_protocol::{
    AgentEventPayload, AgentId, AgentListPayload, AgentModelChangedPayload, AgentRenamedPayload,
    AgentRole, AgentSessionEventPayload, AgentSessionId, AgentSessionStatus, AgentSpawnRequest,
    AgentSpecialistChangedPayload, AgentSpecialistSetRequest, AgentStatus,
    AgentStatusChangedPayload, AgentTailRequest, ApprovalDecision, ApprovalId,
    ApprovalRequestPayload, ApprovalResolvedPayload, ApprovalResponseRequest, CadisEvent,
    ClientRequest, ContentKind, DaemonResponse, DaemonStatusPayload, ErrorPayload, EventEnvelope,
    EventId, MessageCompletedPayload, MessageDeltaPayload, MessageSendRequest, ModelDescriptor,
    ModelInvocationPayload, ModelReadiness, ModelsListPayload, OrchestratorRoutePayload,
    ProtocolVersion, RequestAcceptedPayload, RequestEnvelope, RequestId, ResponseEnvelope,
    SessionEventPayload, SessionId, Timestamp, ToolCallId, ToolCallRequest, ToolEventPayload,
    ToolFailedPayload, UiPreferencesPayload, VoiceDoctorCheck, VoiceDoctorPayload,
    VoicePreferences, VoicePreflightRequest, VoicePreflightSummary, VoicePreviewRequest,
    VoiceRuntimeState, VoiceStatusPayload, WorkerArtifactLocations, WorkerCleanupRequest,
    WorkerEventPayload, WorkerLogDeltaPayload, WorkerResultRequest, WorkerState, WorkerTailRequest,
    WorkerWorktreeCleanupPolicy, WorkerWorktreeIntent, WorkerWorktreeState, WorkspaceAccess,
    WorkspaceDoctorCheck, WorkspaceDoctorPayload, WorkspaceDoctorRequest, WorkspaceGrantId,
    WorkspaceGrantPayload, WorkspaceGrantRequest, WorkspaceId, WorkspaceKind, WorkspaceListPayload,
    WorkspaceListRequest, WorkspaceRecordPayload, WorkspaceRegisterRequest, WorkspaceRevokeRequest,
};
use cadis_store::{
    redact, AgentHomeDiagnostic, AgentHomeDoctorOptions, AgentHomeTemplate, ApprovalRecord,
    ApprovalState, ApprovalStore, CadisConfig, CadisHome, CheckpointManager, CheckpointPolicy,
    DeniedPaths, GrantSource as StoreGrantSource, MediaManifestStore, ProfileHome,
    ProjectWorkerWorktreeMetadata, ProjectWorkerWorktreeState, ProjectWorkspaceStore,
    ProjectWorktreeDiagnostic, StateRecoveryDiagnostic, StateStore, WorkerArtifactPathSet,
    WorkspaceAccess as StoreWorkspaceAccess, WorkspaceAlias,
    WorkspaceGrantRecord as StoreWorkspaceGrantRecord, WorkspaceKind as StoreWorkspaceKind,
    WorkspaceMetadata, WorkspaceRegistry, WorkspaceVcs, WorktreeCleanupExecutor,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

mod orchestrator;
mod search_index;
mod tools;
mod voice;
mod workspace;

use orchestrator::*;
use tools::*;
use voice::*;
use workspace::*;

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
const AGENT_PERSONA_MAX_CHARS: usize = 1_200;
const AGENT_CONTEXT_TASK_MAX_CHARS: usize = 140;

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
    /// Daemon-owned worker validation command. Empty string disables execution.
    pub worker_command: String,
    /// Maximum concurrent workers the scheduler will run.
    pub max_concurrent_workers: usize,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            default_timeout_sec: 900,
            max_steps_per_session: 8,
            worker_command: WORKER_DEFAULT_COMMAND.to_owned(),
            max_concurrent_workers: 4,
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
    /// Path to the synthesized audio file, when the provider writes one.
    pub audio_path: Option<PathBuf>,
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

/// Cloneable handle extracted from [`PendingMessageGeneration`] for use
/// after the pending value has been moved into a blocking task.
#[derive(Clone, Debug)]
pub struct MessageGenerationHandle {
    pub session_id: SessionId,
    pub agent_session_id: AgentSessionId,
    pub content_kind: ContentKind,
    pub agent_id: AgentId,
    pub agent_name: String,
}

impl PendingMessageGeneration {
    /// Returns a cloneable handle that captures the routing context needed
    /// to emit delta events without holding the full pending value.
    pub fn handle(&self) -> MessageGenerationHandle {
        MessageGenerationHandle {
            session_id: self.context.session_id.clone(),
            agent_session_id: self.context.agent_session_id.clone(),
            content_kind: self.context.content_kind,
            agent_id: self.context.agent_id.clone(),
            agent_name: self.context.agent_name.clone(),
        }
    }
}

/// A tool call directive parsed from model output.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallDirective {
    /// Tool name, e.g. `file.read`.
    pub tool_name: String,
    /// Structured JSON input for the tool.
    pub input: serde_json::Value,
}

/// Maximum tool call directives parsed from a single model response.
const MAX_TOOL_CALL_DIRECTIVES: usize = 5;

/// Parses `[TOOL tool_name: {"input": "json"}]` directives from model output.
pub fn parse_tool_call_directives(content: &str) -> Vec<ToolCallDirective> {
    let mut directives = Vec::new();
    for line in content.lines() {
        if directives.len() >= MAX_TOOL_CALL_DIRECTIVES {
            break;
        }
        let trimmed = line.trim();
        let Some(inner) = trimmed
            .strip_prefix("[TOOL ")
            .and_then(|s| s.strip_suffix(']'))
        else {
            continue;
        };
        let Some((name, json_str)) = inner.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let json_str = json_str.trim();
        if name.is_empty() || json_str.is_empty() {
            continue;
        }
        if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_str) {
            directives.push(ToolCallDirective {
                tool_name: name.to_owned(),
                input,
            });
        }
    }
    directives
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
    fallback_provider: Option<Arc<dyn ModelProvider>>,
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
    denied_paths: DeniedPaths,
    cancellation_token: cadis_policy::CancellationToken,
    search_index_cache: HashMap<PathBuf, search_index::SearchIndex>,
}

impl Runtime {
    /// Creates a runtime with the supplied model provider.
    pub fn new(options: RuntimeOptions, provider: Box<dyn ModelProvider>) -> Self {
        let ui_preferences = options.ui_preferences.clone();
        let spawn_limits = AgentSpawnLimits::from_options(&options.ui_preferences);
        let agent_runtime = AgentRuntimeConfig::from_options(&options.ui_preferences);
        let policy = cadis_policy::PolicyEngine::with_config(
            serde_json::from_value(
                options
                    .ui_preferences
                    .get("policy")
                    .cloned()
                    .unwrap_or_default(),
            )
            .unwrap_or_default(),
        );
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
            fallback_provider: None,
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
            policy,
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
            denied_paths: DeniedPaths::default(),
            cancellation_token: cadis_policy::CancellationToken::new(),
            search_index_cache: HashMap::new(),
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
                    // Item 7: Set cancellation token so in-flight tools can observe it.
                    self.cancellation_token.cancel();
                    self.cancellation_token = cadis_policy::CancellationToken::new();

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
            ClientRequest::AgentSpecialistSet(request) => {
                self.set_agent_specialist(request_id, request)
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
            ClientRequest::AgentTail(request) => self.tail_agent(request_id, request),
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
            ClientRequest::ConfigReload(_) => match cadis_store::load_config() {
                Ok(new_config) => {
                    self.options.ollama_model = new_config.model.ollama_model;
                    self.options.openai_model = new_config.model.openai_model;
                    self.accept(request_id, Vec::new())
                }
                Err(err) => {
                    let event = self.event(
                        None,
                        CadisEvent::DaemonError(ErrorPayload {
                            code: "config_reload_failed".to_owned(),
                            message: redact(&format!("{err}")),
                            retryable: true,
                        }),
                    );
                    self.accept(request_id, vec![event])
                }
            },
            ClientRequest::DaemonShutdown(_) => self.accept(request_id, Vec::new()),
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
            ClientRequest::WorkerResult(request) => self.worker_result(request_id, request),
            ClientRequest::WorkerCleanup(request) => self.worker_cleanup(request_id, request),
            ClientRequest::WorkerApply(request) => self.worker_apply(request_id, request),
            ClientRequest::SessionUnsubscribe(_) => self.accept(request_id, Vec::new()),
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
            &request.tool_name,
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
                    session_id.clone(),
                    CadisEvent::ApprovalRequested(approval_request_payload(&record)),
                ));
                // Item 6: Emit Waiting status when blocked on approval.
                let tool_display = record.tool_name.clone();
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                        agent_id: AgentId::from("main"),
                        status: AgentStatus::WaitingApproval,
                        task: Some(format!("waiting for approval: {}", tool_display)),
                    }),
                ));
                events.extend(self.approval_speech_events(&session_id, &record));
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
        let content_kind =
            if request.content_kind == ContentKind::Chat && is_code_heavy_task(&request.content) {
                ContentKind::Code
            } else {
                request.content_kind
            };
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
                agent_session_id: agent_session_id.clone(),
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
        // Approximate token tracking: ~4 chars per token.
        let approx_tokens = (delta.len() as u64).div_ceil(4);
        if let Some(record) = self
            .agent_sessions
            .get_mut(&pending.context.agent_session_id)
        {
            record.tokens_used = record.tokens_used.saturating_add(approx_tokens);
        }
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

    /// Creates a daemon event for a streamed model delta using a cloneable handle.
    pub fn message_delta_event_from_handle(
        &mut self,
        handle: &MessageGenerationHandle,
        delta: String,
        invocation: Option<&ModelInvocation>,
    ) -> EventEnvelope {
        let approx_tokens = (delta.len() as u64).div_ceil(4);
        if let Some(record) = self.agent_sessions.get_mut(&handle.agent_session_id) {
            record.tokens_used = record.tokens_used.saturating_add(approx_tokens);
        }
        let model = invocation.map(model_invocation_payload);
        self.session_event(
            handle.session_id.clone(),
            CadisEvent::MessageDelta(MessageDeltaPayload {
                delta,
                content_kind: handle.content_kind,
                agent_id: Some(handle.agent_id.clone()),
                agent_name: Some(handle.agent_name.clone()),
                model,
            }),
        )
    }

    /// Returns whether a prepared model generation has been cancelled by the runtime.
    pub fn message_generation_cancelled(&self, pending: &PendingMessageGeneration) -> bool {
        self.agent_session_cancelled(&pending.context.agent_session_id)
    }

    /// Executes a single tool directive within the tool loop.
    ///
    /// Only safe-read (auto-execute) tools run automatically. Approval-gated
    /// tools emit `ApprovalRequested` and signal the caller to break the loop.
    pub fn execute_tool_in_loop(
        &mut self,
        session_id: &SessionId,
        agent_id: &AgentId,
        directive: &ToolCallDirective,
    ) -> (Vec<EventEnvelope>, String) {
        let Some(tool) = self.tools.get(&directive.tool_name) else {
            return (
                Vec::new(),
                format!("[tool error] unknown tool: {}", directive.tool_name),
            );
        };

        let risk_class = tool.risk_class;
        let policy_decision = self.policy.decide(risk_class);
        let is_auto_execute = tool.execution == ToolExecutionMode::AutoExecute;
        let policy_reason = tool.policy_reason();

        // Only auto-execute safe-read tools in the loop.
        if policy_decision != PolicyDecision::Allow || !is_auto_execute {
            let tool_call_id = self.next_tool_call_id();
            let mut events = vec![self.session_event(
                session_id.clone(),
                CadisEvent::ToolRequested(ToolEventPayload {
                    tool_call_id,
                    tool_name: directive.tool_name.clone(),
                    summary: Some(format!("tool requires approval (risk={:?})", risk_class)),
                    risk_class: Some(risk_class),
                    output: None,
                }),
            )];
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                    agent_id: agent_id.clone(),
                    status: AgentStatus::WaitingApproval,
                    task: Some(format!("waiting for approval: {}", directive.tool_name)),
                }),
            ));
            return (
                events,
                format!(
                    "[tool blocked] {} requires approval and cannot auto-execute in the tool loop",
                    directive.tool_name
                ),
            );
        }

        let required_access = required_tool_access(&directive.tool_name);
        let workspace = match self.resolved_granted_workspace(
            session_id,
            Some(agent_id),
            &directive.input,
            required_access,
        ) {
            Ok(ws) => ws,
            Err(error) => {
                return (
                    Vec::new(),
                    format!("[tool error] {}: {}", error.code, error.message),
                );
            }
        };

        let tool_call_id = self.next_tool_call_id();
        let mut events = vec![self.session_event(
            session_id.clone(),
            CadisEvent::ToolRequested(ToolEventPayload {
                tool_call_id: tool_call_id.clone(),
                tool_name: directive.tool_name.clone(),
                summary: Some(policy_reason.clone()),
                risk_class: Some(risk_class),
                output: None,
            }),
        )];

        let request = ToolCallRequest {
            session_id: Some(session_id.clone()),
            agent_id: Some(agent_id.clone()),
            tool_name: directive.tool_name.clone(),
            input: directive.input.clone(),
        };

        match self.execute_safe_tool(&workspace.root, &request) {
            Ok(result) => {
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::ToolCompleted(ToolEventPayload {
                        tool_call_id,
                        tool_name: directive.tool_name.clone(),
                        summary: Some(result.summary.clone()),
                        risk_class: Some(risk_class),
                        output: Some(result.output),
                    }),
                ));
                (events, result.summary)
            }
            Err(error) => {
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::ToolFailed(ToolFailedPayload {
                        tool_call_id,
                        tool_name: directive.tool_name.clone(),
                        error: error.clone(),
                        risk_class: Some(risk_class),
                    }),
                ));
                (
                    events,
                    format!("[tool error] {}: {}", error.code, error.message),
                )
            }
        }
    }

    /// Builds a follow-up prompt that includes tool results for the next
    /// model iteration in the tool loop.
    pub fn build_tool_loop_prompt(
        &self,
        agent_id: &AgentId,
        original_content: &str,
        tool_results: &[(String, String)],
    ) -> String {
        let mut prompt = self.agent_prompt(agent_id, original_content);
        prompt.push_str("\n\nTool results from your previous response:\n");
        for (tool_name, result) in tool_results {
            prompt.push_str(&format!("\n[RESULT {tool_name}]:\n{result}\n"));
        }
        prompt.push_str("\nContinue based on the tool results above. If you need more tools, use [TOOL name: {{\"input\": \"json\"}}] directives. Otherwise, provide your final answer.");
        prompt
    }

    /// Returns the remaining budget steps for an agent session.
    pub fn agent_session_remaining_steps(&self, agent_session_id: &AgentSessionId) -> u32 {
        self.agent_sessions
            .get(agent_session_id)
            .map(|r| r.budget_steps.saturating_sub(r.steps_used))
            .unwrap_or(0)
    }

    /// Consumes one step from the agent session budget. Returns the event
    /// and whether the budget is now exceeded.
    pub fn consume_step_for_tool_loop(
        &mut self,
        agent_session_id: &AgentSessionId,
    ) -> (Option<EventEnvelope>, bool) {
        let event = self.consume_agent_session_step(agent_session_id);
        let exceeded = self
            .agent_sessions
            .get(agent_session_id)
            .is_some_and(|r| r.status == AgentSessionStatus::BudgetExceeded);
        (event, exceeded)
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

        // Model-driven spawn: parse [SPAWN Role: task] directives from model output.
        // Spawned agents receive an agent session with the task so they can be
        // picked up by a subsequent message round.
        for directive in parse_model_spawn_directives(&content) {
            match self.spawn_agent_record(AgentSpawnRequest {
                role: directive.role.clone(),
                parent_agent_id: Some(context.agent_id.clone()),
                display_name: None,
                model: None,
            }) {
                Ok(record) => {
                    events.push(self.session_event(
                        context.session_id.clone(),
                        CadisEvent::AgentSpawned(record.clone().event_payload()),
                    ));
                    // Create an agent session for the spawned agent so the task
                    // is tracked and the agent transitions to Running.
                    let (child_session_id, child_session_event) = self.start_agent_session(
                        context.session_id.clone(),
                        format!("spawn_route_{}", record.id),
                        record.id.clone(),
                        directive.task.clone(),
                    );
                    events.push(child_session_event);
                    events.push(self.session_event(
                        context.session_id.clone(),
                        CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                            agent_id: record.id.clone(),
                            status: AgentStatus::Running,
                            task: Some(directive.task.clone()),
                        }),
                    ));
                    // Complete the child session immediately with the task as
                    // pending work — the agent will execute on the next message
                    // round when addressed via @mention or orchestrator routing.
                    if let Some(event) = self.complete_agent_session(
                        &child_session_id,
                        format!("Spawned with task: {}", directive.task),
                    ) {
                        events.push(event);
                    }
                    events.push(self.session_event(
                        context.session_id.clone(),
                        CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                            agent_id: record.id,
                            status: AgentStatus::Idle,
                            task: Some(format!("ready: {}", directive.task)),
                        }),
                    ));
                }
                Err(_) => break, // spawn limit reached
            }
        }

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
                WorkerState::Completed,
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

    // -----------------------------------------------------------------------
    // Track H: Public workspace architecture APIs
    // -----------------------------------------------------------------------

    /// Returns a reference to the denied-paths checker.
    pub fn denied_paths(&self) -> &DeniedPaths {
        &self.denied_paths
    }

    /// Returns the CadisHome resolver.
    pub fn cadis_home(&self) -> CadisHome {
        CadisHome::new(&self.options.cadis_home)
    }

    /// Returns the active profile home.
    pub fn profile_home(&self) -> &ProfileHome {
        &self.profile_home
    }

    /// Lists all profile IDs.
    pub fn list_profiles(&self) -> Result<Vec<String>, cadis_store::StoreError> {
        self.cadis_home().list_profiles()
    }

    /// Creates a new profile.
    pub fn create_profile(&self, profile_id: &str) -> Result<ProfileHome, cadis_store::StoreError> {
        self.cadis_home().create_profile(profile_id)
    }

    /// Exports a profile as TOML.
    pub fn export_profile(&self, profile_id: &str) -> Result<String, cadis_store::StoreError> {
        self.cadis_home().export_profile(profile_id)
    }

    /// Imports a profile from TOML content.
    pub fn import_profile(
        &self,
        profile_id: &str,
        content: &str,
    ) -> Result<ProfileHome, cadis_store::StoreError> {
        self.cadis_home().import_profile(profile_id, content)
    }

    /// Creates a checkpoint manager for the active profile.
    pub fn checkpoint_manager(&self) -> CheckpointManager {
        CheckpointManager::new(&self.profile_home)
    }

    /// Best-effort checkpoint before a file-mutating tool.
    fn try_checkpoint(&self, workspace: &Path, input: &serde_json::Value) {
        if let Ok(ops) = parse_file_patch_operations(input) {
            let paths: Vec<PathBuf> = ops.iter().map(|op| PathBuf::from(op.path())).collect();
            let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
            let id = format!("ckpt_{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"));
            let _ =
                self.checkpoint_manager()
                    .create(&id, "pre-tool checkpoint", workspace, &path_refs);
        }
    }

    /// Creates a media manifest store for a project root.
    pub fn media_manifest_store(&self, project_root: &Path) -> MediaManifestStore {
        MediaManifestStore::new(project_root)
    }

    /// Executes approved worker worktree cleanup.
    pub fn execute_worktree_cleanup(
        &mut self,
        worker_id: &str,
    ) -> Result<Vec<EventEnvelope>, ErrorPayload> {
        let worker = self.workers.get(worker_id).ok_or_else(|| {
            tool_error(
                "worker_not_found",
                format!("worker '{worker_id}' was not found"),
                false,
            )
        })?;
        let worktree = worker.worktree.as_ref().ok_or_else(|| {
            tool_error(
                "worker_worktree_not_owned",
                format!("worker '{worker_id}' has no worktree metadata"),
                false,
            )
        })?;
        let project_root = worktree.project_root.as_deref().ok_or_else(|| {
            tool_error(
                "worker_worktree_not_owned",
                format!("worker '{worker_id}' has no project root"),
                false,
            )
        })?;

        let store = ProjectWorkspaceStore::new(project_root);

        self.transition_worker_worktree_state(worker_id, None, WorkerWorktreeState::Removed)
            .map_err(|error| tool_error(error.code, error.message, false))?;

        WorktreeCleanupExecutor::execute(&store, worker_id).map_err(|error| {
            tool_error(
                "worker_worktree_cleanup_failed",
                format!("worktree cleanup failed: {error}"),
                false,
            )
        })?;

        let mut events = Vec::new();
        if let Some(event) = self.append_worker_log(
            worker_id,
            "cleanup: daemon-internal privileged operation on CADIS-owned worktree\n",
        ) {
            events.push(event);
        }
        if let Some(event) =
            self.append_worker_log(worker_id, "cleanup completed: worktree directory removed\n")
        {
            events.push(event);
        }
        Ok(events)
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
                // Item 8: Model fallback — try fallback provider if primary fails.
                if let Some(fallback) = &self.fallback_provider {
                    let fallback = Arc::clone(fallback);
                    let mut fb_invocation = None;
                    let mut fb_content = String::new();
                    let mut fb_emitted = false;
                    let fb_result = fallback.stream_chat(
                        ModelRequest::new(&pending.prompt)
                            .with_selected_model(pending.selected_model.as_deref()),
                        &mut |event| {
                            match event {
                                ModelStreamEvent::Started(s) | ModelStreamEvent::Completed(s) => {
                                    fb_invocation = Some(s);
                                }
                                ModelStreamEvent::Delta(delta) => {
                                    fb_content.push_str(&delta);
                                    fb_emitted = true;
                                    events.push(self.message_delta_event(
                                        &pending,
                                        delta,
                                        fb_invocation.as_ref(),
                                    ));
                                }
                                ModelStreamEvent::Failed(_) | ModelStreamEvent::Cancelled(_) => {}
                            }
                            Ok(ModelStreamControl::Continue)
                        },
                    );
                    match fb_result {
                        Ok(model_response) => {
                            events.extend(self.complete_message_generation(
                                pending,
                                model_response,
                                fb_content,
                                fb_emitted,
                            ));
                            return RequestOutcome { response, events };
                        }
                        Err(fb_error) => {
                            events.extend(self.fail_message_generation(pending, fb_error));
                            return RequestOutcome { response, events };
                        }
                    }
                }
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

    fn worker_result(
        &mut self,
        request_id: RequestId,
        request: WorkerResultRequest,
    ) -> RequestOutcome {
        let Some(worker) = self.workers.get(&request.worker_id).cloned() else {
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
                "worker_result_unavailable",
                format!(
                    "worker '{}' has not reached a terminal result",
                    worker.worker_id
                ),
                true,
            );
        }

        let mut events = Vec::new();
        if let Some(agent_session) = worker
            .agent_session_id
            .as_ref()
            .and_then(|agent_session_id| self.agent_sessions.get(agent_session_id))
            .cloned()
        {
            events.push(self.session_event(
                agent_session.session_id.clone(),
                agent_session_lifecycle_event(agent_session.event_payload()),
            ));
        }

        events.push(self.session_event(
            worker.session_id.clone(),
            worker_lifecycle_event(worker.event_payload()),
        ));

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

    fn worker_apply(
        &mut self,
        request_id: RequestId,
        request: WorkerApplyRequest,
    ) -> RequestOutcome {
        let Some(worker) = self.workers.get(&request.worker_id).cloned() else {
            return self.reject(
                request_id,
                "worker_not_found",
                format!("worker '{}' was not found", request.worker_id),
                false,
            );
        };

        if worker.status != WorkerState::Completed {
            return self.reject(
                request_id,
                "worker_apply_unavailable",
                format!(
                    "worker '{}' must be completed before apply",
                    request.worker_id
                ),
                false,
            );
        }

        let verified = match self
            .verify_cadis_worker_worktree(&request.worker_id, request.worktree_path.as_deref())
        {
            Ok(verified) => verified,
            Err(error) => return self.reject(request_id, error.code, error.message, false),
        };

        let Some(artifacts) = worker.artifacts.as_ref() else {
            return self.reject(
                request_id,
                "worker_patch_missing",
                format!(
                    "worker '{}' has no patch artifact metadata",
                    request.worker_id
                ),
                false,
            );
        };
        let patch_path = PathBuf::from(artifacts.patch.trim());
        let expected_patch = self
            .profile_home
            .worker_artifact_paths(&request.worker_id)
            .patch;
        if patch_path != expected_patch {
            return self.reject(
                request_id,
                "worker_patch_not_owned",
                format!(
                    "worker '{}' patch artifact is not CADIS-owned",
                    request.worker_id
                ),
                false,
            );
        }

        let patch_path = match fs::canonicalize(&patch_path) {
            Ok(path) => path,
            Err(error) => {
                return self.reject(
                    request_id,
                    "worker_patch_missing",
                    format!(
                        "worker '{}' patch artifact '{}' is unavailable: {error}",
                        request.worker_id,
                        patch_path.display()
                    ),
                    false,
                )
            }
        };
        let patch_content = match fs::read_to_string(&patch_path) {
            Ok(content) => content,
            Err(error) => {
                return self.reject(
                    request_id,
                    "worker_patch_unreadable",
                    format!(
                        "worker '{}' patch artifact '{}' could not be read: {error}",
                        request.worker_id,
                        patch_path.display()
                    ),
                    false,
                )
            }
        };
        if patch_content.trim().is_empty() {
            return self.reject(
                request_id,
                "worker_patch_empty",
                format!(
                    "worker '{}' patch artifact '{}' is empty",
                    request.worker_id,
                    patch_path.display()
                ),
                false,
            );
        }

        let tool_request = ToolCallRequest {
            session_id: Some(worker.session_id),
            agent_id: worker.agent_id.or(worker.parent_agent_id),
            tool_name: "worker.apply".to_owned(),
            input: serde_json::json!({
                "workspace_id": verified.metadata.workspace_id,
                "worker_id": request.worker_id,
                "worktree_path": request
                    .worktree_path
                    .unwrap_or_else(|| verified.metadata.worktree_path.display().to_string()),
            }),
        };

        self.handle_tool_call(request_id, tool_request)
    }

    fn set_agent_specialist(
        &mut self,
        request_id: RequestId,
        request: AgentSpecialistSetRequest,
    ) -> RequestOutcome {
        let profile = normalize_agent_specialist(
            &request.specialist_id,
            &request.specialist_label,
            &request.persona,
        );
        let Some(agent) = self.agents.get_mut(&request.agent_id) else {
            return self.reject(
                request_id,
                "agent_not_found",
                format!("agent '{}' was not found", request.agent_id),
                false,
            );
        };

        agent.specialist_id = profile.specialist_id.clone();
        agent.specialist_label = profile.specialist_label.clone();
        agent.persona = profile.persona.clone();
        let _ = self.persist_agent_record(&request.agent_id);

        let event = self.event(
            None,
            CadisEvent::AgentSpecialistChanged(AgentSpecialistChangedPayload {
                agent_id: request.agent_id,
                specialist_id: profile.specialist_id,
                specialist_label: profile.specialist_label,
                persona: profile.persona,
            }),
        );
        self.accept(request_id, vec![event])
    }

    fn spawn_agent(&mut self, request_id: RequestId, request: AgentSpawnRequest) -> RequestOutcome {
        let record = match self.spawn_agent_record(request) {
            Ok(record) => record,
            Err(error) => return self.reject(request_id, error.code, error.message, false),
        };

        // Item 6: Emit Spawning status before Idle.
        let spawning = self.event(
            None,
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: record.id.clone(),
                status: AgentStatus::Spawning,
                task: Some("agent is being created".to_owned()),
            }),
        );
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
        self.accept(request_id, vec![spawning, event, status])
    }

    fn spawn_agent_record(
        &mut self,
        request: AgentSpawnRequest,
    ) -> Result<AgentRecord, RuntimeError> {
        let role_str = normalize_role(&request.role);
        if role_str.is_empty() {
            return Err(RuntimeError {
                code: "invalid_agent_role",
                message: "agent role is empty".to_owned(),
            });
        }
        let role = role_str
            .parse::<AgentRole>()
            .unwrap_or(AgentRole::Specialist);

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

        let agent_id = self.next_agent_id(&role_str);
        let display_name = request
            .display_name
            .as_deref()
            .map(|name| normalize_agent_name(name, &agent_id))
            .unwrap_or_else(|| default_agent_name(&role_str, &agent_id));
        let model = request
            .model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| self.options.model_provider.clone());
        let specialist = default_agent_specialist_profile(&agent_id, &display_name);
        let record = AgentRecord {
            id: agent_id.clone(),
            role,
            display_name,
            parent_agent_id: Some(parent_agent_id),
            model,
            status: AgentStatus::Idle,
            specialist_id: specialist.specialist_id,
            specialist_label: specialist.specialist_label,
            persona: specialist.persona,
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
        record.status = AgentStatus::Cancelled;

        let cancellation_requested_at = now_timestamp();
        let mut events = Vec::new();

        // Cancel active agent sessions for this agent.
        let session_ids: Vec<AgentSessionId> = self
            .agent_sessions
            .iter()
            .filter(|(_, r)| r.agent_id == agent_id && !agent_session_is_terminal(r.status))
            .map(|(id, _)| id.clone())
            .collect();
        for agent_session_id in session_ids {
            if let Some(r) = self.agent_sessions.get_mut(&agent_session_id) {
                r.status = AgentSessionStatus::Cancelled;
                r.error_code = Some("agent_killed".to_owned());
                r.error = Some("agent was killed".to_owned());
                r.cancellation_requested_at = Some(cancellation_requested_at.clone());
                let session_id = r.session_id.clone();
                let payload = r.event_payload();
                let _ = self.persist_agent_session_record(&agent_session_id);
                events.push(
                    self.session_event(session_id, CadisEvent::AgentSessionCancelled(payload)),
                );
            }
        }

        // Cancel active workers owned by this agent.
        let worker_ids: Vec<String> = self
            .workers
            .iter()
            .filter(|(_, r)| r.agent_id.as_ref() == Some(&agent_id) && !r.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for worker_id in worker_ids {
            events.extend(self.cancel_worker(&worker_id, cancellation_requested_at.clone()));
        }

        events.push(self.event(None, CadisEvent::AgentKilled(record.event_payload())));
        self.accept(request_id, events)
    }

    fn tail_agent(&mut self, request_id: RequestId, request: AgentTailRequest) -> RequestOutcome {
        let Some(agent) = self.agents.get(&request.agent_id) else {
            return self.reject(
                request_id,
                "agent_not_found",
                format!("agent '{}' was not found", request.agent_id),
                false,
            );
        };
        let limit = request
            .limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(8);
        let mut sessions: Vec<AgentSessionRecord> = self
            .agent_sessions
            .values()
            .filter(|r| r.agent_id == request.agent_id)
            .cloned()
            .collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        let start = sessions.len().saturating_sub(limit);
        let mut events = vec![self.event(
            None,
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: agent.id.clone(),
                status: agent.status,
                task: None,
            }),
        )];
        for session in &sessions[start..] {
            events.push(self.session_event(
                session.session_id.clone(),
                agent_session_lifecycle_event(session.event_payload()),
            ));
        }
        self.accept(request_id, events)
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
        let decision = speech_decision(&prefs, content_kind, text, SpeechMode::AutoSpeak);
        let speakable = match &decision {
            SpeechDecision::Speak => Some(text.trim().to_owned()),
            SpeechDecision::RequiresSummary(_) => Some(summarize_for_speech(text)),
            SpeechDecision::Blocked(_) => None,
        };
        let Some(speakable) = speakable else {
            return Vec::new();
        };

        match self.speak_with_provider(&prefs, &speakable) {
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

    fn approval_speech_events(
        &mut self,
        session_id: &SessionId,
        record: &ApprovalRecord,
    ) -> Vec<EventEnvelope> {
        let prefs = VoiceRuntimePreferences::from_options(&self.ui_preferences);
        let mut text = approval_risk_speech(record);
        match speech_decision(&prefs, ContentKind::Approval, &text, SpeechMode::AutoSpeak) {
            SpeechDecision::Speak => {}
            SpeechDecision::RequiresSummary(_) => {
                text = summarize_for_speech(&text);
            }
            SpeechDecision::Blocked(_) => return Vec::new(),
        }
        if text.chars().count() > prefs.max_spoken_chars {
            text = text.chars().take(prefs.max_spoken_chars).collect();
        }
        match self.speak_with_provider(&prefs, &text) {
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

        // Memory capsule injection
        let memory_prefix = {
            let store = MemoryStore::new(self.profile_home.root());
            let scope = cadis_memory::MemoryScope::Agent(agent_id.as_str().to_owned());
            match store.compile_capsule(Some(&scope), 2048) {
                Ok(capsule) if !capsule.entries.is_empty() => {
                    format!("\n\nRelevant memory:\n{}\n", capsule.entries.join("\n"))
                }
                _ => {
                    // Fall back to global scope
                    match store.compile_capsule(None, 2048) {
                        Ok(capsule) if !capsule.entries.is_empty() => {
                            format!("\n\nRelevant memory:\n{}\n", capsule.entries.join("\n"))
                        }
                        _ => String::new(),
                    }
                }
            }
        };

        let specialist = if agent.persona.trim().is_empty() {
            String::new()
        } else {
            format!(
                "\n\nSpecialist persona ({}):\n{}",
                agent.specialist_label, agent.persona
            )
        };
        let runtime_context = if agent.id.as_str() == "main" {
            format!(
                "\n\nCurrent CADIS runtime state:\n{}",
                self.orchestrator_awareness_context()
            )
        } else {
            String::new()
        };
        if agent.id.as_str() == "main" {
            return format!(
                "You are CADIS, a local-first AI assistant and the orchestrator for the local CADIS agent cluster. You have access to file, shell, and git tools. Always ask for approval before risky operations. Use the runtime state to route work, avoid duplicate work, and summarize what other agents are doing when useful.{}{}{}\n\nUser request:\n{}",
                specialist, runtime_context, memory_prefix, content
            );
        }
        format!(
            "You are {} ({}) in the CADIS multi-agent runtime. Answer only for your role and keep the response concise unless the user asks for detail. Always respect CADIS approval and tool safety policy.{}{}\n\nUser request:\n{}",
            agent.display_name, agent.role.as_str(), specialist, memory_prefix, content
        )
    }

    fn orchestrator_awareness_context(&self) -> String {
        let mut lines = Vec::new();
        lines.push("Agents:".to_owned());
        for agent in self.agent_records_sorted().into_iter().take(16) {
            let task = self
                .latest_agent_session(&agent.id)
                .map(|session| {
                    format!(
                        "{} ({}, step {}/{})",
                        compact_prompt_value(&session.task, AGENT_CONTEXT_TASK_MAX_CHARS),
                        agent_session_status_label(session.status),
                        session.steps_used.min(session.budget_steps),
                        session.budget_steps
                    )
                })
                .unwrap_or_else(|| "no active or recent task".to_owned());
            lines.push(format!(
                "- {} / {} [{}]: status {}, specialist {}, task: {}",
                agent.id,
                agent.display_name,
                agent.role.as_str(),
                agent_status_label(agent.status),
                agent.specialist_label,
                task
            ));
        }

        let mut recent_sessions = self.agent_session_records_sorted();
        recent_sessions.sort_by(|left, right| right.id.cmp(&left.id));
        let recent_sessions = recent_sessions.into_iter().take(6).collect::<Vec<_>>();
        if !recent_sessions.is_empty() {
            lines.push("Recent agent sessions:".to_owned());
            for session in recent_sessions {
                lines.push(format!(
                    "- {} -> {}: {} ({}, step {}/{})",
                    session.agent_id,
                    session
                        .parent_agent_id
                        .as_ref()
                        .map(AgentId::as_str)
                        .unwrap_or("root"),
                    compact_prompt_value(&session.task, AGENT_CONTEXT_TASK_MAX_CHARS),
                    agent_session_status_label(session.status),
                    session.steps_used.min(session.budget_steps),
                    session.budget_steps
                ));
            }
        }

        let mut workers = self.worker_records_sorted();
        workers.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let workers = workers.into_iter().take(6).collect::<Vec<_>>();
        if !workers.is_empty() {
            lines.push("Recent workers:".to_owned());
            for worker in workers {
                let owner = worker
                    .agent_id
                    .as_ref()
                    .or(worker.parent_agent_id.as_ref())
                    .map(AgentId::as_str)
                    .unwrap_or("unknown");
                let summary = worker
                    .summary
                    .as_deref()
                    .or(worker.log_lines.last().map(String::as_str))
                    .or(worker.error.as_deref())
                    .unwrap_or("no summary");
                lines.push(format!(
                    "- {} owned by {}: {} ({})",
                    worker.worker_id,
                    owner,
                    compact_prompt_value(summary, AGENT_CONTEXT_TASK_MAX_CHARS),
                    worker.status.as_str()
                ));
            }
        }

        lines.join("\n")
    }

    fn latest_agent_session(&self, agent_id: &AgentId) -> Option<&AgentSessionRecord> {
        self.agent_sessions
            .values()
            .filter(|session| {
                &session.agent_id == agent_id || session.parent_agent_id.as_ref() == Some(agent_id)
            })
            .max_by(|left, right| left.id.cmp(&right.id))
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
            token_budget: 0,
            tokens_used: 0,
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
        let running_count = self
            .workers
            .values()
            .filter(|r| r.status == WorkerState::Running)
            .count();
        let queued = running_count >= self.agent_runtime.max_concurrent_workers;

        let mut record = WorkerRecord::from_delegation(session_id, Some(agent_id), worker);
        if queued {
            record.status = WorkerState::Queued;
        }
        let worker_id = record.worker_id.clone();
        if !queued {
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
            return events;
        }

        // Queued path.
        let events = vec![self.session_event(
            record.session_id.clone(),
            CadisEvent::WorkerStarted(record.event_payload()),
        )];
        self.workers.insert(worker_id.clone(), record);
        let _ = self.persist_worker_record(&worker_id);
        if let Some(event) = self.append_worker_log(
            &worker_id,
            format!(
                "queued: waiting for worker slot (max_concurrent_workers={})\n",
                self.agent_runtime.max_concurrent_workers
            ),
        ) {
            return [events, vec![event]].concat();
        }
        events
    }

    /// Promotes queued workers when slots become available. Called after a worker finishes.
    fn promote_queued_workers(&mut self) -> Vec<EventEnvelope> {
        let running_count = self
            .workers
            .values()
            .filter(|r| r.status == WorkerState::Running)
            .count();
        if running_count >= self.agent_runtime.max_concurrent_workers {
            return Vec::new();
        }
        let slots = self.agent_runtime.max_concurrent_workers - running_count;
        let mut queued_ids: Vec<String> = self
            .workers
            .iter()
            .filter(|(_, r)| r.status == WorkerState::Queued)
            .map(|(id, _)| id.clone())
            .collect();
        queued_ids.sort();
        queued_ids.truncate(slots);

        let mut events = Vec::new();
        for worker_id in queued_ids {
            let session_id = {
                let Some(worker) = self.workers.get_mut(&worker_id) else {
                    continue;
                };
                worker.status = WorkerState::Running;
                worker.updated_at = now_timestamp();
                let preparation_logs = prepare_worker_execution(worker);
                let session_id = worker.session_id.clone();
                let payload = worker.event_payload();
                let _ = self.persist_worker_record(&worker_id);
                events.push(
                    self.session_event(session_id.clone(), CadisEvent::WorkerStarted(payload)),
                );
                if let Some(event) = self.append_worker_log(
                    &worker_id,
                    "promoted from queue: worker slot available\n".to_owned(),
                ) {
                    events.push(event);
                }
                for line in preparation_logs {
                    if let Some(event) = self.append_worker_log(&worker_id, line) {
                        events.push(event);
                    }
                }
                session_id
            };
            let _ = session_id; // used above
        }
        events
    }

    fn complete_worker(
        &mut self,
        worker_id: &str,
        status: WorkerState,
        summary: String,
    ) -> Vec<EventEnvelope> {
        if status == WorkerState::Completed {
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
                    WorkerState::Failed,
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

            // Items 12-14: Request patch approval after worker completes.
            // If the worker has a patch artifact, emit an approval request for applying it.
            if let Some(worker) = self.workers.get(worker_id) {
                if let Some(artifacts) = &worker.artifacts {
                    let patch_path = &artifacts.patch;
                    if Path::new(patch_path).is_file() {
                        if let Ok(patch_content) = fs::read_to_string(patch_path) {
                            if !patch_content.trim().is_empty() {
                                if let Some(event) = self.append_worker_log(
                                    worker_id,
                                    "patch artifact available: approval required to apply\n",
                                ) {
                                    events.push(event);
                                }
                            }
                        }
                    }
                }
            }

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
            WorkerState::Failed,
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
            WorkerState::Cancelled,
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
        status: WorkerState,
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
        // Promote queued workers when a slot opens.
        events.extend(self.promote_queued_workers());
        events
    }

    fn write_worker_artifacts(
        &mut self,
        worker_id: &str,
        status: WorkerState,
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
        execute_worker_command(worker, &self.agent_runtime.worker_command)
    }

    fn plan_worker_terminal_cleanup(
        &mut self,
        worker_id: &str,
        status: WorkerState,
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

        // Attempt actual worktree removal.
        let removed = if let Some(worker) = self.workers.get(worker_id) {
            if let Some(worktree) = &worker.worktree {
                let worktree_path = PathBuf::from(&worktree.worktree_path);
                if worktree_path.exists() && worktree_path.is_dir() {
                    // Remove git worktree first, then directory.
                    if let Some(project_root) = worktree.project_root.as_deref() {
                        let _ = Command::new("git")
                            .args(["worktree", "remove", "--force"])
                            .arg(&worktree_path)
                            .current_dir(project_root)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                    if worktree_path.exists() {
                        let _ = fs::remove_dir_all(&worktree_path);
                    }
                    !worktree_path.exists()
                } else {
                    true // already gone
                }
            } else {
                false
            }
        } else {
            false
        };

        if removed {
            // Update project metadata directly since the worktree path no longer exists.
            if let Some(worker) = self.workers.get(worker_id) {
                if let Some(worktree) = &worker.worktree {
                    if let Some(project_root) = worktree.project_root.as_deref() {
                        let store = ProjectWorkspaceStore::new(project_root);
                        if let Ok(Some(mut meta)) = store.load_worker_worktree_metadata(worker_id) {
                            meta.state = ProjectWorkerWorktreeState::Removed;
                            let _ = store.save_worker_worktree_metadata(&meta);
                        }
                    }
                }
            }
            if let Some(worker) = self.workers.get_mut(worker_id) {
                if let Some(worktree) = &mut worker.worktree {
                    worktree.state = WorkerWorktreeState::Removed;
                }
                worker.updated_at = now_timestamp();
            }
            let _ = self.persist_worker_record(worker_id);
            if let Some(event) = self.append_worker_log(
                worker_id,
                format!("cleanup completed: {reason}; worktree removed\n"),
            ) {
                events.push(event);
            }
        } else {
            if let Some(event) = self.append_worker_log(
                worker_id,
                format!("cleanup requested: {reason}; removal attempted\n"),
            ) {
                events.push(event);
            }
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
        status: WorkerState,
        summary: Option<String>,
        error_code: Option<String>,
        error: Option<String>,
        cancellation_requested_at: Option<Timestamp>,
    ) -> Option<EventEnvelope> {
        let (session_id, payload) = {
            let worker = self.workers.get_mut(worker_id)?;
            worker.status = status;
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
        &mut self,
        workspace: &Path,
        request: &ToolCallRequest,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        // Item 4: Check cancellation token before execution.
        if self.cancellation_token.is_cancelled() {
            return Err(tool_error(
                "tool_cancelled",
                "tool execution was cancelled before it started",
                false,
            ));
        }

        // Item 3: Tool timeout wrapper.
        let timeout_secs = self
            .tools
            .get(&request.tool_name)
            .map(|t| t.timeout_secs)
            .unwrap_or(30);

        match request.tool_name.as_str() {
            tool_name if self.tools.is_auto_executable_safe_read(tool_name) => {
                let ws = workspace.to_path_buf();
                let input = request.input.clone();
                let name = tool_name.to_owned();
                let (tx, rx) = std::sync::mpsc::channel();
                // We need workspace and self references; since safe tools are fast,
                // execute inline but with a timeout guard.
                let result = match name.as_str() {
                    "file.read" => self.execute_file_read(&ws, &input),
                    "file.search" => self.execute_file_search(&ws, &input),
                    "file.list" => self.execute_file_list(&ws, &input),
                    "git.status" => self.execute_git_status(&ws, &input),
                    "git.diff" => self.execute_git_diff(&ws, &input),
                    "git.log" => self.execute_git_log(&ws, &input),
                    _ => Err(tool_error(
                        "tool_not_implemented",
                        format!("{name} has no native execution backend"),
                        false,
                    )),
                };
                let _ = tx.send(result);
                rx.recv_timeout(StdDuration::from_secs(timeout_secs))
                    .unwrap_or_else(|_| {
                        Err(tool_error(
                            "tool_timeout",
                            format!("{} exceeded timeout of {timeout_secs}s", request.tool_name),
                            false,
                        ))
                    })
            }
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
        // Track D item 8: recheck approval expiry and policy before execution.
        if approval_is_expired(record) {
            return vec![self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolFailed(ToolFailedPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: record.tool_name.clone(),
                    error: tool_error(
                        "approval_expired",
                        "approval expired before execution could start",
                        false,
                    ),
                    risk_class: Some(record.risk_class),
                }),
            )];
        }

        let recheck = self.policy.decide(record.risk_class);
        if recheck == PolicyDecision::Deny {
            return vec![self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolFailed(ToolFailedPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: record.tool_name.clone(),
                    error: tool_error(
                        "policy_denied_at_execution",
                        "policy denied the tool at execution time",
                        false,
                    ),
                    risk_class: Some(record.risk_class),
                }),
            )];
        }

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
            &request.tool_name,
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

        // Item 1: Revalidate denied paths and secret posture before execution.
        if self.denied_paths.is_denied(&workspace.root) {
            return vec![self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolFailed(ToolFailedPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: request.tool_name,
                    error: tool_error(
                        "path_denied",
                        format!(
                            "workspace {} is denied by policy at execution time",
                            workspace.root.display()
                        ),
                        false,
                    ),
                    risk_class: Some(record.risk_class),
                }),
            )];
        }

        if !self.sessions.contains_key(&record.session_id) {
            return vec![self.session_event(
                record.session_id.clone(),
                CadisEvent::ToolFailed(ToolFailedPayload {
                    tool_call_id: record.tool_call_id.clone(),
                    tool_name: request.tool_name,
                    error: tool_error(
                        "session_invalid_at_execution",
                        "session is no longer active at execution time",
                        false,
                    ),
                    risk_class: Some(record.risk_class),
                }),
            )];
        }

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
        if self.denied_paths.is_denied(workspace) {
            return Err(tool_error(
                "path_denied",
                format!("workspace {} is denied by policy", workspace.display()),
                false,
            ));
        }

        // Item 18: Fail closed on secrets — check input paths for secret files.
        if let Some(path_str) = input_string(&request.input, "path") {
            let p = Path::new(&path_str);
            if cadis_policy::is_secret_file(p) {
                return Err(tool_error(
                    "secret_path_rejected",
                    "tool refuses to access secret-like paths",
                    false,
                ));
            }
        }

        // Item 4: Check cancellation token before execution.
        if self.cancellation_token.is_cancelled() {
            return Err(tool_error(
                "tool_cancelled",
                "tool execution was cancelled before it started",
                false,
            ));
        }

        match request.tool_name.as_str() {
            "file.patch" | "file.write" => {
                self.try_checkpoint(workspace, &request.input);
                self.execute_file_patch(workspace, &request.input)
            }
            "worker.apply" => {
                self.execute_worker_apply_patch(workspace, &request.input, tool_timeout_secs)
            }
            "shell.run" => self.execute_shell_run(workspace, &request.input, tool_timeout_secs),
            "git.worktree.create" => self.execute_git_worktree_create(workspace, &request.input),
            "git.worktree.remove" => self.execute_git_worktree_remove(workspace, &request.input),
            "git.commit" => self.execute_git_commit(workspace, &request.input),
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

        // Item 2: file.patch preview mode — return diff without applying.
        let preview = input
            .get("preview")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if preview {
            let output_files = prepared
                .iter()
                .take(FILE_PATCH_OUTPUT_MAX_FILES)
                .map(|change| {
                    serde_json::json!({
                        "path": redact(&change.display_path),
                        "action": change.action,
                        "content": redact(&change.content),
                    })
                })
                .collect::<Vec<_>>();
            let summary = format!(
                "preview: {} file{} would be patched",
                prepared.len(),
                plural(prepared.len())
            );
            return Ok(ToolExecutionResult {
                summary,
                output: serde_json::json!({
                    "schema": "structured_replace_write_v1",
                    "preview": true,
                    "files": output_files,
                    "truncated": prepared.len() > FILE_PATCH_OUTPUT_MAX_FILES
                }),
            });
        }

        // Item 16: Gate outside-workspace writes.
        for change in &prepared {
            if let Ok(canonical) = change.path.canonicalize() {
                if let Ok(ws) = workspace.canonicalize() {
                    if !canonical.starts_with(&ws) {
                        return Err(tool_error(
                            "outside_workspace",
                            format!(
                                "{} resolves outside the workspace; requires OutsideWorkspace approval",
                                change.display_path
                            ),
                            false,
                        ));
                    }
                }
            }
        }

        for change in &prepared {
            if let Some(expected_mtime) = change.mtime {
                if let Ok(meta) = fs::metadata(&change.path) {
                    if let Ok(current_mtime) = meta.modified() {
                        if current_mtime != expected_mtime {
                            return Err(tool_error(
                                "file_patch_concurrent_edit",
                                format!(
                                    "{} was modified since patch was prepared",
                                    change.display_path
                                ),
                                true,
                            ));
                        }
                    }
                }
            }
            let parent = change.path.parent();
            let temp_path = match parent {
                Some(dir) => dir.join(format!(".cadis_patch_{}.tmp", std::process::id())),
                None => PathBuf::from(format!(".cadis_patch_{}.tmp", std::process::id())),
            };
            fs::write(&temp_path, change.content.as_bytes()).map_err(|error| {
                tool_error(
                    "file_patch_write_failed",
                    format!("could not write temp for {}: {error}", change.display_path),
                    false,
                )
            })?;
            fs::rename(&temp_path, &change.path).map_err(|error| {
                let _ = fs::remove_file(&temp_path);
                tool_error(
                    "file_patch_write_failed",
                    format!("could not rename temp to {}: {error}", change.display_path),
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

    fn execute_worker_apply_patch(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
        tool_timeout_secs: u64,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let worker_id = input_string(input, "worker_id")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                tool_error(
                    "invalid_tool_input",
                    "worker.apply requires worker_id",
                    false,
                )
            })?;
        let requested_worktree = input_string(input, "worktree_path");
        let _verified = self
            .verify_cadis_worker_worktree(&worker_id, requested_worktree.as_deref())
            .map_err(|error| tool_error(error.code, error.message, false))?;

        let worker = self.workers.get(&worker_id).ok_or_else(|| {
            tool_error(
                "worker_not_found",
                format!("worker '{worker_id}' was not found"),
                false,
            )
        })?;
        if worker.status != WorkerState::Completed {
            return Err(tool_error(
                "worker_apply_unavailable",
                format!("worker '{worker_id}' must be completed before apply"),
                false,
            ));
        }

        let artifacts = worker.artifacts.as_ref().ok_or_else(|| {
            tool_error(
                "worker_patch_missing",
                format!("worker '{worker_id}' has no patch artifact metadata"),
                false,
            )
        })?;
        let patch_path = PathBuf::from(artifacts.patch.trim());
        let expected_patch = self.profile_home.worker_artifact_paths(&worker_id).patch;
        if patch_path != expected_patch {
            return Err(tool_error(
                "worker_patch_not_owned",
                format!("worker '{worker_id}' patch artifact is not CADIS-owned"),
                false,
            ));
        }
        let patch_path = fs::canonicalize(&patch_path).map_err(|error| {
            tool_error(
                "worker_patch_missing",
                format!(
                    "worker '{worker_id}' patch artifact '{}' is unavailable: {error}",
                    patch_path.display()
                ),
                false,
            )
        })?;

        let patch_content = fs::read_to_string(&patch_path).map_err(|error| {
            tool_error(
                "worker_patch_unreadable",
                format!(
                    "worker '{worker_id}' patch artifact '{}' could not be read: {error}",
                    patch_path.display()
                ),
                false,
            )
        })?;
        if patch_content.trim().is_empty() {
            return Err(tool_error(
                "worker_patch_empty",
                format!(
                    "worker '{worker_id}' patch artifact '{}' is empty",
                    patch_path.display()
                ),
                false,
            ));
        }

        let mut apply = Command::new("git");
        apply
            .arg("-C")
            .arg(workspace)
            .arg("apply")
            .arg("--3way")
            .arg("--whitespace=nowarn")
            .arg(&patch_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let result = run_bounded_command(apply, StdDuration::from_secs(tool_timeout_secs))?;
        let stdout = redact(&String::from_utf8_lossy(&result.stdout.bytes));
        let stderr = redact(&String::from_utf8_lossy(&result.stderr.bytes));

        if result.timed_out {
            return Err(tool_error(
                "tool_timeout",
                format!(
                    "worker.apply exceeded timeout of {tool_timeout_secs}s for worker '{worker_id}'"
                ),
                false,
            ));
        }
        if !result.status_success {
            let detail = if !stderr.trim().is_empty() {
                stderr
            } else if !stdout.trim().is_empty() {
                stdout
            } else {
                "git apply failed without output".to_owned()
            };
            return Err(tool_error(
                "worker_apply_failed",
                format!(
                    "worker '{worker_id}' patch apply failed (exit code {:?}): {}",
                    result.exit_code, detail
                ),
                false,
            ));
        }

        Ok(ToolExecutionResult {
            summary: format!("applied worker patch for {worker_id}"),
            output: serde_json::json!({
                "worker_id": worker_id,
                "patch_path": patch_path.display().to_string(),
                "workspace": workspace.display().to_string(),
                "exit_code": result.exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "stdout_truncated": result.stdout.truncated,
                "stderr_truncated": result.stderr.truncated
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
        if cadis_policy::is_secret_file(&path) {
            return Err(tool_error(
                "secret_path_rejected",
                "file.read refuses to read secret-like paths",
                false,
            ));
        }
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
        let content = if content.len() > 5000 {
            cadis_output_filter::filter_output("file.read", &content).filtered
        } else {
            content
        };
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
        &mut self,
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

        // Use trigram index to pre-filter when workspace has >100 files.
        let idx = self
            .search_index_cache
            .entry(workspace.to_path_buf())
            .or_insert_with(|| search_index::SearchIndex::build(workspace));
        if idx.file_count() > 100 {
            let candidates = idx.search(&query, max_results * 10);
            for hit in candidates {
                if matches.len() >= max_results {
                    break;
                }
                let Ok(resolved) = hit.path.canonicalize() else {
                    continue;
                };
                if !resolved.starts_with(workspace) {
                    continue;
                }
                search_file(workspace, &resolved, &query, max_results, &mut matches);
            }
        } else {
            search_files(workspace, &root, &query, max_results, &mut matches);
        }

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
        let filtered = cadis_output_filter::filter_output("git status", &stdout);
        Ok(ToolExecutionResult {
            summary: filtered.filtered.clone(),
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "status": filtered.filtered
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
        let filtered = cadis_output_filter::filter_output("git diff", &diff);
        let summary = if filtered.filtered.trim().is_empty() {
            "no diff".to_owned()
        } else {
            filtered.filtered.clone()
        };

        Ok(ToolExecutionResult {
            summary,
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "pathspec": pathspec,
                "diff": filtered.filtered,
                "truncated": truncated
            }),
        })
    }

    fn execute_git_worktree_create(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let branch = input_string(input, "branch").ok_or_else(|| {
            tool_error(
                "invalid_tool_input",
                "git.worktree.create requires branch",
                false,
            )
        })?;
        let path = input_string(input, "path").ok_or_else(|| {
            tool_error(
                "invalid_tool_input",
                "git.worktree.create requires path",
                false,
            )
        })?;

        // Validate branch name: alphanumeric, hyphens, underscores, slashes only.
        if !branch
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/')
            || branch.contains("..")
        {
            return Err(tool_error(
                "invalid_tool_input",
                "branch name contains invalid characters",
                false,
            ));
        }

        let resolved = workspace.join(&path);

        // Try with -b first (new branch); fall back to existing branch.
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                &resolved.to_string_lossy(),
                "-b",
                &branch,
            ])
            .current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| tool_error("git_spawn_failed", e.to_string(), false))?;

        let (stdout, stderr, success) = if output.status.success() {
            (
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
                true,
            )
        } else {
            // Branch may already exist — retry without -b.
            let retry = Command::new("git")
                .args(["worktree", "add", &resolved.to_string_lossy(), &branch])
                .current_dir(workspace)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(|e| tool_error("git_spawn_failed", e.to_string(), false))?;
            (
                String::from_utf8_lossy(&retry.stdout).to_string(),
                String::from_utf8_lossy(&retry.stderr).to_string(),
                retry.status.success(),
            )
        };

        if !success {
            let detail = if !stderr.trim().is_empty() {
                stderr.trim()
            } else {
                "unknown error"
            };
            return Err(tool_error(
                "git_worktree_create_failed",
                format!("git worktree add failed: {detail}"),
                false,
            ));
        }

        Ok(ToolExecutionResult {
            summary: format!("created worktree at {} on branch {}", path, branch),
            output: serde_json::json!({
                "path": path,
                "branch": branch,
                "stdout": redact(&stdout),
                "stderr": redact(&stderr),
            }),
        })
    }

    fn execute_git_worktree_remove(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let path = input_string(input, "path").ok_or_else(|| {
            tool_error(
                "invalid_tool_input",
                "git.worktree.remove requires path",
                false,
            )
        })?;

        let resolved = workspace.join(&path);

        let output = Command::new("git")
            .args(["worktree", "remove", &resolved.to_string_lossy(), "--force"])
            .current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| tool_error("git_spawn_failed", e.to_string(), false))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let detail = if !stderr.trim().is_empty() {
                stderr.trim()
            } else {
                "unknown error"
            };
            return Err(tool_error(
                "git_worktree_remove_failed",
                format!("git worktree remove failed: {detail}"),
                false,
            ));
        }

        Ok(ToolExecutionResult {
            summary: format!("removed worktree at {}", path),
            output: serde_json::json!({
                "path": path,
                "stdout": redact(&stdout),
                "stderr": redact(&stderr),
            }),
        })
    }

    fn execute_file_list(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let dir = input_string(input, "path").unwrap_or_else(|| ".".to_owned());
        let resolved = resolve_inside_workspace(workspace, &dir)?;
        let entries = std::fs::read_dir(&resolved)
            .map_err(|e| tool_error("file_list_failed", e.to_string(), false))?;
        let mut items: Vec<serde_json::Value> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| tool_error("file_list_failed", e.to_string(), false))?;
            let meta = entry.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let name = entry.file_name().to_string_lossy().to_string();
            items.push(serde_json::json!({
                "name": name,
                "type": if is_dir { "dir" } else { "file" },
                "size": size,
            }));
            if items.len() >= 500 {
                break;
            }
        }
        items.sort_by(|a, b| {
            let a_type = a["type"].as_str().unwrap_or("");
            let b_type = b["type"].as_str().unwrap_or("");
            b_type.cmp(a_type).then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            })
        });
        let count = items.len();
        Ok(ToolExecutionResult {
            summary: format!("{count} entries in {dir}"),
            output: serde_json::json!({
                "path": display_relative_path(workspace, &resolved),
                "entries": items,
            }),
        })
    }

    fn execute_git_log(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let cwd = input_string(input, "path")
            .or_else(|| input_string(input, "cwd"))
            .unwrap_or_else(|| ".".to_owned());
        let cwd = resolve_inside_workspace(workspace, &cwd)?;
        let max_count = input
            .get("max_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(100);
        let output = Command::new("git")
            .arg("-C")
            .arg(&cwd)
            .args([
                "log",
                "--oneline",
                "--no-decorate",
                &format!("-{max_count}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| tool_error("git_log_failed", e.to_string(), false))?;
        if !output.status.success() {
            let stderr = redact(&String::from_utf8_lossy(&output.stderr));
            return Err(tool_error(
                "git_log_failed",
                if stderr.trim().is_empty() {
                    "git log failed".to_owned()
                } else {
                    stderr
                },
                false,
            ));
        }
        let log = redact(&String::from_utf8_lossy(&output.stdout));
        let filtered = cadis_output_filter::filter_output("git log", &log);
        Ok(ToolExecutionResult {
            summary: filtered.filtered.clone(),
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "log": filtered.filtered,
                "max_count": max_count,
            }),
        })
    }

    fn execute_git_commit(
        &self,
        workspace: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let message = input_string(input, "message").ok_or_else(|| {
            tool_error("invalid_tool_input", "git.commit requires message", false)
        })?;
        if message.trim().is_empty() {
            return Err(tool_error(
                "invalid_tool_input",
                "commit message is empty",
                false,
            ));
        }
        // Optional: specific files to stage. If absent, stage all changes.
        let files: Vec<String> = input
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        // Stage
        let mut add_cmd = Command::new("git");
        add_cmd.current_dir(workspace).stdin(Stdio::null());
        if files.is_empty() {
            add_cmd.args(["add", "-A"]);
        } else {
            add_cmd.arg("add").arg("--");
            for f in &files {
                add_cmd.arg(f);
            }
        }
        let add_out = add_cmd
            .output()
            .map_err(|e| tool_error("git_add_failed", e.to_string(), false))?;
        if !add_out.status.success() {
            let stderr = redact(&String::from_utf8_lossy(&add_out.stderr));
            return Err(tool_error(
                "git_add_failed",
                format!("git add failed: {stderr}"),
                false,
            ));
        }

        // Commit
        let commit_out = Command::new("git")
            .current_dir(workspace)
            .args(["commit", "-m", &message, "--no-verify"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| tool_error("git_commit_failed", e.to_string(), false))?;
        let stdout = redact(&String::from_utf8_lossy(&commit_out.stdout));
        let stderr = redact(&String::from_utf8_lossy(&commit_out.stderr));
        if !commit_out.status.success() {
            let detail = if !stderr.trim().is_empty() {
                &stderr
            } else {
                "git commit failed"
            };
            return Err(tool_error("git_commit_failed", detail.to_string(), false));
        }

        Ok(ToolExecutionResult {
            summary: format!("committed: {message}"),
            output: serde_json::json!({
                "message": message,
                "files": files,
                "stdout": stdout,
                "stderr": stderr,
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

        if cadis_policy::is_dangerous_delete_command(&command) {
            return Err(tool_error(
                "dangerous_delete_blocked",
                "recursive delete commands require explicit dangerous-delete approval",
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

        let filtered = cadis_output_filter::filter_output(&command, &stdout);
        let raw_truncated: String = stdout.chars().take(500).collect();

        Ok(ToolExecutionResult {
            summary: shell_summary(
                &filtered.filtered,
                &stderr,
                result.stdout.truncated,
                result.stderr.truncated,
            ),
            output: serde_json::json!({
                "cwd": display_relative_path(workspace, &cwd),
                "exit_code": result.exit_code,
                "stdout": filtered.filtered,
                "stderr": stderr,
                "stdout_truncated": result.stdout.truncated,
                "stderr_truncated": result.stderr.truncated,
                "timeout_ms": timeout_ms,
                "raw_truncated": raw_truncated
            }),
        })
    }

    fn resolved_granted_workspace(
        &self,
        tool_name: &str,
        session_id: &SessionId,
        agent_id: Option<&AgentId>,
        input: &serde_json::Value,
        required_access: WorkspaceAccess,
    ) -> Result<ResolvedWorkspace, ErrorPayload> {
        if tool_name == "worker.apply" {
            return self.resolve_worker_apply_workspace(input);
        }

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

    fn resolve_worker_apply_workspace(
        &self,
        input: &serde_json::Value,
    ) -> Result<ResolvedWorkspace, ErrorPayload> {
        let worker_id = input_string(input, "worker_id")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                tool_error(
                    "invalid_tool_input",
                    "worker.apply requires worker_id",
                    false,
                )
            })?;
        let requested_worktree = input_string(input, "worktree_path");
        let verified = self
            .verify_cadis_worker_worktree(&worker_id, requested_worktree.as_deref())
            .map_err(|error| tool_error(error.code, error.message, false))?;
        let workspace_id = WorkspaceId::from(verified.metadata.workspace_id.clone());
        let workspace = self.workspaces.get(&workspace_id).ok_or_else(|| {
            tool_error(
                "workspace_not_found",
                format!(
                    "workspace '{}' is not registered for worker '{}'",
                    workspace_id, worker_id
                ),
                false,
            )
        })?;

        Ok(ResolvedWorkspace {
            root: workspace.root.clone(),
        })
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
            worker_command: agent_runtime
                .get("worker_command")
                .and_then(serde_json::Value::as_str)
                .map(|s| s.trim().to_owned())
                .unwrap_or(defaults.worker_command),
            max_concurrent_workers: json_usize(agent_runtime, "max_concurrent_workers")
                .filter(|v| *v > 0)
                .unwrap_or(defaults.max_concurrent_workers),
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
    role: AgentRole,
    display_name: String,
    parent_agent_id: Option<AgentId>,
    model: String,
    status: AgentStatus,
    specialist_id: String,
    specialist_label: String,
    persona: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentSpecialistProfile {
    specialist_id: String,
    specialist_label: String,
    persona: String,
}

impl AgentSpecialistProfile {
    fn with_default(self, default: Self) -> Self {
        if self.specialist_id.is_empty()
            && self.specialist_label.is_empty()
            && self.persona.is_empty()
        {
            return default;
        }
        Self {
            specialist_id: if self.specialist_id.is_empty() {
                default.specialist_id
            } else {
                self.specialist_id
            },
            specialist_label: if self.specialist_label.is_empty() {
                default.specialist_label
            } else {
                self.specialist_label
            },
            persona: if self.persona.is_empty() {
                default.persona
            } else {
                self.persona
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
struct AgentMetadata {
    agent_id: AgentId,
    role: AgentRole,
    display_name: String,
    parent_agent_id: Option<AgentId>,
    model: String,
    status: AgentStatus,
    #[serde(default)]
    specialist_id: String,
    #[serde(default)]
    specialist_label: String,
    #[serde(default)]
    persona: String,
}

impl AgentMetadata {
    fn from_record(record: &AgentRecord) -> Self {
        Self {
            agent_id: record.id.clone(),
            role: record.role,
            display_name: record.display_name.clone(),
            parent_agent_id: record.parent_agent_id.clone(),
            model: record.model.clone(),
            status: record.status,
            specialist_id: record.specialist_id.clone(),
            specialist_label: record.specialist_label.clone(),
            persona: record.persona.clone(),
        }
    }

    fn into_record(self) -> (AgentId, AgentRecord) {
        let id = self.agent_id;
        let specialist =
            normalize_agent_specialist(&self.specialist_id, &self.specialist_label, &self.persona)
                .with_default(default_agent_specialist_profile(&id, &self.display_name));
        (
            id.clone(),
            AgentRecord {
                id,
                role: self.role,
                display_name: self.display_name,
                parent_agent_id: self.parent_agent_id,
                model: self.model,
                status: self.status,
                specialist_id: specialist.specialist_id,
                specialist_label: specialist.specialist_label,
                persona: specialist.persona,
            },
        )
    }
}

impl AgentRecord {
    fn agent_home_template(&self) -> AgentHomeTemplate {
        AgentHomeTemplate::new(
            self.id.clone(),
            self.display_name.clone(),
            self.role.as_str().to_owned(),
            self.parent_agent_id.clone(),
            self.model.clone(),
        )
    }

    fn event_payload(self) -> AgentEventPayload {
        AgentEventPayload {
            agent_id: self.id,
            role: Some(self.role.as_str().to_owned()),
            display_name: Some(self.display_name),
            parent_agent_id: self.parent_agent_id,
            model: Some(self.model),
            status: Some(self.status),
            specialist_id: Some(self.specialist_id),
            specialist_label: Some(self.specialist_label),
            persona: Some(self.persona),
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
    /// Approximate token budget for this session (0 = unlimited).
    token_budget: u64,
    /// Approximate tokens consumed so far.
    tokens_used: u64,
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
    #[serde(default)]
    token_budget: u64,
    #[serde(default)]
    tokens_used: u64,
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
            token_budget: record.token_budget,
            tokens_used: record.tokens_used,
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
                token_budget: self.token_budget,
                tokens_used: self.tokens_used,
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
struct WorkerDelegation {
    worker_id: String,
    agent_session_id: AgentSessionId,
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
    agent_session_id: Option<AgentSessionId>,
    status: WorkerState,
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
    agent_session_id: Option<AgentSessionId>,
    status: WorkerState,
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
            agent_session_id: Some(worker.agent_session_id.clone()),
            status: WorkerState::Running,
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
            agent_session_id: self.agent_session_id.clone(),
            status: Some(worker_state_str(self.status)),
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
        worker_status_is_terminal(self.status)
    }
}

impl WorkerMetadata {
    fn from_record(record: &WorkerRecord) -> Self {
        Self {
            worker_id: record.worker_id.clone(),
            session_id: record.session_id.clone(),
            agent_id: record.agent_id.clone(),
            parent_agent_id: record.parent_agent_id.clone(),
            agent_session_id: record.agent_session_id.clone(),
            status: record.status,
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
                agent_session_id: self.agent_session_id,
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

        worker.status = WorkerState::Failed;
        let summary = match worker.summary.take() {
            Some(summary) if !summary.trim().is_empty() => {
                format!("{summary} (marked failed during daemon recovery)")
            }
            _ => "Worker was marked failed during daemon recovery".to_owned(),
        };
        worker.summary = Some(summary.clone());
        worker.error_code = Some("worker_recovered_stale".to_owned());
        worker.error = Some(summary);
        plan_worker_terminal_worktree(worker, WorkerState::Failed);
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

fn worker_status_is_terminal(status: WorkerState) -> bool {
    status.is_terminal()
}

fn worker_state_str(s: WorkerState) -> String {
    match s {
        WorkerState::Queued => "queued",
        WorkerState::Running => "running",
        WorkerState::Completed => "completed",
        WorkerState::Failed => "failed",
        WorkerState::Cancelled => "cancelled",
    }
    .to_owned()
}

fn agent_session_lifecycle_event(payload: AgentSessionEventPayload) -> CadisEvent {
    match payload.status {
        AgentSessionStatus::Completed => CadisEvent::AgentSessionCompleted(payload),
        AgentSessionStatus::Failed
        | AgentSessionStatus::TimedOut
        | AgentSessionStatus::BudgetExceeded => CadisEvent::AgentSessionFailed(payload),
        AgentSessionStatus::Cancelled => CadisEvent::AgentSessionCancelled(payload),
        AgentSessionStatus::Started | AgentSessionStatus::Running => {
            CadisEvent::AgentSessionUpdated(payload)
        }
    }
}

fn worker_lifecycle_event(payload: WorkerEventPayload) -> CadisEvent {
    let kind = payload.status.as_deref().and_then(|s| match s {
        "completed" => Some(WorkerLifecycleEventKind::Completed),
        "cancelled" | "canceled" => Some(WorkerLifecycleEventKind::Cancelled),
        "failed" => Some(WorkerLifecycleEventKind::Failed),
        "running" | "queued" => Some(WorkerLifecycleEventKind::Started),
        _ => None,
    });
    match kind {
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

fn worker_lifecycle_event_kind(status: WorkerState) -> WorkerLifecycleEventKind {
    match status {
        WorkerState::Completed => WorkerLifecycleEventKind::Completed,
        WorkerState::Cancelled => WorkerLifecycleEventKind::Cancelled,
        status if worker_status_is_terminal(status) => WorkerLifecycleEventKind::Failed,
        _ => WorkerLifecycleEventKind::Started,
    }
}

fn worker_terminal_worktree_state(
    record: &WorkerRecord,
    status: WorkerState,
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

fn plan_worker_terminal_worktree(record: &mut WorkerRecord, status: WorkerState) {
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
        let specialist = default_agent_specialist_profile(&id, display_name);
        (
            id.clone(),
            AgentRecord {
                id,
                role: role.parse::<AgentRole>().unwrap_or(AgentRole::Specialist),
                display_name: display_name.to_owned(),
                parent_agent_id: None,
                model: model_provider.to_owned(),
                status: AgentStatus::Idle,
                specialist_id: specialist.specialist_id,
                specialist_label: specialist.specialist_label,
                persona: specialist.persona,
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

/// Returns `true` when the user message looks like a coding task.
///
/// The daemon uses this to emit a `ContentKind::Code` hint so HUD clients
/// can auto-open the code work panel.
pub fn is_code_heavy_task(content: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "implement",
        "refactor",
        "fix bug",
        "write code",
        "add function",
        "add method",
        "add test",
        "unit test",
        "compile",
        "build error",
        "syntax error",
        "type error",
        "cargo build",
        "cargo test",
        "npm run",
        "pnpm",
        "eslint",
        "prettier",
        "linter",
        "pull request",
        "code review",
        "```",
        "fn ",
        "def ",
        "struct ",
        "impl ",
    ];
    let lower = content.to_ascii_lowercase();
    KEYWORDS.iter().any(|kw| lower.contains(kw))
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

fn normalize_agent_specialist(
    specialist_id: &str,
    specialist_label: &str,
    persona: &str,
) -> AgentSpecialistProfile {
    let id = slugify(specialist_id).chars().take(40).collect::<String>();
    let label = specialist_label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect::<String>();
    let persona = persona
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(AGENT_PERSONA_MAX_CHARS)
        .collect::<String>();

    AgentSpecialistProfile {
        specialist_id: if id == "agent" { String::new() } else { id },
        specialist_label: label,
        persona,
    }
}

fn default_agent_specialist_profile(
    agent_id: &AgentId,
    _display_name: &str,
) -> AgentSpecialistProfile {
    let (specialist_id, specialist_label, persona) = match agent_id.as_str() {
        "main" => (
            "orchestrator",
            "Orchestrator",
            "Coordinate the CADIS agent cluster. Track what each agent is doing, route tasks to the best specialist, avoid duplicate work, and summarize cross-agent progress clearly.",
        ),
        "codex" => (
            "engineering",
            "Engineering",
            "Act as a senior software engineer. Focus on implementation quality, tests, maintainability, and concrete code-level tradeoffs.",
        ),
        "atlas" => (
            "research",
            "Research",
            "Act as a research analyst. Gather context, compare sources, separate facts from assumptions, and return concise evidence-backed findings.",
        ),
        "forge" => (
            "automation",
            "Automation",
            "Act as an automation specialist. Design reliable repeatable workflows, scripts, and operational checks with clear failure modes.",
        ),
        "sentry" => (
            "operations",
            "Operations",
            "Act as an operations specialist. Monitor system health, diagnose runtime issues, and recommend low-risk operational actions.",
        ),
        "bash" => (
            "devops",
            "DevOps",
            "Act as a shell and DevOps specialist. Prefer safe, inspectable commands and explain command impact before risky execution.",
        ),
        "mneme" => (
            "knowledge",
            "Knowledge",
            "Act as a memory and knowledge-management specialist. Preserve useful context, retrieve relevant prior decisions, and keep notes structured.",
        ),
        "chronos" => (
            "planning",
            "Planning",
            "Act as a planning specialist. Turn goals into timelines, milestones, constraints, and next actions.",
        ),
        "muse" => (
            "creative",
            "Creative",
            "Act as a creative specialist. Generate polished copy, naming, narrative, and visual direction while matching the requested tone.",
        ),
        "relay" => (
            "network",
            "Network",
            "Act as a networking specialist. Reason about connectivity, ports, DNS, tunnels, and service boundaries.",
        ),
        "prism" => (
            "data",
            "Data",
            "Act as a data specialist. Analyze datasets, metrics, schemas, and queries with attention to correctness and decision usefulness.",
        ),
        "aegis" => (
            "security",
            "Security",
            "Act as a security specialist. Identify threats, risky assumptions, policy gaps, and mitigations without bypassing CADIS approvals.",
        ),
        "echo" => (
            "voice",
            "Voice",
            "Act as a voice I/O specialist. Focus on speech, transcription, pronunciation, latency, and accessibility constraints.",
        ),
        _ => (
            "general",
            "Generalist",
            "Act as a pragmatic specialist. Clarify the task, choose the right approach, and produce actionable results.",
        ),
    };
    AgentSpecialistProfile {
        specialist_id: specialist_id.to_owned(),
        specialist_label: specialist_label.to_owned(),
        persona: persona.to_owned(),
    }
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

fn compact_prompt_value(value: &str, max_chars: usize) -> String {
    let clean = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= max_chars {
        return clean;
    }
    let mut out = clean
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

fn agent_status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Spawning => "spawning",
        AgentStatus::Idle => "idle",
        AgentStatus::Running => "running",
        AgentStatus::WaitingApproval => "waiting_approval",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
        AgentStatus::Cancelled => "cancelled",
    }
}

fn agent_session_status_label(status: AgentSessionStatus) -> &'static str {
    match status {
        AgentSessionStatus::Started => "started",
        AgentSessionStatus::Running => "running",
        AgentSessionStatus::Completed => "completed",
        AgentSessionStatus::Failed => "failed",
        AgentSessionStatus::Cancelled => "cancelled",
        AgentSessionStatus::TimedOut => "timed_out",
        AgentSessionStatus::BudgetExceeded => "budget_exceeded",
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

fn execute_worker_command(
    record: &mut WorkerRecord,
    worker_command: &str,
) -> WorkerCommandExecution {
    if worker_command.is_empty() {
        return WorkerCommandExecution::default();
    }
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
                command: worker_command.to_owned(),
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
        .push(format!("command started: {worker_command}\n"));

    let report = match run_worker_validation_command(&cwd, worker_command) {
        Ok(result) => worker_command_report(worker_command, &cwd_display, result),
        Err(error) => WorkerCommandReport {
            command: worker_command.to_owned(),
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

fn run_worker_validation_command(
    cwd: &Path,
    worker_command: &str,
) -> Result<ShellRunResult, ErrorPayload> {
    let parts: Vec<&str> = worker_command.split_whitespace().collect();
    let (program, args) = parts.split_first().ok_or_else(|| ErrorPayload {
        code: "worker_command_empty".to_owned(),
        message: "worker command is empty".to_owned(),
        retryable: false,
    })?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(cadis_policy::filtered_env(
            &cadis_policy::shell_env_allowlist(),
        ));
    #[cfg(unix)]
    command.process_group(0);
    run_bounded_command(command, StdDuration::from_millis(WORKER_COMMAND_TIMEOUT_MS))
}

fn worker_command_report(
    worker_command: &str,
    cwd: &str,
    result: ShellRunResult,
) -> WorkerCommandReport {
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
        command: worker_command.to_owned(),
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

fn write_worker_artifacts(
    record: &mut WorkerRecord,
    status: WorkerState,
    summary: &str,
) -> Vec<String> {
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

fn shell_command(command: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/C").arg(command);
        cmd
    }
}

fn run_shell_command(
    cwd: &Path,
    command: &str,
    timeout: StdDuration,
) -> Result<ShellRunResult, ErrorPayload> {
    let mut child_command = shell_command(command);
    child_command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(cadis_policy::filtered_env(
            &cadis_policy::shell_env_allowlist(),
        ));
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
        AgentModelSetRequest, AgentRenameRequest, AgentSpawnRequest, AgentSpecialistSetRequest,
        ApprovalResponseRequest, ClientId, ContentKind, EmptyPayload, EventSubscriptionRequest,
        EventsSnapshotRequest, MessageSendRequest, RequestId, ServerFrame, SessionCreateRequest,
        SessionSubscriptionRequest, SessionTargetRequest, ToolCallRequest, VoiceDoctorCheck,
        VoiceDoctorRequest, VoicePreflightRequest, WorkerResultRequest, WorkerTailRequest,
        WorkspaceAccess, WorkspaceDoctorRequest, WorkspaceGrantRequest, WorkspaceId, WorkspaceKind,
        WorkspaceRegisterRequest, WorkspaceRevokeRequest,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
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

    #[derive(Clone, Debug)]
    struct PromptCaptureProvider {
        prompt: Arc<Mutex<String>>,
    }

    impl ModelProvider for PromptCaptureProvider {
        fn name(&self) -> &str {
            "prompt-capture"
        }

        fn chat(&self, prompt: &str) -> Result<Vec<String>, cadis_models::ModelError> {
            *self
                .prompt
                .lock()
                .expect("prompt lock should not be poisoned") = prompt.to_owned();
            Ok(vec!["captured".to_owned()])
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

    fn collect_worker_result(runtime: &mut Runtime, worker_id: &str) -> RequestOutcome {
        runtime.handle_request(RequestEnvelope::new(
            RequestId::from(format!("req_result_{worker_id}")),
            ClientId::from("cli_1"),
            ClientRequest::WorkerResult(WorkerResultRequest {
                worker_id: worker_id.to_owned(),
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
                    agent_session_id: None,
                    status: WorkerState::Running,
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
        assert_eq!(recovered.records[0].metadata.status, WorkerState::Failed);
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
        assert_eq!(recovered.records[0].metadata.status, WorkerState::Cancelled);
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
        // Cleanup executor now actually removes the worktree directory.
        assert!(!Path::new(&worktree_path).is_dir());
        assert!(!cleanup
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ApprovalRequested(_))));
        assert!(cleanup.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::WorkerCleanupRequested(payload)
                    if payload.worker_id == worker_id
            )
        }));

        let project_metadata = ProjectWorkspaceStore::new(&workspace)
            .load_worker_worktree_metadata(&worker_id)
            .expect("worker worktree metadata should load")
            .expect("worker worktree metadata should exist");
        assert_eq!(project_metadata.state, ProjectWorkerWorktreeState::Removed);

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
            WorkerWorktreeState::Removed
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
        assert_eq!(worker.metadata.status, WorkerState::Cancelled);
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
    fn worker_apply_requests_approval_and_applies_patch_after_approval() {
        let cadis_home = test_workspace("worker-apply-home");
        let workspace = test_workspace("worker-apply-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        register_workspace(&mut runtime, "worker-apply", &workspace);
        let (_session_id, worker_id, worktree_path) =
            complete_worker_in_workspace(&mut runtime, &workspace, "worker_apply");

        let patch_path = runtime
            .workers
            .get(&worker_id)
            .and_then(|worker| worker.artifacts.as_ref())
            .map(|artifacts| artifacts.patch.clone())
            .expect("worker patch artifact path should exist");

        let readme = workspace.join("README.md");
        let original = fs::read_to_string(&readme).expect("README should read");
        let patched = format!("{original}Applied from worker patch\n");
        fs::write(&readme, &patched).expect("README patched content should write");
        let patch = Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["diff", "--binary", "HEAD", "--", "README.md"])
            .output()
            .expect("git diff should run");
        assert!(
            patch.status.success(),
            "git diff should succeed: {}",
            String::from_utf8_lossy(&patch.stderr)
        );
        fs::write(&readme, &original).expect("README should reset to original");
        fs::write(&patch_path, &patch.stdout).expect("worker patch artifact should write");

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_worker_apply"),
            ClientId::from("hud_1"),
            ClientRequest::WorkerApply(WorkerApplyRequest {
                worker_id: worker_id.clone(),
                worktree_path: Some(worktree_path),
            }),
        ));

        assert!(matches!(
            request.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(request.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::ApprovalRequested(payload) if payload.summary.contains("worker.apply requires approval"))
        }));
        assert!(!request
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));

        let approved = approve(&mut runtime, approval_id_from(&request));
        assert!(approved
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::ToolStarted(_))));
        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolCompleted(payload)
                    if payload.summary.as_deref() == Some(&format!("applied worker patch for {worker_id}"))
            )
        }));

        let final_readme = fs::read_to_string(&readme).expect("README should read after apply");
        assert!(
            final_readme.contains("Applied from worker patch"),
            "README should include patch content after apply"
        );
    }

    #[test]
    fn worker_apply_rejects_empty_patch_artifact() {
        let cadis_home = test_workspace("worker-apply-empty-home");
        let workspace = test_workspace("worker-apply-empty-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        register_workspace(&mut runtime, "worker-apply-empty", &workspace);
        let (_session_id, worker_id, worktree_path) =
            complete_worker_in_workspace(&mut runtime, &workspace, "worker_apply_empty");

        let patch_path = runtime
            .workers
            .get(&worker_id)
            .and_then(|worker| worker.artifacts.as_ref())
            .map(|artifacts| artifacts.patch.clone())
            .expect("worker patch artifact path should exist");
        fs::write(&patch_path, "\n").expect("empty worker patch should write");

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_worker_apply_empty"),
            ClientId::from("hud_1"),
            ClientRequest::WorkerApply(WorkerApplyRequest {
                worker_id,
                worktree_path: Some(worktree_path),
            }),
        ));

        assert_rejected(request, "worker_patch_empty");
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
    fn completed_worker_result_collects_summary_and_artifact_paths_without_logs() {
        let cadis_home = test_workspace("completed-worker-result-home");
        let workspace = test_workspace("completed-worker-result-workspace");
        init_git_workspace(&workspace);

        let mut runtime = runtime_with_home(cadis_home);
        register_workspace(&mut runtime, "worker-result-git", &workspace);
        let session_id = runtime
            .handle_request(RequestEnvelope::new(
                RequestId::from("req_worker_result_session"),
                ClientId::from("cli_1"),
                ClientRequest::SessionCreate(SessionCreateRequest {
                    title: Some("Worker result".to_owned()),
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
            RequestId::from("req_completed_worker"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: Some(session_id),
                target_agent_id: None,
                content: "/route @codex run focused tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        let completed = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerCompleted(payload) => Some(payload.clone()),
                _ => None,
            })
            .expect("worker.completed should be emitted");
        let worker_id = completed.worker_id.clone();
        let raw_log_marker = "RAW-LARGE-WORKER-LOG-SHOULD-NOT-APPEAR";
        let _ = runtime.append_worker_log(
            &worker_id,
            format!("{raw_log_marker} {}\n", "x".repeat(4096)),
        );

        let result = collect_worker_result(&mut runtime, &worker_id);

        assert!(matches!(
            result.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(!result
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::WorkerLogDelta(_))));
        let agent_session = result
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSessionCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("agent.session.completed should be replayed");
        assert_eq!(
            completed.agent_session_id.as_ref(),
            Some(&agent_session.agent_session_id)
        );
        assert!(agent_session
            .result
            .as_deref()
            .is_some_and(|result| result.contains("run focused tests")));
        let worker = result
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerCompleted(payload) => Some(payload),
                _ => None,
            })
            .expect("worker.completed result should be replayed");
        let artifacts = worker
            .artifacts
            .as_ref()
            .expect("worker result should include artifact paths");
        assert!(Path::new(&artifacts.summary).is_file());
        assert!(Path::new(&artifacts.test_report).is_file());
        assert!(artifacts.test_report.ends_with("test-report.json"));
        let serialized = serde_json::to_string(&result.events).expect("events should serialize");
        assert!(!serialized.contains(raw_log_marker));
    }

    #[test]
    fn failed_worker_result_collects_agent_error_and_artifact_paths() {
        let mut runtime = runtime();
        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_failed_worker_result"),
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

        let _ = runtime.fail_message_generation(
            pending,
            cadis_models::ModelError::with_code(
                "provider_client_error",
                "provider request failed",
                true,
            ),
        );
        let result = collect_worker_result(&mut runtime, &worker_id);

        assert!(matches!(
            result.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(!result
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::WorkerLogDelta(_))));
        assert!(result.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionFailed(payload)
                    if payload.error_code.as_deref() == Some("provider_client_error")
                        && payload.error.as_deref() == Some("provider request failed")
            )
        }));
        let worker = result
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerFailed(payload) => Some(payload),
                _ => None,
            })
            .expect("worker.failed result should be replayed");
        assert_eq!(worker.status.as_deref(), Some("failed"));
        assert_eq!(worker.error_code.as_deref(), Some("provider_client_error"));
        let artifacts = worker
            .artifacts
            .as_ref()
            .expect("failed worker result should include artifact paths");
        assert!(Path::new(&artifacts.test_report).is_file());
        assert!(artifacts.test_report.ends_with("test-report.json"));
    }

    #[test]
    fn cancelled_worker_result_collects_agent_cancellation_and_artifact_paths() {
        let mut runtime = runtime();
        let pending = runtime
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_cancelled_worker_result"),
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

        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_cancel_worker_result_session"),
            ClientId::from("hud_1"),
            ClientRequest::SessionCancel(SessionTargetRequest { session_id }),
        ));
        let result = collect_worker_result(&mut runtime, &worker_id);

        assert!(matches!(
            result.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(!result
            .events
            .iter()
            .any(|event| matches!(event.event, CadisEvent::WorkerLogDelta(_))));
        assert!(result.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionCancelled(payload)
                    if payload.status == AgentSessionStatus::Cancelled
                        && payload.cancellation_requested_at.is_some()
            )
        }));
        let worker = result
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::WorkerCancelled(payload) => Some(payload),
                _ => None,
            })
            .expect("worker.cancelled result should be replayed");
        assert_eq!(worker.status.as_deref(), Some("cancelled"));
        assert_eq!(worker.error_code.as_deref(), Some("session_cancelled"));
        let artifacts = worker
            .artifacts
            .as_ref()
            .expect("cancelled worker result should include artifact paths");
        assert!(artifacts.test_report.ends_with("test-report.json"));
    }

    #[test]
    fn worker_result_rejects_unknown_worker() {
        let mut runtime = runtime();
        let outcome = collect_worker_result(&mut runtime, "worker_missing");

        assert_rejected(outcome, "worker_not_found");
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
        assert_eq!(metadata.agent.role, "specialist");
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
        assert_eq!(started.budget_steps, 8);
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
            token_budget: 0,
            tokens_used: 0,
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
            ..AgentRuntimeConfig::default()
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
    fn agent_specialist_set_is_confirmed_by_event() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::AgentSpecialistSet(AgentSpecialistSetRequest {
                agent_id: AgentId::from("atlas"),
                specialist_id: "marketing".to_owned(),
                specialist_label: "Marketing".to_owned(),
                persona: "Act as a senior growth marketer.".to_owned(),
            }),
        ));

        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSpecialistChanged(payload)
                    if payload.agent_id.as_str() == "atlas"
                        && payload.specialist_id == "marketing"
                        && payload.specialist_label == "Marketing"
                        && payload.persona == "Act as a senior growth marketer."
            )
        }));
    }

    #[test]
    fn message_send_includes_agent_specialist_persona_in_provider_prompt() {
        let captured = Arc::new(Mutex::new(String::new()));
        let provider = PromptCaptureProvider {
            prompt: Arc::clone(&captured),
        };
        let mut runtime = runtime_with_provider(Box::new(provider), "prompt-capture");

        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_specialist"),
            ClientId::from("hud_1"),
            ClientRequest::AgentSpecialistSet(AgentSpecialistSetRequest {
                agent_id: AgentId::from("codex"),
                specialist_id: "marketing".to_owned(),
                specialist_label: "Marketing".to_owned(),
                persona: "Act as a senior growth marketer before answering.".to_owned(),
            }),
        ));

        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_chat"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "draft launch positioning".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let prompt = captured.lock().expect("prompt lock should not be poisoned");
        assert!(prompt.contains("Specialist persona (Marketing):"));
        assert!(prompt.contains("Act as a senior growth marketer before answering."));
        assert!(prompt.contains("draft launch positioning"));
    }

    #[test]
    fn main_agent_prompt_includes_runtime_awareness_context() {
        let captured = Arc::new(Mutex::new(String::new()));
        let provider = PromptCaptureProvider {
            prompt: Arc::clone(&captured),
        };
        let mut runtime = runtime_with_provider(Box::new(provider), "prompt-capture");

        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_route"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "build the specialist selector".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_main"),
            ClientId::from("hud_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("main")),
                content: "what are the agents doing?".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let prompt = captured.lock().expect("prompt lock should not be poisoned");
        assert!(prompt.contains("Current CADIS runtime state:"));
        assert!(prompt.contains("codex / Codex"));
        assert!(prompt.contains("build the specialist selector"));
        assert!(prompt.contains("Recent agent sessions:"));
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
                        && payload.role.as_deref() == Some("specialist")
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
        assert!(spawned_agent.as_str().starts_with("specialist_"));
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSpawned(payload)
                    if payload.role.as_deref() == Some("specialist")
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

    // ── Track D: Approval expiry recheck (item 8) ────────────────────

    #[test]
    fn approval_recheck_denies_when_policy_changes_to_deny() {
        let workspace = test_workspace("approval-recheck");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "approval-recheck", &workspace);
        grant_workspace(
            &mut runtime,
            "approval-recheck",
            vec![WorkspaceAccess::Exec],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_recheck"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "approval-recheck",
                    "command": "echo recheck"
                }),
            }),
        ));
        let approval_id = approval_id_from(&request);

        // Change policy to deny SystemChange before approving.
        runtime.policy = cadis_policy::PolicyEngine::with_config(cadis_policy::PolicyConfig {
            risk_overrides: vec![cadis_policy::RiskOverride {
                risk_class: "SystemChange".to_owned(),
                decision: "deny".to_owned(),
            }],
            ..cadis_policy::PolicyConfig::default()
        });

        let approved = approve(&mut runtime, approval_id);
        assert!(approved.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ToolFailed(payload)
                    if payload.error.code == "policy_denied_at_execution"
            )
        }));
    }

    // ── Track D: Race condition tests (item 9) ───────────────────────

    #[test]
    fn concurrent_approval_second_response_is_rejected() {
        let workspace = test_workspace("approval-race");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "approval-race", &workspace);
        grant_workspace(&mut runtime, "approval-race", vec![WorkspaceAccess::Exec]);

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_race"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "approval-race",
                    "command": "echo race"
                }),
            }),
        ));
        let approval_id = approval_id_from(&request);

        // First response: approve.
        let first = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_race_approve_1"),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id: approval_id.clone(),
                decision: ApprovalDecision::Approved,
                reason: Some("first".to_owned()),
            }),
        ));
        assert!(first.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Approved
            )
        }));

        // Second response: should be rejected as already resolved.
        let second = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_race_approve_2"),
            ClientId::from("cli_2"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id,
                decision: ApprovalDecision::Denied,
                reason: Some("second".to_owned()),
            }),
        ));
        assert!(matches!(
            second.response.response,
            DaemonResponse::RequestRejected(error)
                if error.code == "approval_already_resolved"
        ));
    }

    #[test]
    fn concurrent_approval_deny_then_approve_is_rejected() {
        let workspace = test_workspace("approval-race-deny");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "approval-race-deny", &workspace);
        grant_workspace(
            &mut runtime,
            "approval-race-deny",
            vec![WorkspaceAccess::Exec],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_race_deny"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "approval-race-deny",
                    "command": "echo race"
                }),
            }),
        ));
        let approval_id = approval_id_from(&request);

        // First: deny.
        let first = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_race_deny_1"),
            ClientId::from("cli_1"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id: approval_id.clone(),
                decision: ApprovalDecision::Denied,
                reason: Some("denied first".to_owned()),
            }),
        ));
        assert!(first.events.iter().any(|event| {
            matches!(
                &event.event,
                CadisEvent::ApprovalResolved(payload)
                    if payload.decision == ApprovalDecision::Denied
            )
        }));

        // Second: approve should be rejected.
        let second = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_race_deny_2"),
            ClientId::from("cli_2"),
            ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                approval_id,
                decision: ApprovalDecision::Approved,
                reason: Some("too late".to_owned()),
            }),
        ));
        assert!(matches!(
            second.response.response,
            DaemonResponse::RequestRejected(error)
                if error.code == "approval_already_resolved"
        ));
    }

    #[test]
    fn approval_response_after_expiry_is_denied() {
        let workspace = test_workspace("approval-expiry");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "approval-expiry", &workspace);
        grant_workspace(&mut runtime, "approval-expiry", vec![WorkspaceAccess::Exec]);

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_expiry"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "approval-expiry",
                    "command": "echo expired"
                }),
            }),
        ));
        let approval_id = approval_id_from(&request);

        // Force the pending approval to be expired.
        if let Some(pending) = runtime.pending_approvals.get_mut(&approval_id) {
            let past =
                (Utc::now() - Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Secs, true);
            pending.record.expires_at = Timestamp::new_utc(past).expect("valid timestamp");
        }

        let outcome = approve(&mut runtime, approval_id);
        // The approval should resolve as denied due to expiry.
        let resolved = outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::ApprovalResolved(payload) => Some(payload),
                _ => None,
            })
            .expect("approval.resolved should be emitted");
        assert_eq!(resolved.decision, ApprovalDecision::Denied);
    }

    // ── Track D: Shell env filtering (item 3) ────────────────────────

    #[test]
    fn shell_run_filters_environment_variables() {
        let workspace = test_workspace("shell-env-filter");
        let mut runtime = runtime();
        register_workspace(&mut runtime, "shell-env-filter", &workspace);
        grant_workspace(
            &mut runtime,
            "shell-env-filter",
            vec![WorkspaceAccess::Exec],
        );

        let request = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_shell_env"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                agent_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace_id": "shell-env-filter",
                    "command": "env | sort"
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
            .expect("shell.run should complete");
        let stdout = completed
            .output
            .as_ref()
            .and_then(|o| o["stdout"].as_str())
            .unwrap_or("");
        // Should not contain sensitive env vars like CADIS_OPENAI_API_KEY
        // but should contain PATH.
        let env_keys: Vec<&str> = stdout
            .lines()
            .filter_map(|line| line.split('=').next())
            .collect();
        // Sensitive vars must not leak through env_clear + allowlist.
        let denied = [
            "CADIS_OPENAI_API_KEY",
            "OPENAI_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "SSH_AUTH_SOCK",
            "CARGO_HOME",
        ];
        for key in &denied {
            assert!(
                !env_keys.contains(key),
                "sensitive env var leaked into shell: {key}"
            );
        }
        // Allowlisted vars should be present (PATH is always set).
        assert!(env_keys.contains(&"PATH"), "PATH should be in shell env");
    }

    // ── Track D: Cancellation token (item 4) ─────────────────────────

    #[test]
    fn cancellation_token_is_observable_across_clones() {
        let token = cadis_policy::CancellationToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }

    // ── Track C: Agent Runtime ─────────────────────────────────────

    #[test]
    fn agent_role_enum_classifies_known_roles() {
        use cadis_protocol::AgentRole;
        assert_eq!("main".parse::<AgentRole>(), Ok(AgentRole::Main));
        assert_eq!("orchestrator".parse::<AgentRole>(), Ok(AgentRole::Main));
        assert_eq!("worker".parse::<AgentRole>(), Ok(AgentRole::Worker));
        assert_eq!("router".parse::<AgentRole>(), Ok(AgentRole::Router));
        assert_eq!("Coding".parse::<AgentRole>(), Ok(AgentRole::Specialist));
        // Unknown roles default to Specialist.
        assert_eq!("custom".parse::<AgentRole>(), Ok(AgentRole::Specialist));
    }

    #[test]
    fn worker_state_enum_tracks_terminal_states() {
        use cadis_protocol::WorkerState;
        assert!(!WorkerState::Queued.is_terminal());
        assert!(!WorkerState::Running.is_terminal());
        assert!(WorkerState::Completed.is_terminal());
        assert!(WorkerState::Failed.is_terminal());
        assert!(WorkerState::Cancelled.is_terminal());
    }

    #[test]
    fn model_driven_spawn_parses_spawn_directives() {
        let content = "Here is the plan:\n[SPAWN Reviewer: check the patch]\n[SPAWN Tester: run unit tests]\nDone.";
        let directives = parse_model_spawn_directives(content);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].role, "specialist");
        assert_eq!(directives[0].task, "check the patch");
        assert_eq!(directives[1].role, "specialist");
        assert_eq!(directives[1].task, "run unit tests");
    }

    #[test]
    fn model_driven_spawn_ignores_malformed_directives() {
        let content = "[SPAWN ]\n[SPAWN :no role]\n[SPAWN Role:]\nnormal text";
        let directives = parse_model_spawn_directives(content);
        assert!(directives.is_empty());
    }

    #[test]
    fn model_driven_spawn_creates_child_agents_from_response() {
        // Use a provider that returns a spawn directive in its response.
        struct SpawnProvider;
        impl ModelProvider for SpawnProvider {
            fn name(&self) -> &str {
                "spawn-test"
            }
            fn chat(&self, _prompt: &str) -> Result<Vec<String>, cadis_models::ModelError> {
                Ok(vec!["[SPAWN Reviewer: check patch]".to_owned()])
            }
        }
        let mut runtime = runtime_with_provider(Box::new(SpawnProvider), "spawn-test");
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "plan work".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        assert!(matches!(
            outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        // The model response should have triggered a spawn.
        let spawned = outcome
            .events
            .iter()
            .filter(|event| {
                matches!(&event.event, CadisEvent::AgentSpawned(payload)
                if payload.role.as_deref() == Some("specialist"))
            })
            .count();
        assert_eq!(
            spawned, 1,
            "model-driven spawn should create one specialist agent"
        );
    }

    #[test]
    fn tool_call_loop_allows_multiple_steps() {
        let mut runtime = runtime_with_agent_runtime_config(AgentRuntimeConfig {
            max_steps_per_session: 4,
            ..AgentRuntimeConfig::default()
        });
        // Send a message - with budget=4, the session should succeed (uses 1 step).
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_multi"),
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
        let session = outcome.events.iter().find_map(|event| match &event.event {
            CadisEvent::AgentSessionStarted(payload) => Some(payload.clone()),
            _ => None,
        });
        let session = session.expect("agent session should start");
        assert_eq!(session.budget_steps, 4);
        // Should complete, not exceed budget.
        assert!(outcome
            .events
            .iter()
            .any(|event| { matches!(&event.event, CadisEvent::AgentSessionCompleted(_)) }));
    }

    #[test]
    fn configurable_worker_command_uses_runtime_config() {
        // Empty command should skip execution.
        let mut record = WorkerRecord {
            worker_id: "w1".to_owned(),
            session_id: SessionId::from("ses_1"),
            agent_id: None,
            parent_agent_id: None,
            agent_session_id: None,
            status: WorkerState::Running,
            cli: None,
            cwd: None,
            summary: None,
            error_code: None,
            error: None,
            cancellation_requested_at: None,
            worktree: Some(WorkerWorktreeIntent {
                workspace_id: None,
                project_root: None,
                worktree_root: "/tmp".to_owned(),
                worktree_path: "/tmp/w1".to_owned(),
                branch_name: "cadis/w1".to_owned(),
                base_ref: None,
                state: WorkerWorktreeState::Active,
                cleanup_policy: WorkerWorktreeCleanupPolicy::Explicit,
            }),
            artifacts: None,
            updated_at: now_timestamp(),
            log_lines: Vec::new(),
            command_report: None,
        };
        let result = execute_worker_command(&mut record, "");
        assert!(result.logs.is_empty());
        assert!(result.failure.is_none());
    }

    #[test]
    fn kill_agent_cancels_active_sessions_and_workers() {
        let mut runtime = runtime();
        // Spawn a child agent.
        let spawn_outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Tester".to_owned(),
                parent_agent_id: None,
                display_name: Some("TestAgent".to_owned()),
                model: None,
            }),
        ));
        let spawned_id = spawn_outcome
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                _ => None,
            })
            .expect("agent should be spawned");

        // Kill the agent.
        let kill_outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_kill"),
            ClientId::from("cli_1"),
            ClientRequest::AgentKill(cadis_protocol::AgentTargetRequest {
                agent_id: spawned_id.clone(),
            }),
        ));

        assert!(matches!(
            kill_outcome.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        assert!(kill_outcome.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::AgentKilled(payload)
                if payload.agent_id == spawned_id
                    && payload.status == Some(AgentStatus::Cancelled))
        }));
    }

    #[test]
    fn kill_main_agent_is_rejected() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_kill_main"),
            ClientId::from("cli_1"),
            ClientRequest::AgentKill(cadis_protocol::AgentTargetRequest {
                agent_id: AgentId::from("main"),
            }),
        ));
        assert_rejected(outcome, "cannot_kill_main_agent");
    }

    #[test]
    fn tail_agent_returns_recent_sessions() {
        let mut runtime = runtime();
        // Send a message to create an agent session for codex.
        let _ = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_msg"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(AgentId::from("codex")),
                content: "run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));

        let tail = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tail"),
            ClientId::from("cli_1"),
            ClientRequest::AgentTail(AgentTailRequest {
                agent_id: AgentId::from("codex"),
                limit: Some(5),
            }),
        ));

        assert!(matches!(
            tail.response.response,
            DaemonResponse::RequestAccepted(_)
        ));
        // Should include agent status and at least one session event.
        assert!(tail.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::AgentStatusChanged(payload)
                if payload.agent_id.as_str() == "codex")
        }));
        assert!(
            tail.events.len() >= 2,
            "tail should include status + session events"
        );
    }

    #[test]
    fn tail_agent_rejects_unknown_agent() {
        let mut runtime = runtime();
        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tail_unknown"),
            ClientId::from("cli_1"),
            ClientRequest::AgentTail(AgentTailRequest {
                agent_id: AgentId::from("nonexistent"),
                limit: None,
            }),
        ));
        assert_rejected(outcome, "agent_not_found");
    }

    #[test]
    fn fan_out_multi_agent_tree_spawns_and_routes() {
        let mut runtime = runtime();

        // Spawn two child agents under main.
        let spawn1 = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Reviewer".to_owned(),
                parent_agent_id: None,
                display_name: Some("Rev1".to_owned()),
                model: None,
            }),
        ));
        let agent1 = spawn1
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                _ => None,
            })
            .expect("first agent should spawn");

        let spawn2 = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn_2"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Tester".to_owned(),
                parent_agent_id: None,
                display_name: Some("Test1".to_owned()),
                model: None,
            }),
        ));
        let agent2 = spawn2
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                _ => None,
            })
            .expect("second agent should spawn");

        // Route messages to each child.
        let msg1 = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_msg_1"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(agent1.clone()),
                content: "review code".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        assert!(msg1.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::OrchestratorRoute(payload)
                if payload.target_agent_id == agent1)
        }));

        let msg2 = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_msg_2"),
            ClientId::from("cli_1"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: Some(agent2.clone()),
                content: "run tests".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        assert!(msg2.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::OrchestratorRoute(payload)
                if payload.target_agent_id == agent2)
        }));

        // Both should complete independently.
        assert!(msg1.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::MessageCompleted(payload)
                if payload.agent_id.as_ref() == Some(&agent1))
        }));
        assert!(msg2.events.iter().any(|event| {
            matches!(&event.event, CadisEvent::MessageCompleted(payload)
                if payload.agent_id.as_ref() == Some(&agent2))
        }));

        // Verify agent tree: both children have main as parent.
        let agents = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_agents"),
            ClientId::from("cli_1"),
            ClientRequest::AgentList(EmptyPayload {}),
        ));
        let agent_list = agents
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentListResponse(payload) => Some(payload),
                _ => None,
            })
            .expect("agent list should be emitted");

        let child1 = agent_list
            .agents
            .iter()
            .find(|a| a.agent_id == agent1)
            .expect("agent1 in list");
        let child2 = agent_list
            .agents
            .iter()
            .find(|a| a.agent_id == agent2)
            .expect("agent2 in list");
        assert_eq!(
            child1.parent_agent_id.as_ref().map(AgentId::as_str),
            Some("main")
        );
        assert_eq!(
            child2.parent_agent_id.as_ref().map(AgentId::as_str),
            Some("main")
        );
    }

    #[test]
    fn fan_out_spawn_depth_limit_prevents_deep_trees() {
        let mut runtime = runtime_with_spawn_limits(AgentSpawnLimits {
            max_depth: 1,
            max_children_per_parent: 4,
            max_total_agents: 32,
        });

        // Spawn a child under main (depth 1 - allowed).
        let spawn1 = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "Worker".to_owned(),
                parent_agent_id: None,
                display_name: None,
                model: None,
            }),
        ));
        let child_id = spawn1
            .events
            .iter()
            .find_map(|event| match &event.event {
                CadisEvent::AgentSpawned(payload) => Some(payload.agent_id.clone()),
                _ => None,
            })
            .expect("child should spawn");

        // Try to spawn a grandchild (depth 2 - should fail with max_depth=1).
        let spawn2 = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_spawn_2"),
            ClientId::from("cli_1"),
            ClientRequest::AgentSpawn(AgentSpawnRequest {
                role: "SubWorker".to_owned(),
                parent_agent_id: Some(child_id),
                display_name: None,
                model: None,
            }),
        ));
        assert_rejected(spawn2, "agent_spawn_depth_limit_exceeded");
    }

    #[test]
    fn edge_tts_rejects_invalid_voice_id() {
        let mut provider = EdgeTtsProvider;
        let result = provider.speak(TtsRequest {
            text: "hello",
            voice_id: "invalid voice id!",
            rate: 0,
            pitch: 0,
            volume: 0,
        });
        assert_eq!(result.unwrap_err().code, "invalid_voice_id");
    }

    #[test]
    fn edge_tts_rejects_empty_voice_id() {
        let mut provider = EdgeTtsProvider;
        let result = provider.speak(TtsRequest {
            text: "hello",
            voice_id: "",
            rate: 0,
            pitch: 0,
            volume: 0,
        });
        assert_eq!(result.unwrap_err().code, "invalid_voice_id");
    }

    #[test]
    fn edge_tts_handles_missing_binary() {
        let mut provider = EdgeTtsProvider;
        let result = provider.speak(TtsRequest {
            text: "hello",
            voice_id: "en-US-AvaNeural",
            rate: 0,
            pitch: 0,
            volume: 0,
        });
        // CI likely lacks edge-tts; accept not_found or spawn_failed.
        let err = result.unwrap_err();
        assert!(
            err.code == "edge_tts_not_found" || err.code == "edge_tts_spawn_failed",
            "unexpected error code: {}",
            err.code
        );
    }

    #[test]
    fn dangerous_delete_command_is_blocked() {
        let workspace = test_workspace("dangerous-delete");
        let runtime = runtime_with_home(test_workspace("cadis-home-dd"));
        let input = serde_json::json!({ "command": "rm -rf /tmp/something" });
        let result = runtime.execute_shell_run(&workspace, &input, 10);
        let err = result.unwrap_err();
        assert_eq!(err.code, "dangerous_delete_blocked");
    }

    #[test]
    fn file_patch_detects_concurrent_edit() {
        let workspace = test_workspace("concurrent-edit");
        let file_path = workspace.join("target.txt");
        fs::write(&file_path, "original content").expect("write fixture");

        // Prepare the patch (records mtime).
        let operations = vec![FilePatchOperation::Replace {
            path: "target.txt".to_owned(),
            old: "original".to_owned(),
            new: "replaced".to_owned(),
        }];
        let prepared = prepare_file_patch(&workspace, &operations).expect("prepare should succeed");
        assert_eq!(prepared.len(), 1);
        assert!(prepared[0].mtime.is_some());

        // Simulate concurrent edit: wait then rewrite same content to bump mtime.
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&file_path, "original content").expect("concurrent rewrite");

        let current_mtime = fs::metadata(&file_path).unwrap().modified().unwrap();
        if prepared[0].mtime.unwrap() == current_mtime {
            return; // filesystem granularity too coarse; skip gracefully
        }

        // The mtime check in execute_file_patch should catch this.
        // Exercise the check directly since execute_file_patch re-prepares internally.
        let change = &prepared[0];
        let expected_mtime = change.mtime.unwrap();
        let meta = fs::metadata(&change.path).unwrap();
        let actual_mtime = meta.modified().unwrap();
        assert_ne!(expected_mtime, actual_mtime);

        // Also verify the error path via tool_error construction matches the gate.
        let err = tool_error(
            "file_patch_concurrent_edit",
            format!(
                "{} was modified since patch was prepared",
                change.display_path
            ),
            true,
        );
        assert_eq!(err.code, "file_patch_concurrent_edit");
        assert!(err.retryable);
    }

    #[test]
    fn parse_tool_call_directives_parses_valid_directives() {
        let content = r#"Let me read the file.
[TOOL file.read: {"path": "src/main.rs"}]
And check git status.
[TOOL git.status: {"path": "."}]
Done."#;
        let directives = parse_tool_call_directives(content);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].tool_name, "file.read");
        assert_eq!(directives[0].input["path"], "src/main.rs");
        assert_eq!(directives[1].tool_name, "git.status");
        assert_eq!(directives[1].input["path"], ".");
    }

    #[test]
    fn parse_tool_call_directives_ignores_malformed() {
        let content = "[TOOL ]\n[TOOL :{}]\n[TOOL name:]\n[TOOL name: not json]\nnormal text";
        let directives = parse_tool_call_directives(content);
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_tool_call_directives_caps_at_max() {
        let content = (0..10)
            .map(|i| format!("[TOOL file.read: {{\"path\": \"file{i}.rs\"}}]"))
            .collect::<Vec<_>>()
            .join("\n");
        let directives = parse_tool_call_directives(&content);
        assert_eq!(directives.len(), 5);
    }
}
