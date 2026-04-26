//! Typed protocol contract between CADIS clients and `cadisd`.
//!
//! The protocol is intentionally JSON-friendly. Envelopes serialize with a
//! dot-separated `type` field and a typed `payload` object so CLI, HUD,
//! Telegram, and tests can share the same daemon contract.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Current CADIS protocol version.
pub const CURRENT_PROTOCOL_VERSION: &str = "0.1";

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates a new identifier.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// Protocol version identifier.
    ProtocolVersion
);
string_id!(
    /// Client request identifier.
    RequestId
);
string_id!(
    /// Client process or surface identifier.
    ClientId
);
string_id!(
    /// Daemon event identifier.
    EventId
);
string_id!(
    /// CADIS session identifier.
    SessionId
);
string_id!(
    /// CADIS agent identifier.
    AgentId
);
string_id!(
    /// CADIS registered workspace identifier.
    WorkspaceId
);
string_id!(
    /// CADIS workspace grant identifier.
    WorkspaceGrantId
);
string_id!(
    /// CADIS tool call identifier.
    ToolCallId
);
string_id!(
    /// CADIS approval identifier.
    ApprovalId
);

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::current()
    }
}

impl ProtocolVersion {
    /// Returns the current protocol version.
    pub fn current() -> Self {
        Self::new(CURRENT_PROTOCOL_VERSION)
    }

    /// Ensures this protocol version is supported by the current crate.
    pub fn ensure_supported(&self) -> Result<(), ProtocolError> {
        if self.as_str() == CURRENT_PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedVersion {
                expected: CURRENT_PROTOCOL_VERSION,
                actual: self.as_str().to_owned(),
            })
        }
    }
}

/// UTC timestamp used by protocol event envelopes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(String);

impl Timestamp {
    /// Creates a timestamp from an RFC3339-style UTC string.
    pub fn new_utc(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if is_utc_timestamp(&value) {
            Ok(Self(value))
        } else {
            Err(ProtocolError::InvalidTimestamp { value })
        }
    }

    /// Returns the timestamp as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new_utc(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_utc_timestamp(value: &str) -> bool {
    value.len() >= "2026-04-26T00:00:00Z".len() && value.contains('T') && value.ends_with('Z')
}

/// Errors emitted by protocol validation helpers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// Protocol version does not match this crate.
    UnsupportedVersion {
        /// Supported protocol version.
        expected: &'static str,
        /// Version supplied by the client or event source.
        actual: String,
    },
    /// Timestamp is not a UTC RFC3339-style value.
    InvalidTimestamp {
        /// Invalid timestamp string.
        value: String,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { expected, actual } => {
                write!(
                    formatter,
                    "unsupported protocol version {actual}; expected {expected}"
                )
            }
            Self::InvalidTimestamp { value } => {
                write!(formatter, "invalid UTC timestamp {value}")
            }
        }
    }
}

impl Error for ProtocolError {}

/// Request envelope sent by a client to `cadisd`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RequestEnvelope {
    /// Protocol version used by the client.
    pub protocol_version: ProtocolVersion,
    /// Client-generated request ID.
    pub request_id: RequestId,
    /// Client or surface ID.
    pub client_id: ClientId,
    /// Typed client request.
    #[serde(flatten)]
    pub request: ClientRequest,
}

impl RequestEnvelope {
    /// Creates a request envelope with the current protocol version.
    pub fn new(request_id: RequestId, client_id: ClientId, request: ClientRequest) -> Self {
        Self {
            protocol_version: ProtocolVersion::current(),
            request_id,
            client_id,
            request,
        }
    }
}

/// Event envelope emitted by `cadisd`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct EventEnvelope {
    /// Protocol version used by the daemon.
    pub protocol_version: ProtocolVersion,
    /// Daemon-generated event ID.
    pub event_id: EventId,
    /// UTC event timestamp.
    pub timestamp: Timestamp,
    /// Source component, usually `cadisd`.
    pub source: String,
    /// Session ID when the event belongs to a session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Typed daemon event.
    #[serde(flatten)]
    pub event: CadisEvent,
}

impl EventEnvelope {
    /// Creates an event envelope with the current protocol version.
    pub fn new(
        event_id: EventId,
        timestamp: Timestamp,
        source: impl Into<String>,
        session_id: Option<SessionId>,
        event: CadisEvent,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::current(),
            event_id,
            timestamp,
            source: source.into(),
            session_id,
            event,
        }
    }
}

/// Response envelope emitted by `cadisd` for one client request.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ResponseEnvelope {
    /// Protocol version used by the daemon.
    pub protocol_version: ProtocolVersion,
    /// Request ID this response belongs to.
    pub request_id: RequestId,
    /// Typed immediate daemon response.
    #[serde(flatten)]
    pub response: DaemonResponse,
}

impl ResponseEnvelope {
    /// Creates a response envelope with the current protocol version.
    pub fn new(request_id: RequestId, response: DaemonResponse) -> Self {
        Self {
            protocol_version: ProtocolVersion::current(),
            request_id,
            response,
        }
    }
}

/// Newline-delimited JSON frame sent by `cadisd`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "frame", content = "payload", rename_all = "snake_case")]
pub enum ServerFrame {
    /// Immediate response to a request.
    Response(ResponseEnvelope),
    /// Runtime event emitted by the daemon.
    Event(EventEnvelope),
}

/// Immediate response returned for request handling failures or acknowledgements.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonResponse {
    /// Request was accepted and follow-up state will arrive through events.
    #[serde(rename = "request.accepted")]
    RequestAccepted(RequestAcceptedPayload),
    /// Current daemon status.
    #[serde(rename = "daemon.status.response")]
    DaemonStatus(DaemonStatusPayload),
    /// Request was rejected before execution.
    #[serde(rename = "request.rejected")]
    RequestRejected(ErrorPayload),
}

/// Acknowledgement payload for accepted requests.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RequestAcceptedPayload {
    /// Request ID that was accepted.
    pub request_id: RequestId,
}

