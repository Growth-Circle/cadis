//! Core CADIS request handling and event production.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use cadis_models::ModelProvider;
use cadis_protocol::{
    AgentId, AgentModelChangedPayload, AgentRenamedPayload, AgentStatus, AgentStatusChangedPayload,
    CadisEvent, ClientRequest, ContentKind, DaemonResponse, DaemonStatusPayload, ErrorPayload,
    EventEnvelope, EventId, MessageCompletedPayload, MessageDeltaPayload, ModelDescriptor,
    ModelsListPayload, ProtocolVersion, RequestAcceptedPayload, RequestEnvelope, RequestId,
    ResponseEnvelope, SessionEventPayload, SessionId, Timestamp, UiPreferencesPayload,
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
    sessions: HashMap<SessionId, SessionRecord>,
    agent_names: HashMap<AgentId, String>,
    agent_models: HashMap<AgentId, String>,
    ui_preferences: serde_json::Value,
}

impl Runtime {
    /// Creates a runtime with the supplied model provider.
    pub fn new(options: RuntimeOptions, provider: Box<dyn ModelProvider>) -> Self {
        let ui_preferences = options.ui_preferences.clone();
        let mut agent_names = HashMap::new();
        agent_names.insert(AgentId::from("main"), "CADIS".to_owned());

        let mut agent_models = HashMap::new();
        agent_models.insert(AgentId::from("main"), options.model_provider.clone());

        Self {
            options,
            provider,
            started_at: Instant::now(),
            next_event: 1,
            next_session: 1,
            sessions: HashMap::new(),
            agent_names,
            agent_models,
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
            ClientRequest::MessageSend(request) => {
                self.handle_message(request_id, request.session_id, request.content)
            }
            ClientRequest::AgentRename(request) => {
                let display_name = normalize_agent_name(&request.display_name, &request.agent_id);
                self.agent_names
                    .insert(request.agent_id.clone(), display_name.clone());
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
                self.agent_models
                    .insert(request.agent_id.clone(), request.model.clone());
                let event = self.event(
                    None,
                    CadisEvent::AgentModelChanged(AgentModelChangedPayload {
                        agent_id: request.agent_id,
                        model: request.model,
                    }),
                );
                self.accept(request_id, vec![event])
            }
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
            | ClientRequest::AgentList(_)
            | ClientRequest::AgentSpawn(_)
            | ClientRequest::AgentKill(_)
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
        session_id: Option<SessionId>,
        content: String,
    ) -> RequestOutcome {
        let (session_id, mut events) = match session_id {
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
            CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                agent_id: AgentId::from("main"),
                status: AgentStatus::Running,
                task: Some(content.clone()),
            }),
        ));

        match self.provider.chat(&content) {
            Ok(deltas) => {
                for delta in deltas {
                    events.push(self.session_event(
                        session_id.clone(),
                        CadisEvent::MessageDelta(MessageDeltaPayload {
                            delta,
                            content_kind: ContentKind::Chat,
                        }),
                    ));
                }

                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::MessageCompleted(MessageCompletedPayload {
                        content_kind: ContentKind::Chat,
                    }),
                ));
                events.push(self.session_event(
                    session_id.clone(),
                    CadisEvent::AgentStatusChanged(AgentStatusChangedPayload {
                        agent_id: AgentId::from("main"),
                        status: AgentStatus::Completed,
                        task: None,
                    }),
                ));
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
        AgentModelSetRequest, AgentRenameRequest, ClientId, EmptyPayload, MessageSendRequest,
        RequestId, ServerFrame, SessionCreateRequest,
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
}
