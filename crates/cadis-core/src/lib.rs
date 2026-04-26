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
    AgentSpawnRequest, AgentStatus, AgentStatusChangedPayload, ApprovalDecision, ApprovalId,
    ApprovalRequestPayload, ApprovalResolvedPayload, ApprovalResponseRequest, CadisEvent,
    ClientRequest, DaemonResponse, DaemonStatusPayload, ErrorPayload, EventEnvelope, EventId,
    MessageCompletedPayload, MessageDeltaPayload, MessageSendRequest, ModelDescriptor,
    ModelInvocationPayload, ModelReadiness, ModelsListPayload, OrchestratorRoutePayload,
    ProtocolVersion, RequestAcceptedPayload, RequestEnvelope, RequestId, ResponseEnvelope,
    SessionEventPayload, SessionId, Timestamp, ToolCallId, ToolCallRequest, ToolEventPayload,
    ToolFailedPayload, UiPreferencesPayload, WorkerEventPayload,
};
use cadis_store::{redact, ApprovalRecord, ApprovalState, ApprovalStore};
use chrono::{DateTime, Duration, SecondsFormat, Utc};

const FILE_READ_LIMIT_BYTES: usize = 64 * 1024;
const FILE_SEARCH_LIMIT_BYTES: u64 = 1024 * 1024;
const FILE_SEARCH_DEFAULT_LIMIT: usize = 50;
const APPROVAL_TIMEOUT_MINUTES: i64 = 5;

/// Runtime options supplied by the daemon process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeOptions {
    /// Local CADIS state directory.
    pub cadis_home: PathBuf,
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
    next_route: u64,
    next_worker: u64,
    sessions: HashMap<SessionId, SessionRecord>,
    agents: HashMap<AgentId, AgentRecord>,
    orchestrator: Orchestrator,
    ui_preferences: serde_json::Value,
    spawn_limits: AgentSpawnLimits,
    policy: PolicyEngine,
    approval_store: ApprovalStore,
    pending_approvals: HashMap<ApprovalId, PendingApproval>,
    next_tool: u64,
    next_approval: u64,
}