/// Current daemon health and runtime status.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DaemonStatusPayload {
    /// Human-readable daemon state.
    pub status: String,
    /// CADIS binary version.
    pub version: String,
    /// Protocol version served by the daemon.
    pub protocol_version: ProtocolVersion,
    /// Local CADIS state directory.
    pub cadis_home: String,
    /// Local socket path when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,
    /// Number of sessions known by this daemon process.
    pub sessions: usize,
    /// Configured model provider label.
    pub model_provider: String,
    /// Daemon uptime in seconds.
    pub uptime_seconds: u64,
}

/// Machine-readable error payload for responses and error events.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ErrorPayload {
    /// Stable error code.
    pub code: String,
    /// Redacted human-readable message.
    pub message: String,
    /// Whether retrying may be useful.
    pub retryable: bool,
}

/// Empty payload used by request and event variants that need no fields.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct EmptyPayload {}

/// Client requests supported by protocol version 0.1.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientRequest {
    /// Subscribe to daemon runtime events.
    #[serde(rename = "events.subscribe")]
    EventsSubscribe(EventSubscriptionRequest),
    /// Request a daemon-owned runtime state snapshot.
    #[serde(rename = "events.snapshot")]
    EventsSnapshot(EventsSnapshotRequest),
    /// Ask daemon for health and runtime status.
    #[serde(rename = "daemon.status")]
    DaemonStatus(EmptyPayload),
    /// Create a new session.
    #[serde(rename = "session.create")]
    SessionCreate(SessionCreateRequest),
    /// Cancel a session.
    #[serde(rename = "session.cancel")]
    SessionCancel(SessionTargetRequest),
    /// Subscribe to a session stream.
    #[serde(rename = "session.subscribe")]
    SessionSubscribe(SessionSubscriptionRequest),
    /// Unsubscribe from a session stream.
    #[serde(rename = "session.unsubscribe")]
    SessionUnsubscribe(SessionTargetRequest),
    /// Send a user message.
    #[serde(rename = "message.send")]
    MessageSend(MessageSendRequest),
    /// Request daemon-owned tool execution.
    #[serde(rename = "tool.call")]
    ToolCall(ToolCallRequest),
    /// Respond to a pending approval.
    #[serde(rename = "approval.respond")]
    ApprovalRespond(ApprovalResponseRequest),
    /// List known agents.
    #[serde(rename = "agent.list")]
    AgentList(EmptyPayload),
    /// Rename an agent display name.
    #[serde(rename = "agent.rename")]
    AgentRename(AgentRenameRequest),
    /// Set an agent model.
    #[serde(rename = "agent.model.set")]
    AgentModelSet(AgentModelSetRequest),
    /// Spawn an agent.
    #[serde(rename = "agent.spawn")]
    AgentSpawn(AgentSpawnRequest),
    /// Kill an agent.
    #[serde(rename = "agent.kill")]
    AgentKill(AgentTargetRequest),
    /// List registered project workspaces and active grants.
    #[serde(rename = "workspace.list")]
    WorkspaceList(WorkspaceListRequest),
    /// Register or replace a project workspace.
    #[serde(rename = "workspace.register")]
    WorkspaceRegister(WorkspaceRegisterRequest),
    /// Grant an agent access to a registered workspace.
    #[serde(rename = "workspace.grant")]
    WorkspaceGrant(WorkspaceGrantRequest),
    /// Revoke one or more workspace grants.
    #[serde(rename = "workspace.revoke")]
    WorkspaceRevoke(WorkspaceRevokeRequest),
    /// Check workspace registry, root, and grant health.
    #[serde(rename = "workspace.doctor")]
    WorkspaceDoctor(WorkspaceDoctorRequest),
    /// Tail a worker log stream.
    #[serde(rename = "worker.tail")]
    WorkerTail(WorkerTailRequest),
    /// List available model descriptors.
    #[serde(rename = "models.list")]
    ModelsList(EmptyPayload),
    /// Get daemon-owned UI preferences.
    #[serde(rename = "ui.preferences.get")]
    UiPreferencesGet(EmptyPayload),
    /// Patch daemon-owned UI preferences.
    #[serde(rename = "ui.preferences.set")]
    UiPreferencesSet(UiPreferencesSetRequest),
    /// Preview voice output.
    #[serde(rename = "voice.preview")]
    VoicePreview(VoicePreviewRequest),
    /// Stop current voice output.
    #[serde(rename = "voice.stop")]
    VoiceStop(EmptyPayload),
    /// Reload daemon configuration.
    #[serde(rename = "config.reload")]
    ConfigReload(EmptyPayload),
}

/// Payload for subscribing to daemon runtime events.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct EventSubscriptionRequest {
    /// Optional event ID. Buffered replay starts after this ID when still retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_event_id: Option<EventId>,
    /// Maximum buffered events to replay before live events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_limit: Option<u32>,
    /// Whether the daemon should send a current runtime snapshot before replay.
    #[serde(default = "default_true")]
    pub include_snapshot: bool,
}

impl Default for EventSubscriptionRequest {
    fn default() -> Self {
        Self {
            since_event_id: None,
            replay_limit: Some(128),
            include_snapshot: true,
        }
    }
}

/// Payload for requesting a one-shot daemon-owned runtime state snapshot.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct EventsSnapshotRequest {}

fn default_true() -> bool {
    true
}

/// Payload for creating a session.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct SessionCreateRequest {
    /// Optional user-facing session title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional working directory for session-scoped work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// Payload that targets one session.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SessionTargetRequest {
    /// Target session ID.
    pub session_id: SessionId,
}

/// Payload for subscribing to one session's event stream.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SessionSubscriptionRequest {
    /// Target session ID.
    pub session_id: SessionId,
    /// Optional event ID. Buffered replay starts after this ID when still retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_event_id: Option<EventId>,
    /// Maximum buffered matching events to replay before live events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_limit: Option<u32>,
    /// Whether the daemon should send the current session state before replay.
    #[serde(default = "default_true")]
    pub include_snapshot: bool,
}

