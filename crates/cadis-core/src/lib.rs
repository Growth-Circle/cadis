//! Core CADIS request handling and event production.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use cadis_models::ModelProvider;
use cadis_protocol::{
    AgentEventPayload, AgentId, AgentListPayload, AgentModelChangedPayload, AgentRenamedPayload,
    AgentSpawnRequest, AgentStatus, AgentStatusChangedPayload, CadisEvent, ClientRequest,
    DaemonResponse, DaemonStatusPayload, ErrorPayload, EventEnvelope, EventId,
    MessageCompletedPayload, MessageDeltaPayload, MessageSendRequest, ModelDescriptor,
    ModelsListPayload, OrchestratorRoutePayload, ProtocolVersion, RequestAcceptedPayload,
    RequestEnvelope, RequestId, ResponseEnvelope, SessionEventPayload, SessionId, Timestamp,
    UiPreferencesPayload,
};
use chrono::{SecondsFormat, Utc};

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
    started_at: Instant,
    next_event: u64,
    next_session: u64,
    next_agent: u64,
    next_route: u64,
    sessions: HashMap<SessionId, SessionRecord>,
    agents: HashMap<AgentId, AgentRecord>,
    ui_preferences: serde_json::Value,
}

impl Runtime {
    /// Creates a runtime with the supplied model provider.
    pub fn new(options: RuntimeOptions, provider: Box<dyn ModelProvider>) -> Self {
        let ui_preferences = options.ui_preferences.clone();
        let agents = default_agents(&options.model_provider);

        Self {
            options,
            provider,
            started_at: Instant::now(),
            next_event: 1,
            next_session: 1,
            next_agent: 1,
            next_route: 1,
            sessions: HashMap::new(),
            agents,
            ui_preferences,
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
                let event = self.event(
                    None,
                    CadisEvent::ModelsListResponse(ModelsListPayload {
                        models: vec![
                            ModelDescriptor {
                                provider: "auto".to_owned(),
                                model: "ollama-or-echo".to_owned(),
                                display_name: "Auto (Ollama, then local fallback)".to_owned(),
                                capabilities: vec![
                                    "streaming".to_owned(),
                                    "local_fallback".to_owned(),
                                ],
                            },
                            ModelDescriptor {
                                provider: "ollama".to_owned(),
                                model: "configured".to_owned(),
                                display_name: "Ollama local model".to_owned(),
                                capabilities: vec![
                                    "streaming".to_owned(),
                                    "local_model".to_owned(),
                                ],
                            },
                            ModelDescriptor {
                                provider: "codex-cli".to_owned(),
                                model: "chatgpt-plan".to_owned(),
                                display_name: "Codex CLI (ChatGPT Plus/Pro login)".to_owned(),
                                capabilities: vec![
                                    "codex_cli".to_owned(),
                                    "chatgpt_login".to_owned(),
                                    "read_only_sandbox".to_owned(),
                                ],
                            },
                            ModelDescriptor {
                                provider: "openai".to_owned(),
                                model: "configured".to_owned(),
                                display_name: "OpenAI API model".to_owned(),
                                capabilities: vec!["api_key".to_owned()],
                            },
                            ModelDescriptor {
                                provider: "echo".to_owned(),
                                model: "cadis-local-fallback".to_owned(),
                                display_name: "CADIS local fallback".to_owned(),
                                capabilities: vec!["offline".to_owned()],
                            },
                        ],
                    }),
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
            ClientRequest::SessionUnsubscribe(_)
            | ClientRequest::ApprovalRespond(_)
            | ClientRequest::WorkerTail(_) => self.reject(
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

    fn handle_message(
        &mut self,
        request_id: RequestId,
        request: MessageSendRequest,
    ) -> RequestOutcome {
        let content_kind = request.content_kind;
        let content = request.content;
        let route = match self.route_message(request.target_agent_id, &content) {
            Ok(route) => route,
            Err(error) => return self.reject(request_id, error.code, error.message, false),
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
        match self.provider.chat(&prompt) {
            Ok(deltas) => {
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
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::SessionCompleted(SessionEventPayload {
                        session_id,
                        title: None,
                    }),
                ));
            }
            Err(error) => {
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                        agent_id: route.agent_id,
                        status: AgentStatus::Failed,
                        task: Some(error.to_string()),
                    }),
                ));
                events.push(self.session_event(
                    session_id,
                    CadisEvent::SessionFailed(ErrorPayload {
                        code: "model_error".to_owned(),
                        message: error.to_string(),
                        retryable: true,
                    }),
                ));
            }
        }