impl Runtime {
    /// Creates a runtime with the supplied model provider.
    pub fn new(options: RuntimeOptions, provider: Box<dyn ModelProvider>) -> Self {
        let ui_preferences = options.ui_preferences.clone();
        let spawn_limits = AgentSpawnLimits::from_options(&options.ui_preferences);
        let orchestrator =
            Orchestrator::new(OrchestratorConfig::from_options(&options.ui_preferences));
        let agents = default_agents(&options.model_provider);

        let approval_store = ApprovalStore::new(&options.cadis_home);

        Self {
            options,
            provider,
            tools: ToolRegistry::builtin().expect("built-in tool registry should be valid"),
            started_at: Instant::now(),
            next_event: 1,
            next_session: 1,
            next_agent: 1,
            next_route: 1,
            next_worker: 1,
            sessions: HashMap::new(),
            agents,
            orchestrator,
            ui_preferences,
            spawn_limits,
            policy: PolicyEngine,
            approval_store,
            pending_approvals: HashMap::new(),
            next_tool: 1,
            next_approval: 1,
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
                    let event = self.session_event(
                        request.session_id.clone(),
                        CadisEvent::SessionCompleted(SessionEventPayload {
                            session_id: request.session_id,
                            title: None,
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
            ClientRequest::SessionUnsubscribe(_) | ClientRequest::WorkerTail(_) => self.reject(
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

                match self.execute_safe_tool(&session_id, &request) {
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
                let workspace = tool_workspace_summary(&request.input)
                    .or_else(|| self.session_workspace(&session_id));
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
                self.sessions.insert(
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

        events.push(self.session_event(
            session_id.clone(),
            CadisEvent::OrchestratorRoute(OrchestratorRoutePayload {
                id: format!("route_{:06}", self.next_route),
                source: "cadisd".to_owned(),
                target_agent_id: route.agent_id.clone(),
                target_agent_name: route.agent_name.clone(),
                reason: route.reason.clone(),
            }),
        ));
        self.next_route += 1;

        let worker = route
            .worker_summary
            .as_ref()
            .map(|summary| WorkerDelegation {
                worker_id: self.next_worker_id(),
                parent_agent_id: self
                    .agents
                    .get(&route.agent_id)
                    .and_then(|agent| agent.parent_agent_id.clone())
                    .or_else(|| (route.agent_id.as_str() != "main").then(|| AgentId::from("main"))),
                summary: summary.clone(),
            });

        if let Some(worker) = &worker {
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::WorkerStarted(WorkerEventPayload {
                    worker_id: worker.worker_id.clone(),
                    agent_id: Some(route.agent_id.clone()),
                    parent_agent_id: worker.parent_agent_id.clone(),
                    status: Some("running".to_owned()),
                    cli: None,
                    cwd: None,
                    summary: Some(worker.summary.clone()),
                }),
            ));
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
                        content: Some(final_content),
                        agent_id: Some(route.agent_id.clone()),
                        agent_name: Some(route.agent_name.clone()),
                        model,
                    }),
                ));
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
                    events.push(self.session_event(
                        session_id.clone(),
                        CadisEvent::WorkerCompleted(WorkerEventPayload {
                            worker_id: worker.worker_id.clone(),
                            agent_id: Some(route.agent_id.clone()),
                            parent_agent_id: worker.parent_agent_id.clone(),
                            status: Some("completed".to_owned()),
                            cli: None,
                            cwd: None,
                            summary: Some(worker.summary.clone()),
                        }),
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
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                        agent_id: route.agent_id.clone(),
                        status: AgentStatus::Failed,
                        task: Some(error_message.clone()),
                    }),
                ));
                if let Some(worker) = &worker {
                    events.push(self.session_event(
                        session_id.clone(),
                        CadisEvent::WorkerCompleted(WorkerEventPayload {
                            worker_id: worker.worker_id.clone(),
                            agent_id: Some(route.agent_id.clone()),
                            parent_agent_id: worker.parent_agent_id.clone(),
                            status: Some("failed".to_owned()),
                            cli: None,
                            cwd: None,
                            summary: Some(error_message.clone()),
                        }),
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
        record.status = AgentStatus::Completed;
        let event = self.event(None, CadisEvent::AgentCompleted(record.event_payload()));
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
                CadisEvent::UiPreferencesUpdated(UiPreferencesPayload {
                    preferences: self.ui_preferences.clone(),
                }),
            ),
        ];

        for (session_id, session) in self.session_records_sorted() {
            events.push(self.session_event(
                session_id.clone(),
                CadisEvent::SessionUpdated(SessionEventPayload {
                    session_id,
                    title: session.title,
                }),
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

    fn resolve_tool_session(
        &mut self,
        requested_session_id: Option<SessionId>,
        input: &serde_json::Value,
    ) -> (SessionId, Vec<EventEnvelope>) {
        let cwd = tool_workspace_summary(input);
        match requested_session_id {
            Some(session_id) if self.sessions.contains_key(&session_id) => (session_id, Vec::new()),
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
        session_id: &SessionId,
        request: &ToolCallRequest,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        match request.tool_name.as_str() {
            tool_name if self.tools.is_auto_executable_safe_read(tool_name) => match tool_name {
                "file.read" => self.execute_file_read(session_id, &request.input),
                "file.search" => self.execute_file_search(session_id, &request.input),
                "git.status" => self.execute_git_status(session_id, &request.input),
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
        session_id: &SessionId,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let workspace = self.resolved_workspace(session_id, input)?;
        let path = input_string(input, "path")
            .ok_or_else(|| tool_error("invalid_tool_input", "file.read requires path", false))?;
        let path = resolve_inside_workspace(&workspace, &path)?;
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
        let relative = display_relative_path(&workspace, &path);

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
        session_id: &SessionId,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let workspace = self.resolved_workspace(session_id, input)?;
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
        let root = resolve_inside_workspace(&workspace, &root)?;
        let max_results = input_usize(input, "max_results").unwrap_or(FILE_SEARCH_DEFAULT_LIMIT);
        let mut matches = Vec::new();
        search_files(&workspace, &root, &query, max_results, &mut matches);
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
        session_id: &SessionId,
        input: &serde_json::Value,
    ) -> Result<ToolExecutionResult, ErrorPayload> {
        let workspace = self.resolved_workspace(session_id, input)?;
        let cwd = input_string(input, "path")
            .or_else(|| input_string(input, "cwd"))
            .unwrap_or_else(|| ".".to_owned());
        let cwd = resolve_inside_workspace(&workspace, &cwd)?;
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
                "cwd": display_relative_path(&workspace, &cwd),
                "status": stdout
            }),
        })
    }

    fn resolved_workspace(
        &self,
        session_id: &SessionId,
        input: &serde_json::Value,
    ) -> Result<PathBuf, ErrorPayload> {
        let workspace = tool_workspace_summary(input)
            .or_else(|| self.session_workspace(session_id))
            .map(PathBuf::from)
            .unwrap_or_else(|| self.options.cadis_home.clone());
        workspace.canonicalize().map_err(|error| {
            tool_error(
                "invalid_workspace",
                format!(
                    "could not resolve workspace {}: {error}",
                    workspace.display()
                ),
                false,
            )
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
        self.sessions
            .insert(session_id.clone(), SessionRecord { title, _cwd: cwd });
        session_id
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct AgentRecord {
    id: AgentId,
    role: String,
    display_name: String,
    parent_agent_id: Option<AgentId>,
    model: String,
    status: AgentStatus,
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
    input_string(input, "workspace").or_else(|| input_string(input, "cwd"))
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

fn json_usize(value: &serde_json::Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
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
        MessageSendRequest, RequestId, ServerFrame, SessionCreateRequest, ToolCallRequest,
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
        Runtime::new(
            RuntimeOptions {
                cadis_home: PathBuf::from("/tmp/cadis-test"),
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
                cadis_home: PathBuf::from("/tmp/cadis-test"),
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

    fn test_workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cadis-core-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&path).expect("test workspace should be created");
        path
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

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace": workspace,
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

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                tool_name: "file.read".to_owned(),
                input: serde_json::json!({
                    "workspace": workspace,
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
    fn risky_shell_tool_requests_approval_and_denial_fails_tool() {
        let workspace = test_workspace("shell-approval");
        let mut runtime = runtime();

        let outcome = runtime.handle_request(RequestEnvelope::new(
            RequestId::from("req_tool"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: None,
                tool_name: "shell.run".to_owned(),
                input: serde_json::json!({
                    "workspace": workspace,
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