/// Payload for a user message.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MessageSendRequest {
    /// Optional existing session ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Optional agent target selected by the client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_agent_id: Option<AgentId>,
    /// Message content.
    pub content: String,
    /// Content routing hint.
    pub content_kind: ContentKind,
}

/// Payload for an approval response.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ApprovalResponseRequest {
    /// Target approval ID.
    pub approval_id: ApprovalId,
    /// Approval decision.
    pub decision: ApprovalDecision,
    /// Optional redacted reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Payload for requesting daemon-owned tool execution.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ToolCallRequest {
    /// Optional session this tool call belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Optional agent context for agent-scoped workspace grants.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Stable tool name, for example `file.read`.
    pub tool_name: String,
    /// Structured tool input.
    #[serde(default)]
    pub input: serde_json::Value,
}

/// Payload for agent rename.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentRenameRequest {
    /// Target agent ID.
    pub agent_id: AgentId,
    /// New display name.
    pub display_name: String,
}

/// Payload for selecting an agent model.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentModelSetRequest {
    /// Target agent ID.
    pub agent_id: AgentId,
    /// Provider/model identifier.
    pub model: String,
}

/// Payload for agent spawn.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentSpawnRequest {
    /// Agent role identifier.
    pub role: String,
    /// Optional parent agent that requested or owns this child agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    /// Optional display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Optional provider/model identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Payload that targets one agent.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentTargetRequest {
    /// Target agent ID.
    pub agent_id: AgentId,
}

/// Payload for tailing worker output.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkerTailRequest {
    /// Worker identifier.
    pub worker_id: String,
    /// Optional number of recent lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
}

/// Payload for listing registered workspaces.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceListRequest {
    /// Include active grants in the response.
    #[serde(default)]
    pub include_grants: bool,
}

/// Payload for registering a project workspace root.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceRegisterRequest {
    /// Stable workspace identifier.
    pub workspace_id: WorkspaceId,
    /// Workspace category.
    pub kind: WorkspaceKind,
    /// Filesystem root supplied by the client.
    pub root: String,
    /// Human or routing aliases for this workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Version control system label, for example `git` or `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcs: Option<String>,
    /// Whether the user considers this workspace trusted.
    #[serde(default)]
    pub trusted: bool,
    /// Relative worktree root under the project workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<String>,
    /// Relative artifact root under the project workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_root: Option<String>,
}

/// Payload for granting workspace access.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceGrantRequest {
    /// Agent receiving access. Missing means the default local runtime context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Registered workspace being granted.
    pub workspace_id: WorkspaceId,
    /// Access modes granted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<WorkspaceAccess>,
    /// Optional UTC expiration timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    /// Source of the grant, for example `user`, `policy`, or `worker_spawn`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Payload for revoking workspace grants.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceRevokeRequest {
    /// Revoke a specific grant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<WorkspaceGrantId>,
    /// Revoke grants for a workspace when grant_id is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
    /// Optionally narrow workspace revocation to one agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
}

/// Payload for workspace registry health checks.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceDoctorRequest {
    /// Check a registered workspace by ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<WorkspaceId>,
    /// Check an explicit root supplied by the client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// Payload for patching UI preferences.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct UiPreferencesSetRequest {
    /// Partial preference object owned by `cadisd`.
    pub patch: serde_json::Value,
}

/// Voice preview payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct VoicePreviewRequest {
    /// Text to preview.
    pub text: String,
    /// Optional voice preferences for the preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefs: Option<VoicePreferences>,
}

/// Voice preference payload shared by UI and daemon.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct VoicePreferences {
    /// Voice identifier.
    pub voice_id: String,
    /// Speaking rate adjustment.
    pub rate: i16,
    /// Pitch adjustment.
    pub pitch: i16,
    /// Volume adjustment.
    pub volume: i16,
}