        self.accept(request_id, events)
    }

    fn spawn_agent(&mut self, request_id: RequestId, request: AgentSpawnRequest) -> RequestOutcome {
        let role = normalize_role(&request.role);
        if role.is_empty() {
            return self.reject(
                request_id,
                "invalid_agent_role",
                "agent role is empty",
                false,
            );
        }

        let parent_agent_id = if let Some(parent_agent_id) = request.parent_agent_id {
            if !self.agents.contains_key(&parent_agent_id) {
                return self.reject(
                    request_id,
                    "parent_agent_not_found",
                    format!("parent agent '{parent_agent_id}' was not found"),
                    false,
                );
            }
            Some(parent_agent_id)
        } else {
            Some(AgentId::from("main"))
        };
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
            parent_agent_id,
            model,
            status: AgentStatus::Idle,
        };
        self.agents.insert(agent_id.clone(), record.clone());

        let event = self.event(None, CadisEvent::AgentSpawned(record.event_payload()));
        let status = self.event(
            None,
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id,
                status: AgentStatus::Idle,
                task: Some("spawned and ready".to_owned()),
            }),
        );
        self.accept(request_id, vec![event, status])
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

    fn route_message(
        &self,
        explicit_agent_id: Option<AgentId>,
        content: &str,
    ) -> Result<RouteDecision, RouteError> {
        if let Some(agent_id) = explicit_agent_id {
            return self.route_to_agent(
                agent_id,
                content.to_owned(),
                "explicit target_agent_id".to_owned(),
            );
        }

        if let Some((mention, remaining)) = leading_mention(content) {
            let Some(agent_id) = self.resolve_agent_mention(&mention) else {
                return Err(RouteError {
                    code: "agent_not_found",
                    message: format!("no agent matches @{mention}"),
                });
            };
            return self.route_to_agent(agent_id, remaining, format!("@{mention} mention"));
        }

        self.route_to_agent(
            AgentId::from("main"),
            content.to_owned(),
            "default orchestrator".to_owned(),
        )
    }

    fn route_to_agent(
        &self,
        agent_id: AgentId,
        content: String,
        reason: String,
    ) -> Result<RouteDecision, RouteError> {
        let Some(agent) = self.agents.get(&agent_id) else {
            return Err(RouteError {
                code: "agent_not_found",
                message: format!("agent '{agent_id}' was not found"),
            });
        };
        let content = content.trim().to_owned();
        Ok(RouteDecision {
            agent_id: agent.id.clone(),
            agent_name: agent.display_name.clone(),
            content: if content.is_empty() {
                "Continue.".to_owned()
            } else {
                content
            },
            reason,
        })
    }

    fn resolve_agent_mention(&self, mention: &str) -> Option<AgentId> {
        let normalized = normalize_lookup(mention);
        self.agents
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RouteError {
    code: &'static str,
    message: String,
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
    use cadis_models::EchoProvider;
    use cadis_protocol::{
        AgentModelSetRequest, AgentRenameRequest, AgentSpawnRequest, ClientId, ContentKind,
        EmptyPayload, MessageSendRequest, RequestId, ServerFrame, SessionCreateRequest,
    };

    fn runtime() -> Runtime {
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
                    }
                }),
            },
            Box::<EchoProvider>::default(),
        )
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
}