/// Daemon events emitted by protocol version 0.1.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum CadisEvent {
    /// Daemon started.
    #[serde(rename = "daemon.started")]
    DaemonStarted(EmptyPayload),
    /// Daemon is stopping.
    #[serde(rename = "daemon.stopping")]
    DaemonStopping(EmptyPayload),
    /// Daemon error.
    #[serde(rename = "daemon.error")]
    DaemonError(ErrorPayload),
    /// Session started.
    #[serde(rename = "session.started")]
    SessionStarted(SessionEventPayload),
    /// Session state changed.
    #[serde(rename = "session.updated")]
    SessionUpdated(SessionEventPayload),
    /// Session completed.
    #[serde(rename = "session.completed")]
    SessionCompleted(SessionEventPayload),
    /// Session failed.
    #[serde(rename = "session.failed")]
    SessionFailed(ErrorPayload),
    /// Streaming message delta.
    #[serde(rename = "message.delta")]
    MessageDelta(MessageDeltaPayload),
    /// Message completed.
    #[serde(rename = "message.completed")]
    MessageCompleted(MessageCompletedPayload),
    /// Agent spawned.
    #[serde(rename = "agent.spawned")]
    AgentSpawned(AgentEventPayload),
    /// Agent roster snapshot.
    #[serde(rename = "agent.list.response")]
    AgentListResponse(AgentListPayload),
    /// Agent renamed.
    #[serde(rename = "agent.renamed")]
    AgentRenamed(AgentRenamedPayload),
    /// Agent model changed.
    #[serde(rename = "agent.model.changed")]
    AgentModelChanged(AgentModelChangedPayload),
    /// Agent status changed.
    #[serde(rename = "agent.status.changed")]
    AgentStatusChanged(AgentStatusChangedPayload),
    /// Agent completed a task.
    #[serde(rename = "agent.completed")]
    AgentCompleted(AgentEventPayload),
    /// Workspace registry snapshot.
    #[serde(rename = "workspace.list.response")]
    WorkspaceListResponse(WorkspaceListPayload),
    /// Workspace was registered.
    #[serde(rename = "workspace.registered")]
    WorkspaceRegistered(WorkspaceRecordPayload),
    /// Workspace grant was created.
    #[serde(rename = "workspace.grant.created")]
    WorkspaceGrantCreated(WorkspaceGrantPayload),
    /// Workspace grant was revoked.
    #[serde(rename = "workspace.grant.revoked")]
    WorkspaceGrantRevoked(WorkspaceGrantPayload),
    /// Workspace doctor result.
    #[serde(rename = "workspace.doctor.response")]
    WorkspaceDoctorResponse(WorkspaceDoctorPayload),
    /// Model list response.
    #[serde(rename = "models.list.response")]
    ModelsListResponse(ModelsListPayload),
    /// UI preferences changed.
    #[serde(rename = "ui.preferences.updated")]
    UiPreferencesUpdated(UiPreferencesPayload),
    /// Orchestrator routed a user request to an agent.
    #[serde(rename = "orchestrator.route")]
    OrchestratorRoute(OrchestratorRoutePayload),
    /// Tool was requested.
    #[serde(rename = "tool.requested")]
    ToolRequested(ToolEventPayload),
    /// Tool started.
    #[serde(rename = "tool.started")]
    ToolStarted(ToolEventPayload),
    /// Tool completed.
    #[serde(rename = "tool.completed")]
    ToolCompleted(ToolEventPayload),
    /// Tool failed.
    #[serde(rename = "tool.failed")]
    ToolFailed(ToolFailedPayload),
    /// Approval is required.
    #[serde(rename = "approval.requested")]
    ApprovalRequested(ApprovalRequestPayload),
    /// Approval was resolved.
    #[serde(rename = "approval.resolved")]
    ApprovalResolved(ApprovalResolvedPayload),
    /// Worker started.
    #[serde(rename = "worker.started")]
    WorkerStarted(WorkerEventPayload),
    /// Worker log delta.
    #[serde(rename = "worker.log.delta")]
    WorkerLogDelta(WorkerLogDeltaPayload),
    /// Worker completed.
    #[serde(rename = "worker.completed")]
    WorkerCompleted(WorkerEventPayload),
    /// Patch was created.
    #[serde(rename = "patch.created")]
    PatchCreated(PatchCreatedPayload),
    /// Test result emitted.
    #[serde(rename = "test.result")]
    TestResult(TestResultPayload),
    /// Voice preview started.
    #[serde(rename = "voice.preview.started")]
    VoicePreviewStarted(EmptyPayload),
    /// Voice preview completed.
    #[serde(rename = "voice.preview.completed")]
    VoicePreviewCompleted(EmptyPayload),
    /// Voice preview failed.
    #[serde(rename = "voice.preview.failed")]
    VoicePreviewFailed(ErrorPayload),
    /// Voice playback started.
    #[serde(rename = "voice.started")]
    VoiceStarted(EmptyPayload),
    /// Voice playback completed.
    #[serde(rename = "voice.completed")]
    VoiceCompleted(EmptyPayload),
}

/// Session event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SessionEventPayload {
    /// Session ID.
    pub session_id: SessionId,
    /// Optional display title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Message delta payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MessageDeltaPayload {
    /// Delta text.
    pub delta: String,
    /// Content routing kind.
    pub content_kind: ContentKind,
    /// Agent that produced this content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Display name for the producing agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Provider/model metadata for this model invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInvocationPayload>,
}

/// Message completion payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MessageCompletedPayload {
    /// Final content kind.
    pub content_kind: ContentKind,
    /// Optional final content snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Agent that produced this content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Display name for the producing agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Provider/model metadata for this model invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInvocationPayload>,
}

/// Provider/model metadata attached to model-backed events.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ModelInvocationPayload {
    /// Requested provider/model ID, when supplied by an agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    /// Provider that actually served the request.
    pub effective_provider: String,
    /// Model that actually served the request.
    pub effective_model: String,
    /// Whether this response came from a fallback provider.
    #[serde(default)]
    pub fallback: bool,
    /// Redacted fallback reason, when fallback occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

/// Agent event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentEventPayload {
    /// Agent ID.
    pub agent_id: AgentId,
    /// Agent role identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// User-facing display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Parent agent ID for child/subagents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    /// Provider/model identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Current lifecycle status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
}

/// Agent roster snapshot payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentListPayload {
    /// Known agents.
    pub agents: Vec<AgentEventPayload>,
}

/// Agent rename event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentRenamedPayload {
    /// Agent ID.
    pub agent_id: AgentId,
    /// New display name.
    pub display_name: String,
}

/// Agent model change event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentModelChangedPayload {
    /// Agent ID.
    pub agent_id: AgentId,
    /// Provider/model identifier.
    pub model: String,
}

/// Agent status change event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AgentStatusChangedPayload {
    /// Agent ID.
    pub agent_id: AgentId,
    /// New status.
    pub status: AgentStatus,
    /// Optional current task summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// Workspace registry snapshot payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceListPayload {
    /// Registered workspaces.
    pub workspaces: Vec<WorkspaceRecordPayload>,
    /// Active grants when requested.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<WorkspaceGrantPayload>,
}

/// Registered workspace payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceRecordPayload {
    /// Stable workspace ID.
    pub workspace_id: WorkspaceId,
    /// Workspace category.
    pub kind: WorkspaceKind,
    /// Canonical root path.
    pub root: String,
    /// Human or routing aliases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Version control system label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcs: Option<String>,
    /// Whether this workspace is trusted by the user.
    #[serde(default)]
    pub trusted: bool,
    /// Relative worktree root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<String>,
    /// Relative artifact root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_root: Option<String>,
}

/// Active workspace grant payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceGrantPayload {
    /// Grant ID.
    pub grant_id: WorkspaceGrantId,
    /// Agent receiving access. Missing means the default local runtime context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Workspace being granted.
    pub workspace_id: WorkspaceId,
    /// Canonical root path covered by the grant.
    pub root: String,
    /// Granted access modes.
    pub access: Vec<WorkspaceAccess>,
    /// Optional UTC expiration timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    /// Source of the grant.
    pub source: String,
}

/// Workspace doctor response payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceDoctorPayload {
    /// Checks performed by the daemon.
    pub checks: Vec<WorkspaceDoctorCheck>,
}

/// One workspace doctor check.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkspaceDoctorCheck {
    /// Stable check name.
    pub name: String,
    /// Result status, for example `ok`, `warn`, or `error`.
    pub status: String,
    /// Human-readable result.
    pub message: String,
}

/// Model list payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ModelsListPayload {
    /// Available models.
    pub models: Vec<ModelDescriptor>,
}

/// Model descriptor exposed to clients.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ModelDescriptor {
    /// Provider name.
    pub provider: String,
    /// Model identifier.
    pub model: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Capability labels.
    pub capabilities: Vec<String>,
    /// Conservative readiness state for this provider/model entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<ModelReadiness>,
    /// Provider that will actually serve requests when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_provider: Option<String>,
    /// Model that will actually serve requests when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_model: Option<String>,
    /// Whether this entry is a local fallback rather than a real provider.
    #[serde(default)]
    pub fallback: bool,
}

/// Conservative model catalog readiness state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelReadiness {
    /// Entry is usable without known additional setup.
    Ready,
    /// Entry is usable as a local fallback, not a real model provider.
    Fallback,
    /// Entry requires daemon, credential, login, or local service setup.
    RequiresConfiguration,
    /// Entry is known unavailable in the current daemon configuration.
    Unavailable,
}

/// UI preference event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct UiPreferencesPayload {
    /// Full or partial preference object.
    pub preferences: serde_json::Value,
}

/// Orchestrator routing decision payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct OrchestratorRoutePayload {
    /// Route identifier.
    pub id: String,
    /// Source surface or subsystem.
    pub source: String,
    /// Target agent ID.
    pub target_agent_id: AgentId,
    /// Target agent display name.
    pub target_agent_name: String,
    /// Redacted routing reason.
    pub reason: String,
}

/// Generic tool event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ToolEventPayload {
    /// Tool call ID.
    pub tool_call_id: ToolCallId,
    /// Tool name.
    pub tool_name: String,
    /// Optional redacted summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Risk class assigned by daemon policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_class: Option<RiskClass>,
    /// Structured redacted result for completed tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

/// Tool failure payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ToolFailedPayload {
    /// Tool call ID.
    pub tool_call_id: ToolCallId,
    /// Tool name.
    pub tool_name: String,
    /// Redacted error.
    pub error: ErrorPayload,
    /// Risk class assigned by daemon policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_class: Option<RiskClass>,
}

/// Approval request payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ApprovalRequestPayload {
    /// Approval ID.
    pub approval_id: ApprovalId,
    /// Session ID.
    pub session_id: SessionId,
    /// Tool call ID.
    pub tool_call_id: ToolCallId,
    /// Risk class.
    pub risk_class: RiskClass,
    /// UI title.
    pub title: String,
    /// Redacted risk summary.
    pub summary: String,
    /// Redacted command or operation details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Workspace affected by the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Expiration timestamp.
    pub expires_at: Timestamp,
}

/// Approval resolution payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ApprovalResolvedPayload {
    /// Approval ID.
    pub approval_id: ApprovalId,
    /// Final decision.
    pub decision: ApprovalDecision,
}

/// Worker event payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkerEventPayload {
    /// Worker ID.
    pub worker_id: String,
    /// Owning or reporting agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Parent agent for tree display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    /// Worker status label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Optional CLI or runner label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    /// Optional working directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Redacted worker summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Non-destructive worktree intent and path planning metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorkerWorktreeIntent>,
    /// Artifact paths where worker outputs are expected to be written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<WorkerArtifactLocations>,
}

/// Planned or actual worker worktree metadata.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkerWorktreeIntent {
    /// Workspace registry ID, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Canonical project root, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Root directory where CADIS worktrees for this workspace live.
    pub worktree_root: String,
    /// Intended or actual worktree path for this worker.
    pub worktree_path: String,
    /// Intended or actual branch name for this worker.
    pub branch_name: String,
    /// Base ref for worktree branch creation, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Current lifecycle state of the worktree metadata.
    pub state: WorkerWorktreeState,
    /// Policy for retaining or removing the worktree.
    pub cleanup_policy: WorkerWorktreeCleanupPolicy,
}

/// Worker worktree lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerWorktreeState {
    /// Worktree is planned but has not been created by the daemon.
    Planned,
    /// Worktree exists and is assigned to the worker.
    Active,
    /// Worktree should remain available for user review.
    ReviewPending,
    /// Worktree cleanup has been requested.
    CleanupPending,
    /// Worktree was removed by CADIS.
    Removed,
}

/// Worker worktree retention policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerWorktreeCleanupPolicy {
    /// Keep until an explicit user or policy action removes it.
    Explicit,
    /// Remove automatically after successful apply.
    AfterApply,
    /// Remove automatically when the worker reaches a terminal state.
    OnCompletion,
}

/// Expected artifact locations for worker outputs.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WorkerArtifactLocations {
    /// Worker artifact root.
    pub root: String,
    /// Patch/diff artifact path.
    pub patch: String,
    /// Test report artifact path.
    pub test_report: String,
    /// Worker summary artifact path.
    pub summary: String,
    /// Changed-files manifest path.
    pub changed_files: String,
    /// Candidate memory updates path.
    pub memory_candidates: String,
}

/// Worker log delta payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WorkerLogDeltaPayload {
    /// Worker ID.
    pub worker_id: String,
    /// Log content delta.
    pub delta: String,
    /// Owning or reporting agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Parent agent for tree display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
}

/// Patch creation payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PatchCreatedPayload {
    /// Patch ID.
    pub patch_id: String,
    /// Redacted patch summary.
    pub summary: String,
}

/// Test result payload.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TestResultPayload {
    /// Test status.
    pub status: TestStatus,
    /// Redacted summary.
    pub summary: String,
}

/// Content routing kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// Conversational chat text.
    Chat,
    /// Short summary text.
    Summary,
    /// Source code.
    Code,
    /// Patch or diff content.
    Diff,
    /// Terminal log content.
    TerminalLog,
    /// Test result content.
    TestResult,
    /// Approval content.
    Approval,
    /// Error content.
    Error,
}

/// Tool or operation risk class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum RiskClass {
    /// Safe read-only operation.
    #[serde(rename = "safe-read")]
    SafeRead,
    /// Workspace-local edit operation.
    #[serde(rename = "workspace-edit")]
    WorkspaceEdit,
    /// Network access operation.
    #[serde(rename = "network-access")]
    NetworkAccess,
    /// Secret access operation.
    #[serde(rename = "secret-access")]
    SecretAccess,
    /// System-changing operation.
    #[serde(rename = "system-change")]
    SystemChange,
    /// Dangerous delete operation.
    #[serde(rename = "dangerous-delete")]
    DangerousDelete,
    /// Outside-workspace operation.
    #[serde(rename = "outside-workspace")]
    OutsideWorkspace,
    /// Push to main branch operation.
    #[serde(rename = "git-push-main")]
    GitPushMain,
    /// Force push operation.
    #[serde(rename = "git-force-push")]
    GitForcePush,
    /// Sudo/system operation.
    #[serde(rename = "sudo-system")]
    SudoSystem,
}

/// Workspace category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// User project checkout or project root.
    Project,
    /// Documents or notes tree.
    Documents,
    /// Ephemeral sandbox root.
    Sandbox,
    /// Worker git worktree.
    Worktree,
}

/// Workspace grant access mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccess {
    /// Read files and inspect status.
    Read,
    /// Write files under the workspace root.
    Write,
    /// Execute commands under the workspace root.
    Exec,
    /// Administrative workspace operations.
    Admin,
}

/// Approval decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Approval accepted.
    Approved,
    /// Approval denied.
    Denied,
}

/// Agent status exposed to clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Agent is idle.
    Idle,
    /// Agent is running.
    Running,
    /// Agent is waiting on approval.
    WaitingApproval,
    /// Agent completed work.
    Completed,
    /// Agent failed.
    Failed,
}

/// Test status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    /// Tests passed.
    Passed,
    /// Tests failed.
    Failed,
    /// Tests were skipped.
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_envelope_matches_documented_shape() {
        let envelope = RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::AgentRename(AgentRenameRequest {
                agent_id: AgentId::from("main"),
                display_name: "CADIS".to_owned(),
            }),
        );

        let value = serde_json::to_value(&envelope).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "request_id": "req_1",
                "client_id": "cli_1",
                "type": "agent.rename",
                "payload": {
                    "agent_id": "main",
                    "display_name": "CADIS"
                }
            })
        );
    }

    #[test]
    fn event_subscription_request_matches_documented_shape() {
        let envelope = RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("hud_1"),
            ClientRequest::EventsSubscribe(EventSubscriptionRequest {
                since_event_id: Some(EventId::from("evt_000007")),
                replay_limit: Some(32),
                include_snapshot: true,
            }),
        );

        let value = serde_json::to_value(&envelope).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "request_id": "req_1",
                "client_id": "hud_1",
                "type": "events.subscribe",
                "payload": {
                    "since_event_id": "evt_000007",
                    "replay_limit": 32,
                    "include_snapshot": true
                }
            })
        );
    }

    #[test]
    fn session_subscription_request_matches_documented_shape() {
        let envelope = RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::SessionSubscribe(SessionSubscriptionRequest {
                session_id: SessionId::from("ses_1"),
                since_event_id: Some(EventId::from("evt_000010")),
                replay_limit: Some(16),
                include_snapshot: true,
            }),
        );

        let value = serde_json::to_value(&envelope).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "request_id": "req_1",
                "client_id": "cli_1",
                "type": "session.subscribe",
                "payload": {
                    "session_id": "ses_1",
                    "since_event_id": "evt_000010",
                    "replay_limit": 16,
                    "include_snapshot": true
                }
            })
        );
    }

    #[test]
    fn tool_call_request_matches_documented_shape() {
        let envelope = RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::ToolCall(ToolCallRequest {
                session_id: Some(SessionId::from("ses_1")),
                agent_id: Some(AgentId::from("codex")),
                tool_name: "file.read".to_owned(),
                input: json!({
                    "workspace": "/home/user/project",
                    "path": "README.md"
                }),
            }),
        );

        let value = serde_json::to_value(&envelope).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "request_id": "req_1",
                "client_id": "cli_1",
                "type": "tool.call",
                "payload": {
                    "session_id": "ses_1",
                    "agent_id": "codex",
                    "tool_name": "file.read",
                    "input": {
                        "workspace": "/home/user/project",
                        "path": "README.md"
                    }
                }
            })
        );
    }

    #[test]
    fn workspace_register_request_matches_documented_shape() {
        let envelope = RequestEnvelope::new(
            RequestId::from("req_1"),
            ClientId::from("cli_1"),
            ClientRequest::WorkspaceRegister(WorkspaceRegisterRequest {
                workspace_id: WorkspaceId::from("example-project"),
                kind: WorkspaceKind::Project,
                root: "/home/user/project".to_owned(),
                aliases: vec!["example".to_owned()],
                vcs: Some("git".to_owned()),
                trusted: true,
                worktree_root: Some(".cadis/worktrees".to_owned()),
                artifact_root: Some(".cadis/artifacts".to_owned()),
            }),
        );

        let value = serde_json::to_value(&envelope).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "request_id": "req_1",
                "client_id": "cli_1",
                "type": "workspace.register",
                "payload": {
                    "workspace_id": "example-project",
                    "kind": "project",
                    "root": "/home/user/project",
                    "aliases": ["example"],
                    "vcs": "git",
                    "trusted": true,
                    "worktree_root": ".cadis/worktrees",
                    "artifact_root": ".cadis/artifacts"
                }
            })
        );
    }

    #[test]
    fn workspace_grant_event_matches_documented_shape() {
        let envelope = EventEnvelope::new(
            EventId::from("evt_1"),
            Timestamp::new_utc("2026-04-26T00:00:00Z").expect("timestamp should be UTC"),
            "cadisd",
            None,
            CadisEvent::WorkspaceGrantCreated(WorkspaceGrantPayload {
                grant_id: WorkspaceGrantId::from("grant_000001"),
                agent_id: Some(AgentId::from("codex")),
                workspace_id: WorkspaceId::from("example-project"),
                root: "/home/user/project".to_owned(),
                access: vec![WorkspaceAccess::Read, WorkspaceAccess::Exec],
                expires_at: None,
                source: "user".to_owned(),
            }),
        );

        let value = serde_json::to_value(&envelope).expect("event should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "event_id": "evt_1",
                "timestamp": "2026-04-26T00:00:00Z",
                "source": "cadisd",
                "type": "workspace.grant.created",
                "payload": {
                    "grant_id": "grant_000001",
                    "agent_id": "codex",
                    "workspace_id": "example-project",
                    "root": "/home/user/project",
                    "access": ["read", "exec"],
                    "source": "user"
                }
            })
        );
    }

    #[test]
    fn event_subscription_defaults_include_snapshot() {
        let value = json!({
            "protocol_version": CURRENT_PROTOCOL_VERSION,
            "request_id": "req_1",
            "client_id": "hud_1",
            "type": "events.subscribe",
            "payload": {}
        });

        let envelope =
            serde_json::from_value::<RequestEnvelope>(value).expect("request should parse");

        match envelope.request {
            ClientRequest::EventsSubscribe(payload) => {
                assert!(payload.include_snapshot);
                assert_eq!(payload.since_event_id, None);
                assert_eq!(payload.replay_limit, None);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn session_subscription_defaults_include_snapshot() {
        let value = json!({
            "protocol_version": CURRENT_PROTOCOL_VERSION,
            "request_id": "req_1",
            "client_id": "cli_1",
            "type": "session.subscribe",
            "payload": {
                "session_id": "ses_1"
            }
        });

        let envelope =
            serde_json::from_value::<RequestEnvelope>(value).expect("request should parse");

        match envelope.request {
            ClientRequest::SessionSubscribe(payload) => {
                assert!(payload.include_snapshot);
                assert_eq!(payload.session_id.as_str(), "ses_1");
                assert_eq!(payload.since_event_id, None);
                assert_eq!(payload.replay_limit, None);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn event_envelope_matches_documented_shape() {
        let envelope = EventEnvelope::new(
            EventId::from("evt_1"),
            Timestamp::new_utc("2026-04-26T00:00:00Z").expect("timestamp should be UTC"),
            "cadisd",
            Some(SessionId::from("ses_1")),
            CadisEvent::MessageDelta(MessageDeltaPayload {
                delta: "Halo".to_owned(),
                content_kind: ContentKind::Chat,
                agent_id: None,
                agent_name: None,
                model: None,
            }),
        );

        let value = serde_json::to_value(&envelope).expect("event should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "event_id": "evt_1",
                "timestamp": "2026-04-26T00:00:00Z",
                "source": "cadisd",
                "session_id": "ses_1",
                "type": "message.delta",
                "payload": {
                    "delta": "Halo",
                    "content_kind": "chat"
                }
            })
        );
    }

    #[test]
    fn worker_event_serializes_worktree_and_artifact_planning_metadata() {
        let envelope = EventEnvelope::new(
            EventId::from("evt_2"),
            Timestamp::new_utc("2026-04-26T00:00:00Z").expect("timestamp should be UTC"),
            "cadisd",
            Some(SessionId::from("ses_1")),
            CadisEvent::WorkerStarted(WorkerEventPayload {
                worker_id: "worker_000001".to_owned(),
                agent_id: Some(AgentId::from("coder")),
                parent_agent_id: Some(AgentId::from("main")),
                status: Some("running".to_owned()),
                cli: None,
                cwd: None,
                summary: Some("implement worktree baseline".to_owned()),
                worktree: Some(WorkerWorktreeIntent {
                    workspace_id: Some("cadis".to_owned()),
                    project_root: Some("/home/user/Project/cadis".to_owned()),
                    worktree_root: "/home/user/Project/cadis/.cadis/worktrees".to_owned(),
                    worktree_path: "/home/user/Project/cadis/.cadis/worktrees/worker_000001"
                        .to_owned(),
                    branch_name: "cadis/worker_000001/worktree-baseline".to_owned(),
                    base_ref: Some("HEAD".to_owned()),
                    state: WorkerWorktreeState::Planned,
                    cleanup_policy: WorkerWorktreeCleanupPolicy::Explicit,
                }),
                artifacts: Some(WorkerArtifactLocations {
                    root: "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001"
                        .to_owned(),
                    patch: "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/patch.diff"
                        .to_owned(),
                    test_report:
                        "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/test-report.json"
                            .to_owned(),
                    summary:
                        "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/summary.md"
                            .to_owned(),
                    changed_files:
                        "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/changed-files.json"
                            .to_owned(),
                    memory_candidates:
                        "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/memory-candidates.jsonl"
                            .to_owned(),
                }),
            }),
        );

        let value = serde_json::to_value(&envelope).expect("event should serialize");

        assert_eq!(
            value,
            json!({
                "protocol_version": CURRENT_PROTOCOL_VERSION,
                "event_id": "evt_2",
                "timestamp": "2026-04-26T00:00:00Z",
                "source": "cadisd",
                "session_id": "ses_1",
                "type": "worker.started",
                "payload": {
                    "worker_id": "worker_000001",
                    "agent_id": "coder",
                    "parent_agent_id": "main",
                    "status": "running",
                    "summary": "implement worktree baseline",
                    "worktree": {
                        "workspace_id": "cadis",
                        "project_root": "/home/user/Project/cadis",
                        "worktree_root": "/home/user/Project/cadis/.cadis/worktrees",
                        "worktree_path": "/home/user/Project/cadis/.cadis/worktrees/worker_000001",
                        "branch_name": "cadis/worker_000001/worktree-baseline",
                        "base_ref": "HEAD",
                        "state": "planned",
                        "cleanup_policy": "explicit"
                    },
                    "artifacts": {
                        "root": "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001",
                        "patch": "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/patch.diff",
                        "test_report": "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/test-report.json",
                        "summary": "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/summary.md",
                        "changed_files": "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/changed-files.json",
                        "memory_candidates": "/home/user/.cadis/profiles/default/artifacts/workers/worker_000001/memory-candidates.jsonl"
                    }
                }
            })
        );
    }

    #[test]
    fn worker_event_deserializes_legacy_shape_without_worktree_metadata() {
        let value = json!({
            "protocol_version": CURRENT_PROTOCOL_VERSION,
            "event_id": "evt_3",
            "timestamp": "2026-04-26T00:00:00Z",
            "source": "cadisd",
            "session_id": "ses_1",
            "type": "worker.completed",
            "payload": {
                "worker_id": "worker_000001",
                "agent_id": "coder",
                "status": "completed",
                "summary": "done"
            }
        });

        let envelope =
            serde_json::from_value::<EventEnvelope>(value).expect("legacy event should parse");

        match envelope.event {
            CadisEvent::WorkerCompleted(payload) => {
                assert_eq!(payload.worktree, None);
                assert_eq!(payload.artifacts, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn server_response_frame_matches_transport_shape() {
        let frame = ServerFrame::Response(ResponseEnvelope::new(
            RequestId::from("req_1"),
            DaemonResponse::DaemonStatus(DaemonStatusPayload {
                status: "ok".to_owned(),
                version: "0.1.0".to_owned(),
                protocol_version: ProtocolVersion::current(),
                cadis_home: "/home/user/.cadis".to_owned(),
                socket_path: Some("/run/user/1000/cadis/cadisd.sock".to_owned()),
                sessions: 0,
                model_provider: "echo".to_owned(),
                uptime_seconds: 3,
            }),
        ));

        let value = serde_json::to_value(&frame).expect("frame should serialize");

        assert_eq!(
            value,
            json!({
                "frame": "response",
                "payload": {
                    "protocol_version": CURRENT_PROTOCOL_VERSION,
                    "request_id": "req_1",
                    "type": "daemon.status.response",
                    "payload": {
                        "status": "ok",
                        "version": "0.1.0",
                        "protocol_version": CURRENT_PROTOCOL_VERSION,
                        "cadis_home": "/home/user/.cadis",
                        "socket_path": "/run/user/1000/cadis/cadisd.sock",
                        "sessions": 0,
                        "model_provider": "echo",
                        "uptime_seconds": 3
                    }
                }
            })
        );
    }

    #[test]
    fn unknown_request_type_is_rejected() {
        let value = json!({
            "protocol_version": CURRENT_PROTOCOL_VERSION,
            "request_id": "req_1",
            "client_id": "cli_1",
            "type": "unknown.request",
            "payload": {}
        });

        let error = serde_json::from_value::<RequestEnvelope>(value)
            .expect_err("unknown requests must fail");

        assert!(
            error.to_string().contains("unknown variant"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unsupported_protocol_version_is_reported() {
        let version = ProtocolVersion::from("9.9");

        assert_eq!(
            version.ensure_supported(),
            Err(ProtocolError::UnsupportedVersion {
                expected: CURRENT_PROTOCOL_VERSION,
                actual: "9.9".to_owned()
            })
        );
    }

    #[test]
    fn invalid_timestamp_is_rejected() {
        let value = json!({
            "protocol_version": CURRENT_PROTOCOL_VERSION,
            "event_id": "evt_1",
            "timestamp": "2026-04-26T00:00:00+07:00",
            "source": "cadisd",
            "type": "daemon.started",
            "payload": {}
        });

        let error = serde_json::from_value::<EventEnvelope>(value)
            .expect_err("non-UTC timestamps must fail");

        assert!(
            error.to_string().contains("invalid UTC timestamp"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn model_descriptor_serializes_readiness_metadata() {
        let descriptor = ModelDescriptor {
            provider: "echo".to_owned(),
            model: "cadis-local-fallback".to_owned(),
            display_name: "CADIS local fallback".to_owned(),
            capabilities: vec!["offline".to_owned()],
            readiness: Some(ModelReadiness::Fallback),
            effective_provider: Some("echo".to_owned()),
            effective_model: Some("cadis-local-fallback".to_owned()),
            fallback: true,
        };

        let value = serde_json::to_value(&descriptor).expect("descriptor should serialize");

        assert_eq!(
            value,
            json!({
                "provider": "echo",
                "model": "cadis-local-fallback",
                "display_name": "CADIS local fallback",
                "capabilities": ["offline"],
                "readiness": "fallback",
                "effective_provider": "echo",
                "effective_model": "cadis-local-fallback",
                "fallback": true
            })
        );
    }

    #[test]
    fn model_descriptor_deserializes_legacy_shape() {
        let value = json!({
            "provider": "ollama",
            "model": "configured",
            "display_name": "Ollama local model",
            "capabilities": ["streaming", "local_model"]
        });

        let descriptor =
            serde_json::from_value::<ModelDescriptor>(value).expect("legacy shape should parse");

        assert_eq!(descriptor.readiness, None);
        assert_eq!(descriptor.effective_provider, None);
        assert_eq!(descriptor.effective_model, None);
        assert!(!descriptor.fallback);
    }

    #[test]
    fn message_completed_serializes_model_invocation_metadata() {
        let payload = MessageCompletedPayload {
            content_kind: ContentKind::Chat,
            content: Some("done".to_owned()),
            agent_id: Some(AgentId::from("codex")),
            agent_name: Some("Codex".to_owned()),
            model: Some(ModelInvocationPayload {
                requested_model: Some("echo/cadis-local-fallback".to_owned()),
                effective_provider: "echo".to_owned(),
                effective_model: "cadis-local-fallback".to_owned(),
                fallback: false,
                fallback_reason: None,
            }),
        };

        let value = serde_json::to_value(&payload).expect("payload should serialize");

        assert_eq!(
            value,
            json!({
                "content_kind": "chat",
                "content": "done",
                "agent_id": "codex",
                "agent_name": "Codex",
                "model": {
                    "requested_model": "echo/cadis-local-fallback",
                    "effective_provider": "echo",
                    "effective_model": "cadis-local-fallback",
                    "fallback": false
                }
            })
        );
    }
}
