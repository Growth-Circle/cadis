use std::env;
use std::error::Error;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{self, Command as ProcessCommand};
use std::time::{SystemTime, UNIX_EPOCH};

use cadis_protocol::{
    AgentEventPayload, AgentId, AgentSpawnRequest, ApprovalDecision, ApprovalId,
    ApprovalResponseRequest, ClientId, ClientRequest, ContentKind, DaemonResponse, EmptyPayload,
    ErrorPayload, EventId, EventSubscriptionRequest, EventsSnapshotRequest, MessageSendRequest,
    ModelsListPayload, RequestEnvelope, RequestId, ServerFrame, SessionId,
    SessionSubscriptionRequest, ToolCallRequest, VoiceDoctorPayload, VoiceDoctorRequest,
    VoiceRuntimeState, VoiceStatusPayload, WorkerTailRequest, WorkspaceAccess,
    WorkspaceDoctorPayload, WorkspaceDoctorRequest, WorkspaceGrantPayload, WorkspaceGrantRequest,
    WorkspaceId, WorkspaceKind, WorkspaceListPayload, WorkspaceListRequest, WorkspaceRecordPayload,
    WorkspaceRegisterRequest, WorkspaceRevokeRequest,
};
use cadis_store::load_config;

fn main() {
    if let Err(error) = run() {
        eprintln!("cadis: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse(env::args().skip(1))?;

    if cli.version {
        println!("cadis {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    match &cli.command {
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::Daemon(args) => launch_daemon(args.clone()),
        Command::Status => {
            let frames = send_request(&cli, ClientRequest::DaemonStatus(EmptyPayload::default()))?;
            render_status(&frames, cli.json)
        }
        Command::Doctor => run_doctor(&cli),
        Command::Models => {
            let frames = send_request(&cli, ClientRequest::ModelsList(EmptyPayload::default()))?;
            render_models(&frames, cli.json)
        }
        Command::Agents => {
            let frames = send_request(&cli, ClientRequest::AgentList(EmptyPayload::default()))?;
            render_agents(&frames, cli.json)
        }
        Command::Worker(command) => run_worker(&cli, command),
        Command::Workspace(command) => run_workspace(&cli, command),
        Command::Session(command) => run_session(&cli, command),
        Command::Voice(command) => run_voice(&cli, command),
        Command::Events {
            replay_limit,
            since_event_id,
            include_snapshot,
            snapshot_only,
        } => {
            if *snapshot_only {
                let frames = send_request(
                    &cli,
                    ClientRequest::EventsSnapshot(EventsSnapshotRequest::default()),
                )?;
                render_events(&frames, cli.json)
            } else {
                stream_events(
                    &cli,
                    EventSubscriptionRequest {
                        since_event_id: since_event_id
                            .as_ref()
                            .map(|value| EventId::from(value.clone())),
                        replay_limit: *replay_limit,
                        include_snapshot: *include_snapshot,
                    },
                )
            }
        }
        Command::Spawn {
            role,
            name,
            parent,
            model,
        } => {
            let frames = send_request(
                &cli,
                ClientRequest::AgentSpawn(AgentSpawnRequest {
                    role: role.clone(),
                    parent_agent_id: parent.as_ref().map(|value| AgentId::from(value.clone())),
                    display_name: name.clone(),
                    model: model.clone(),
                }),
            )?;
            render_agents(&frames, cli.json)
        }
        Command::Chat(message) => {
            let frames = send_request(
                &cli,
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content: message.clone(),
                    content_kind: ContentKind::Chat,
                }),
            )?;
            render_chat(&frames, cli.json)
        }
        Command::Run { cwd, task } => {
            let content = match cwd {
                Some(cwd) => format!("Run in {}: {task}", cwd.display()),
                None => task.clone(),
            };
            let frames = send_request(
                &cli,
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content,
                    content_kind: ContentKind::Chat,
                }),
            )?;
            render_chat(&frames, cli.json)
        }
        Command::Tool {
            session_id,
            agent_id,
            cwd,
            workspace_id,
            tool_name,
            input,
        } => {
            let mut input = input.clone();
            if let Some(workspace_id) = workspace_id {
                input["workspace_id"] = serde_json::json!(workspace_id);
            }
            if let Some(cwd) = cwd {
                input["workspace"] = serde_json::json!(cwd);
            }
            let frames = send_request(
                &cli,
                ClientRequest::ToolCall(ToolCallRequest {
                    session_id: session_id
                        .as_ref()
                        .map(|value| SessionId::from(value.clone())),
                    agent_id: agent_id.as_ref().map(|value| AgentId::from(value.clone())),
                    tool_name: tool_name.clone(),
                    input,
                }),
            )?;
            render_tool(&frames, cli.json)
        }
        Command::Approve(approval_id) => {
            send_approval(&cli, approval_id.clone(), ApprovalDecision::Approved)
        }
        Command::Deny(approval_id) => {
            send_approval(&cli, approval_id.clone(), ApprovalDecision::Denied)
        }
    }
}

fn send_approval(
    cli: &Cli,
    approval_id: String,
    decision: ApprovalDecision,
) -> Result<(), Box<dyn Error>> {
    let frames = send_request(
        cli,
        ClientRequest::ApprovalRespond(ApprovalResponseRequest {
            approval_id: ApprovalId::from(approval_id),
            decision,
            reason: Some("resolved from CLI".to_owned()),
        }),
    )?;
    render_rejections_or_json(&frames, cli.json)
}

fn run_doctor(cli: &Cli) -> Result<(), Box<dyn Error>> {
    let config = load_config()?;
    let status_frames = send_request(cli, ClientRequest::DaemonStatus(EmptyPayload::default()))?;
    let voice_frames = send_request(
        cli,
        ClientRequest::VoiceDoctor(VoiceDoctorRequest::default()),
    )?;

    if cli.json {
        print_json_frames(&status_frames)?;
        return print_json_frames(&voice_frames);
    }

    println!("cadis doctor");
    println!("cadis_home: {}", config.cadis_home.display());
    println!("config: {}", config.config_path().display());
    println!("socket: {}", cli.socket_path()?.display());

    print!("daemon: ");
    match daemon_status(&status_frames) {
        Some(status) => {
            println!("{}", status.status);
            println!("model_provider: {}", status.model_provider);
            println!("sessions: {}", status.sessions);
            render_voice(&voice_frames, false)?;
            Ok(())
        }
        None => {
            println!("unexpected response");
            render_rejections_or_json(&status_frames, cli.json)
        }
    }
}

fn run_voice(cli: &Cli, command: &VoiceCommand) -> Result<(), Box<dyn Error>> {
    let frames = match command {
        VoiceCommand::Status => {
            send_request(cli, ClientRequest::VoiceStatus(EmptyPayload::default()))?
        }
        VoiceCommand::Doctor => send_request(
            cli,
            ClientRequest::VoiceDoctor(VoiceDoctorRequest::default()),
        )?,
    };

    render_voice(&frames, cli.json)
}

fn run_workspace(cli: &Cli, command: &WorkspaceCommand) -> Result<(), Box<dyn Error>> {
    let frames = match command {
        WorkspaceCommand::List { include_grants } => send_request(
            cli,
            ClientRequest::WorkspaceList(WorkspaceListRequest {
                include_grants: *include_grants,
            }),
        )?,
        WorkspaceCommand::Register {
            workspace_id,
            root,
            kind,
            aliases,
            vcs,
            trusted,
            worktree_root,
            artifact_root,
        } => send_request(
            cli,
            ClientRequest::WorkspaceRegister(WorkspaceRegisterRequest {
                workspace_id: WorkspaceId::from(workspace_id.clone()),
                kind: *kind,
                root: root.display().to_string(),
                aliases: aliases.clone(),
                vcs: vcs.clone(),
                trusted: *trusted,
                worktree_root: worktree_root.clone(),
                artifact_root: artifact_root.clone(),
            }),
        )?,
        WorkspaceCommand::Grant {
            workspace_id,
            agent_id,
            access,
            source,
        } => send_request(
            cli,
            ClientRequest::WorkspaceGrant(WorkspaceGrantRequest {
                agent_id: agent_id.as_ref().map(|value| AgentId::from(value.clone())),
                workspace_id: WorkspaceId::from(workspace_id.clone()),
                access: access.clone(),
                expires_at: None,
                source: source.clone(),
            }),
        )?,
        WorkspaceCommand::Revoke {
            grant_id,
            workspace_id,
            agent_id,
        } => send_request(
            cli,
            ClientRequest::WorkspaceRevoke(WorkspaceRevokeRequest {
                grant_id: grant_id
                    .as_ref()
                    .map(|value| cadis_protocol::WorkspaceGrantId::from(value.clone())),
                workspace_id: workspace_id
                    .as_ref()
                    .map(|value| WorkspaceId::from(value.clone())),
                agent_id: agent_id.as_ref().map(|value| AgentId::from(value.clone())),
            }),
        )?,
        WorkspaceCommand::Doctor { workspace_id, root } => send_request(
            cli,
            ClientRequest::WorkspaceDoctor(WorkspaceDoctorRequest {
                workspace_id: workspace_id
                    .as_ref()
                    .map(|value| WorkspaceId::from(value.clone())),
                root: root.as_ref().map(|path| path.display().to_string()),
            }),
        )?,
    };

    render_workspace(&frames, cli.json)
}

fn run_session(cli: &Cli, command: &SessionCommand) -> Result<(), Box<dyn Error>> {
    match command {
        SessionCommand::Subscribe {
            session_id,
            replay_limit,
            since_event_id,
            include_snapshot,
        } => stream_subscription(
            cli,
            ClientRequest::SessionSubscribe(SessionSubscriptionRequest {
                session_id: SessionId::from(session_id.clone()),
                since_event_id: since_event_id
                    .as_ref()
                    .map(|value| EventId::from(value.clone())),
                replay_limit: *replay_limit,
                include_snapshot: *include_snapshot,
            }),
        ),
    }
}

fn run_worker(cli: &Cli, command: &WorkerCommand) -> Result<(), Box<dyn Error>> {
    let frames = match command {
        WorkerCommand::Tail { worker_id, lines } => send_request(
            cli,
            ClientRequest::WorkerTail(WorkerTailRequest {
                worker_id: worker_id.clone(),
                lines: *lines,
            }),
        )?,
    };

    render_worker_tail(&frames, cli.json)
}

fn send_request(cli: &Cli, request: ClientRequest) -> Result<Vec<ServerFrame>, Box<dyn Error>> {
    let socket_path = cli.socket_path()?;
    let mut stream = UnixStream::connect(&socket_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not connect to cadisd at {}: {error}. Start it with `cadisd`.",
                socket_path.display()
            ),
        )
    })?;

    let envelope = RequestEnvelope::new(next_request_id(), client_id(), request);
    serde_json::to_writer(&mut stream, &envelope)?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let reader = BufReader::new(stream);
    let mut frames = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        frames.push(serde_json::from_str::<ServerFrame>(&line)?);
    }

    Ok(frames)
}

fn stream_events(cli: &Cli, request: EventSubscriptionRequest) -> Result<(), Box<dyn Error>> {
    stream_subscription(cli, ClientRequest::EventsSubscribe(request))
}

fn stream_subscription(cli: &Cli, request: ClientRequest) -> Result<(), Box<dyn Error>> {
    let socket_path = cli.socket_path()?;
    let mut stream = UnixStream::connect(&socket_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not connect to cadisd at {}: {error}. Start it with `cadisd`.",
                socket_path.display()
            ),
        )
    })?;

    let envelope = RequestEnvelope::new(next_request_id(), client_id(), request);
    serde_json::to_writer(&mut stream, &envelope)?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let frame = serde_json::from_str::<ServerFrame>(&line)?;
        if cli.json {
            println!("{}", serde_json::to_string(&frame)?);
        } else {
            render_event_frame(&frame)?;
        }
    }

    Ok(())
}

fn render_status(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    let status =
        daemon_status(frames).ok_or_else(|| invalid_data("daemon did not return status"))?;
    println!("status: {}", status.status);
    println!("version: {}", status.version);
    println!("protocol: {}", status.protocol_version);
    println!("cadis_home: {}", status.cadis_home);
    if let Some(socket_path) = &status.socket_path {
        println!("socket: {socket_path}");
    }
    println!("sessions: {}", status.sessions);
    println!("model_provider: {}", status.model_provider);
    println!("uptime_seconds: {}", status.uptime_seconds);
    print_voice_status(&status.voice);
    Ok(())
}

fn render_models(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    for frame in frames {
        if let ServerFrame::Event(event) = frame {
            if let cadis_protocol::CadisEvent::ModelsListResponse(ModelsListPayload { models }) =
                &event.event
            {
                for model in models {
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{}",
                        model.provider,
                        model.model,
                        model_readiness_label(model.readiness),
                        effective_model_label(
                            model.effective_provider.as_deref(),
                            model.effective_model.as_deref()
                        ),
                        if model.fallback { "fallback" } else { "real" },
                        model.display_name
                    );
                }
            }
        }
    }
    Ok(())
}

fn model_readiness_label(readiness: Option<cadis_protocol::ModelReadiness>) -> &'static str {
    match readiness {
        Some(cadis_protocol::ModelReadiness::Ready) => "ready",
        Some(cadis_protocol::ModelReadiness::Fallback) => "fallback",
        Some(cadis_protocol::ModelReadiness::RequiresConfiguration) => "requires_configuration",
        Some(cadis_protocol::ModelReadiness::Unavailable) => "unavailable",
        None => "unknown",
    }
}

fn effective_model_label(provider: Option<&str>, model: Option<&str>) -> String {
    match (provider, model) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        (Some(provider), None) => provider.to_owned(),
        (None, Some(model)) => model.to_owned(),
        (None, None) => "-".to_owned(),
    }
}

fn render_agents(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    let mut rendered = false;
    for frame in frames {
        let ServerFrame::Event(event) = frame else {
            continue;
        };
        match &event.event {
            cadis_protocol::CadisEvent::AgentListResponse(payload) => {
                for agent in &payload.agents {
                    print_agent(agent);
                    rendered = true;
                }
            }
            cadis_protocol::CadisEvent::AgentSpawned(agent)
            | cadis_protocol::CadisEvent::AgentCompleted(agent) => {
                print_agent(agent);
                rendered = true;
            }
            _ => {}
        }
    }

    if !rendered {
        println!("no agents returned");
    }
    Ok(())
}

fn render_workspace(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    let mut rendered = false;
    for frame in frames {
        let ServerFrame::Event(event) = frame else {
            continue;
        };
        match &event.event {
            cadis_protocol::CadisEvent::WorkspaceListResponse(WorkspaceListPayload {
                workspaces,
                grants,
            }) => {
                for workspace in workspaces {
                    print_workspace(workspace);
                    rendered = true;
                }
                for grant in grants {
                    print_workspace_grant(grant);
                    rendered = true;
                }
            }
            cadis_protocol::CadisEvent::WorkspaceRegistered(workspace) => {
                print_workspace(workspace);
                rendered = true;
            }
            cadis_protocol::CadisEvent::WorkspaceGrantCreated(grant)
            | cadis_protocol::CadisEvent::WorkspaceGrantRevoked(grant) => {
                print_workspace_grant(grant);
                rendered = true;
            }
            cadis_protocol::CadisEvent::WorkspaceDoctorResponse(WorkspaceDoctorPayload {
                checks,
            }) => {
                for check in checks {
                    println!("{}\t{}\t{}", check.status, check.name, check.message);
                    rendered = true;
                }
            }
            _ => {}
        }
    }

    if !rendered {
        println!("no workspace data returned");
    }
    Ok(())
}

fn render_voice(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    let mut rendered = false;
    for frame in frames {
        let ServerFrame::Event(event) = frame else {
            continue;
        };
        match &event.event {
            cadis_protocol::CadisEvent::VoiceStatusUpdated(status) => {
                print_voice_status(status);
                rendered = true;
            }
            cadis_protocol::CadisEvent::VoiceDoctorResponse(payload)
            | cadis_protocol::CadisEvent::VoicePreflightResponse(payload) => {
                print_voice_doctor(payload);
                rendered = true;
            }
            _ => {}
        }
    }

    if !rendered {
        println!("no voice data returned");
    }
    Ok(())
}

fn print_voice_doctor(payload: &VoiceDoctorPayload) {
    print_voice_status(&payload.status);
    for check in &payload.checks {
        println!("{}\t{}\t{}", check.status, check.name, check.message);
    }
}

fn print_voice_status(status: &VoiceStatusPayload) {
    println!(
        "voice: {}\tenabled={}\tprovider={}\tvoice={}\tstt={}\tbridge={}\tmax_spoken_chars={}",
        voice_state_name(status.state),
        status.enabled,
        status.provider,
        status.voice_id,
        status.stt_language,
        status.bridge,
        status.max_spoken_chars
    );
    if let Some(preflight) = &status.last_preflight {
        println!(
            "voice_preflight: {}\t{}\tsurface={}\tchecked_at={}",
            preflight.status, preflight.summary, preflight.surface, preflight.checked_at
        );
    }
}

fn voice_state_name(state: VoiceRuntimeState) -> &'static str {
    match state {
        VoiceRuntimeState::Disabled => "disabled",
        VoiceRuntimeState::Ready => "ready",
        VoiceRuntimeState::Degraded => "degraded",
        VoiceRuntimeState::Blocked => "blocked",
        VoiceRuntimeState::Unknown => "unknown",
    }
}

fn print_workspace(workspace: &WorkspaceRecordPayload) {
    let aliases = if workspace.aliases.is_empty() {
        "-".to_owned()
    } else {
        workspace.aliases.join(",")
    };
    let vcs = workspace.vcs.as_deref().unwrap_or("-");
    println!(
        "{}\t{:?}\t{}\tvcs={}\ttrusted={}\taliases={}",
        workspace.workspace_id, workspace.kind, workspace.root, vcs, workspace.trusted, aliases
    );
}

fn print_workspace_grant(grant: &WorkspaceGrantPayload) {
    let agent = grant.agent_id.as_ref().map(|id| id.as_str()).unwrap_or("-");
    let access = grant
        .access
        .iter()
        .map(|access| format!("{access:?}").to_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{}\tworkspace={}\tagent={}\taccess={}\troot={}\tsource={}",
        grant.grant_id, grant.workspace_id, agent, access, grant.root, grant.source
    );
}

fn render_events(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    for frame in frames {
        render_event_frame(frame)?;
    }
    Ok(())
}

fn render_event_frame(frame: &ServerFrame) -> Result<(), Box<dyn Error>> {
    match frame {
        ServerFrame::Response(response) => {
            if let DaemonResponse::RequestRejected(error) = &response.response {
                return Err(invalid_data(format_error(error)).into());
            }
        }
        ServerFrame::Event(event) => {
            let session_id = event
                .session_id
                .as_ref()
                .map(|session_id| session_id.as_str())
                .unwrap_or("-");
            println!(
                "{}\t{}\t{}",
                event.event_id,
                event_type_name(frame),
                session_id
            );
        }
    }
    Ok(())
}

fn event_type_name(frame: &ServerFrame) -> String {
    serde_json::to_value(frame)
        .ok()
        .and_then(|value| {
            value
                .get("payload")
                .and_then(|payload| payload.get("type"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "event.unknown".to_owned())
}

fn print_agent(agent: &AgentEventPayload) {
    let name = agent
        .display_name
        .as_deref()
        .unwrap_or_else(|| agent.agent_id.as_str());
    let role = agent.role.as_deref().unwrap_or("agent");
    let parent = agent
        .parent_agent_id
        .as_ref()
        .map(|id| id.as_str())
        .unwrap_or("-");
    let model = agent.model.as_deref().unwrap_or("-");
    let status = agent
        .status
        .map(|status| format!("{status:?}").to_lowercase())
        .unwrap_or_else(|| "-".to_owned());
    println!(
        "{}\t{}\t{}\tparent={}\tmodel={}\tstatus={}",
        agent.agent_id, name, role, parent, model, status
    );
}

fn render_chat(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    let mut wrote_delta = false;
    for frame in frames {
        match frame {
            ServerFrame::Event(event) => match &event.event {
                cadis_protocol::CadisEvent::MessageDelta(payload) => {
                    print!("{}", payload.delta);
                    io::stdout().flush()?;
                    wrote_delta = true;
                }
                cadis_protocol::CadisEvent::MessageCompleted(_) if wrote_delta => {
                    println!();
                }
                cadis_protocol::CadisEvent::SessionFailed(error)
                | cadis_protocol::CadisEvent::DaemonError(error) => {
                    return Err(invalid_data(format_error(error)).into());
                }
                _ => {}
            },
            ServerFrame::Response(_) => {}
        }
    }
    Ok(())
}

fn render_tool(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    for frame in frames {
        let ServerFrame::Event(event) = frame else {
            continue;
        };
        match &event.event {
            cadis_protocol::CadisEvent::ToolCompleted(payload) => {
                if let Some(summary) = &payload.summary {
                    println!("{summary}");
                } else if let Some(output) = &payload.output {
                    println!("{}", serde_json::to_string_pretty(output)?);
                }
            }
            cadis_protocol::CadisEvent::ToolFailed(payload) => {
                return Err(invalid_data(format_error(&payload.error)).into());
            }
            cadis_protocol::CadisEvent::ApprovalRequested(payload) => {
                println!(
                    "approval required: {} ({:?})",
                    payload.approval_id, payload.risk_class
                );
                if let Some(command) = &payload.command {
                    println!("command: {command}");
                }
            }
            cadis_protocol::CadisEvent::ApprovalResolved(payload) => {
                println!(
                    "approval resolved: {} {:?}",
                    payload.approval_id, payload.decision
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn render_worker_tail(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }

    render_rejections(frames)?;
    for frame in frames {
        let ServerFrame::Event(event) = frame else {
            continue;
        };
        if let cadis_protocol::CadisEvent::WorkerLogDelta(payload) = &event.event {
            print!("{}", payload.delta);
            if !payload.delta.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}

fn render_rejections_or_json(frames: &[ServerFrame], json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        return print_json_frames(frames);
    }
    render_rejections(frames)
}

fn render_rejections(frames: &[ServerFrame]) -> Result<(), Box<dyn Error>> {
    for frame in frames {
        if let ServerFrame::Response(response) = frame {
            if let DaemonResponse::RequestRejected(error) = &response.response {
                return Err(invalid_data(format_error(error)).into());
            }
        }
    }
    Ok(())
}

fn print_json_frames(frames: &[ServerFrame]) -> Result<(), Box<dyn Error>> {
    for frame in frames {
        println!("{}", serde_json::to_string(frame)?);
    }
    Ok(())
}

fn daemon_status(frames: &[ServerFrame]) -> Option<&cadis_protocol::DaemonStatusPayload> {
    frames.iter().find_map(|frame| {
        let ServerFrame::Response(response) = frame else {
            return None;
        };
        match &response.response {
            DaemonResponse::DaemonStatus(status) => Some(status),
            _ => None,
        }
    })
}

fn format_error(error: &ErrorPayload) -> String {
    format!("{}: {}", error.code, error.message)
}

fn launch_daemon(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let exe = env::current_exe()?;
    let sibling = exe.with_file_name("cadisd");
    let program = if sibling.exists() {
        sibling
    } else {
        PathBuf::from("cadisd")
    };

    let status = ProcessCommand::new(&program).args(args).status()?;
    process::exit(status.code().unwrap_or(1));
}

fn next_request_id() -> RequestId {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    RequestId::from(format!("req_{}_{}", process::id(), millis))
}

fn client_id() -> ClientId {
    ClientId::from(format!("cli_{}", process::id()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Cli {
    json: bool,
    version: bool,
    socket_path: Option<PathBuf>,
    command: Command,
}

impl Cli {
    fn parse<I>(args: I) -> Result<Self, Box<dyn Error>>
    where
        I: IntoIterator<Item = String>,
    {
        let mut json = false;
        let mut version = false;
        let mut socket_path = None;
        let mut args = args.into_iter().peekable();

        while let Some(arg) = args.peek() {
            match arg.as_str() {
                "--json" => {
                    json = true;
                    args.next();
                }
                "--version" | "-V" => {
                    version = true;
                    args.next();
                }
                "--socket" => {
                    args.next();
                    socket_path = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| invalid_input("--socket requires a path"))?,
                    ));
                }
                "--help" | "-h" => {
                    args.next();
                    return Ok(Self {
                        json,
                        version,
                        socket_path,
                        command: Command::Help,
                    });
                }
                _ => break,
            }
        }

        let command = match args.next().as_deref() {
            None if version => Command::Help,
            None => Command::Help,
            Some("daemon") => Command::Daemon(args.collect()),
            Some("status") => Command::Status,
            Some("doctor") => Command::Doctor,
            Some("models") => Command::Models,
            Some("agents") => Command::Agents,
            Some("worker") => Command::Worker(parse_worker(args.collect())?),
            Some("workspace") => Command::Workspace(parse_workspace(args.collect())?),
            Some("session") => Command::Session(parse_session(args.collect())?),
            Some("voice") => Command::Voice(parse_voice(args.collect())?),
            Some("events") => parse_events(args.collect())?,
            Some("spawn") => parse_spawn(args.collect())?,
            Some("chat") => {
                Command::Chat(required_text(args.collect(), "chat requires a message")?)
            }
            Some("run") => parse_run(args.collect())?,
            Some("tool") => parse_tool(args.collect())?,
            Some("approve") => Command::Approve(
                args.next()
                    .ok_or_else(|| invalid_input("approve requires an ID"))?,
            ),
            Some("deny") => Command::Deny(
                args.next()
                    .ok_or_else(|| invalid_input("deny requires an ID"))?,
            ),
            Some(other) => return Err(invalid_input(format!("unknown command: {other}")).into()),
        };

        Ok(Self {
            json,
            version,
            socket_path,
            command,
        })
    }

    fn socket_path(&self) -> Result<PathBuf, Box<dyn Error>> {
        if let Some(path) = &self.socket_path {
            return Ok(path.clone());
        }

        Ok(load_config()?.effective_socket_path())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Help,
    Daemon(Vec<String>),
    Status,
    Doctor,
    Models,
    Agents,
    Worker(WorkerCommand),
    Workspace(WorkspaceCommand),
    Session(SessionCommand),
    Voice(VoiceCommand),
    Events {
        replay_limit: Option<u32>,
        since_event_id: Option<String>,
        include_snapshot: bool,
        snapshot_only: bool,
    },
    Spawn {
        role: String,
        name: Option<String>,
        parent: Option<String>,
        model: Option<String>,
    },
    Chat(String),
    Run {
        cwd: Option<PathBuf>,
        task: String,
    },
    Tool {
        session_id: Option<String>,
        agent_id: Option<String>,
        cwd: Option<PathBuf>,
        workspace_id: Option<String>,
        tool_name: String,
        input: serde_json::Value,
    },
    Approve(String),
    Deny(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SessionCommand {
    Subscribe {
        session_id: String,
        replay_limit: Option<u32>,
        since_event_id: Option<String>,
        include_snapshot: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WorkerCommand {
    Tail {
        worker_id: String,
        lines: Option<u32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WorkspaceCommand {
    List {
        include_grants: bool,
    },
    Register {
        workspace_id: String,
        root: PathBuf,
        kind: WorkspaceKind,
        aliases: Vec<String>,
        vcs: Option<String>,
        trusted: bool,
        worktree_root: Option<String>,
        artifact_root: Option<String>,
    },
    Grant {
        workspace_id: String,
        agent_id: Option<String>,
        access: Vec<WorkspaceAccess>,
        source: Option<String>,
    },
    Revoke {
        grant_id: Option<String>,
        workspace_id: Option<String>,
        agent_id: Option<String>,
    },
    Doctor {
        workspace_id: Option<String>,
        root: Option<PathBuf>,
    },
}

fn parse_worker(args: Vec<String>) -> Result<WorkerCommand, Box<dyn Error>> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("tail") => parse_worker_tail(args.collect()),
        Some(other) => Err(invalid_input(format!("unknown worker command: {other}")).into()),
        None => Err(invalid_input("worker requires a subcommand").into()),
    }
}

fn parse_worker_tail(args: Vec<String>) -> Result<WorkerCommand, Box<dyn Error>> {
    let mut lines = None;
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lines" => {
                let value = args
                    .next()
                    .ok_or_else(|| invalid_input("--lines requires a count"))?;
                lines = Some(value.parse::<u32>().map_err(|error| {
                    invalid_input(format!("--lines requires a non-negative integer: {error}"))
                })?);
            }
            value if value.starts_with("--") => {
                return Err(invalid_input(format!("unknown worker tail option: {value}")).into());
            }
            value => positionals.push(value.to_owned()),
        }
    }

    Ok(WorkerCommand::Tail {
        worker_id: positionals
            .first()
            .cloned()
            .ok_or_else(|| invalid_input("worker tail requires a worker ID"))?,
        lines,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum VoiceCommand {
    Status,
    Doctor,
}

fn parse_voice(args: Vec<String>) -> Result<VoiceCommand, Box<dyn Error>> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("status") | None => Ok(VoiceCommand::Status),
        Some("doctor") => Ok(VoiceCommand::Doctor),
        Some(other) => Err(invalid_input(format!("unknown voice command: {other}")).into()),
    }
}

fn parse_workspace(args: Vec<String>) -> Result<WorkspaceCommand, Box<dyn Error>> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("list") => parse_workspace_list(args.collect()),
        Some("register") => parse_workspace_register(args.collect()),
        Some("grant") => parse_workspace_grant(args.collect()),
        Some("revoke") => parse_workspace_revoke(args.collect()),
        Some("doctor") => parse_workspace_doctor(args.collect()),
        Some(other) => Err(invalid_input(format!("unknown workspace command: {other}")).into()),
        None => Err(invalid_input("workspace requires a subcommand").into()),
    }
}

fn parse_workspace_list(args: Vec<String>) -> Result<WorkspaceCommand, Box<dyn Error>> {
    let mut include_grants = false;
    for arg in args {
        match arg.as_str() {
            "--grants" => include_grants = true,
            value => {
                return Err(invalid_input(format!("unknown workspace list option: {value}")).into())
            }
        }
    }
    Ok(WorkspaceCommand::List { include_grants })
}

fn parse_workspace_register(args: Vec<String>) -> Result<WorkspaceCommand, Box<dyn Error>> {
    let mut kind = WorkspaceKind::Project;
    let mut aliases = Vec::new();
    let mut vcs = Some("git".to_owned());
    let mut trusted = true;
    let mut worktree_root = Some(".cadis/worktrees".to_owned());
    let mut artifact_root = Some(".cadis/artifacts".to_owned());
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--kind" => {
                let value = args
                    .next()
                    .ok_or_else(|| invalid_input("--kind requires a value"))?;
                kind = parse_workspace_kind(&value)?;
            }
            "--alias" => {
                aliases.push(
                    args.next()
                        .ok_or_else(|| invalid_input("--alias requires a value"))?,
                );
            }
            "--vcs" => {
                vcs = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--vcs requires a value"))?,
                );
            }
            "--no-vcs" => vcs = Some("none".to_owned()),
            "--trusted" => trusted = true,
            "--untrusted" => trusted = false,
            "--worktree-root" => {
                worktree_root = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--worktree-root requires a path"))?,
                );
            }
            "--artifact-root" => {
                artifact_root = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--artifact-root requires a path"))?,
                );
            }
            value if value.starts_with("--") => {
                return Err(
                    invalid_input(format!("unknown workspace register option: {value}")).into(),
                );
            }
            value => positionals.push(value.to_owned()),
        }
    }

    let workspace_id = positionals
        .first()
        .ok_or_else(|| invalid_input("workspace register requires an ID"))?
        .clone();
    let root = positionals
        .get(1)
        .ok_or_else(|| invalid_input("workspace register requires a root path"))?;

    Ok(WorkspaceCommand::Register {
        workspace_id,
        root: PathBuf::from(root),
        kind,
        aliases,
        vcs,
        trusted,
        worktree_root,
        artifact_root,
    })
}

fn parse_workspace_grant(args: Vec<String>) -> Result<WorkspaceCommand, Box<dyn Error>> {
    let mut agent_id = None;
    let mut access = Vec::new();
    let mut source = Some("user".to_owned());
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => {
                agent_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--agent requires an agent ID"))?,
                );
            }
            "--access" => {
                let value = args
                    .next()
                    .ok_or_else(|| invalid_input("--access requires a comma-separated value"))?;
                access = parse_workspace_access_list(&value)?;
            }
            "--source" => {
                source = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--source requires a value"))?,
                );
            }
            value if value.starts_with("--") => {
                return Err(
                    invalid_input(format!("unknown workspace grant option: {value}")).into(),
                );
            }
            value => positionals.push(value.to_owned()),
        }
    }

    Ok(WorkspaceCommand::Grant {
        workspace_id: positionals
            .first()
            .cloned()
            .ok_or_else(|| invalid_input("workspace grant requires a workspace ID"))?,
        agent_id,
        access,
        source,
    })
}

fn parse_workspace_revoke(args: Vec<String>) -> Result<WorkspaceCommand, Box<dyn Error>> {
    let mut grant_id = None;
    let mut workspace_id = None;
    let mut agent_id = None;
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--grant" => {
                grant_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--grant requires a grant ID"))?,
                );
            }
            "--workspace" => {
                workspace_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--workspace requires a workspace ID"))?,
                );
            }
            "--agent" => {
                agent_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--agent requires an agent ID"))?,
                );
            }
            value if value.starts_with("--") => {
                return Err(
                    invalid_input(format!("unknown workspace revoke option: {value}")).into(),
                );
            }
            value => positionals.push(value.to_owned()),
        }
    }
    if workspace_id.is_none() {
        workspace_id = positionals.first().cloned();
    }

    Ok(WorkspaceCommand::Revoke {
        grant_id,
        workspace_id,
        agent_id,
    })
}

fn parse_workspace_doctor(args: Vec<String>) -> Result<WorkspaceCommand, Box<dyn Error>> {
    let mut workspace_id = None;
    let mut root = None;
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                workspace_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--workspace requires a workspace ID"))?,
                );
            }
            "--root" => {
                root = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| invalid_input("--root requires a path"))?,
                ));
            }
            value if value.starts_with("--") => {
                return Err(
                    invalid_input(format!("unknown workspace doctor option: {value}")).into(),
                );
            }
            value => positionals.push(value.to_owned()),
        }
    }
    if workspace_id.is_none() {
        workspace_id = positionals.first().cloned();
    }

    Ok(WorkspaceCommand::Doctor { workspace_id, root })
}

fn parse_workspace_kind(value: &str) -> Result<WorkspaceKind, Box<dyn Error>> {
    match value {
        "project" => Ok(WorkspaceKind::Project),
        "documents" => Ok(WorkspaceKind::Documents),
        "sandbox" => Ok(WorkspaceKind::Sandbox),
        "worktree" => Ok(WorkspaceKind::Worktree),
        other => Err(invalid_input(format!("unknown workspace kind: {other}")).into()),
    }
}

fn parse_workspace_access_list(value: &str) -> Result<Vec<WorkspaceAccess>, Box<dyn Error>> {
    let mut access = Vec::new();
    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        access.push(match item {
            "read" => WorkspaceAccess::Read,
            "write" => WorkspaceAccess::Write,
            "exec" => WorkspaceAccess::Exec,
            "admin" => WorkspaceAccess::Admin,
            other => return Err(invalid_input(format!("unknown workspace access: {other}")).into()),
        });
    }
    Ok(access)
}

fn parse_session(args: Vec<String>) -> Result<SessionCommand, Box<dyn Error>> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("subscribe") => parse_session_subscribe(args.collect()),
        Some(other) => Err(invalid_input(format!("unknown session command: {other}")).into()),
        None => Err(invalid_input("session requires a subcommand").into()),
    }
}

fn parse_session_subscribe(args: Vec<String>) -> Result<SessionCommand, Box<dyn Error>> {
    let mut replay_limit = Some(128);
    let mut since_event_id = None;
    let mut include_snapshot = true;
    let mut positionals = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--replay" => {
                let value = args
                    .next()
                    .ok_or_else(|| invalid_input("--replay requires a count"))?;
                replay_limit = Some(value.parse::<u32>().map_err(|error| {
                    invalid_input(format!("--replay requires a non-negative integer: {error}"))
                })?);
            }
            "--since" => {
                since_event_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--since requires an event ID"))?,
                );
            }
            "--no-snapshot" => include_snapshot = false,
            value if value.starts_with("--") => {
                return Err(
                    invalid_input(format!("unknown session subscribe option: {value}")).into(),
                );
            }
            value => positionals.push(value.to_owned()),
        }
    }

    let session_id = positionals
        .first()
        .ok_or_else(|| invalid_input("session subscribe requires a session ID"))?
        .clone();

    Ok(SessionCommand::Subscribe {
        session_id,
        replay_limit,
        since_event_id,
        include_snapshot,
    })
}

fn parse_events(args: Vec<String>) -> Result<Command, Box<dyn Error>> {
    let mut replay_limit = Some(128);
    let mut since_event_id = None;
    let mut include_snapshot = true;
    let mut snapshot_only = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--replay" => {
                let value = args
                    .next()
                    .ok_or_else(|| invalid_input("--replay requires a count"))?;
                replay_limit = Some(value.parse::<u32>().map_err(|error| {
                    invalid_input(format!("--replay requires a non-negative integer: {error}"))
                })?);
            }
            "--since" => {
                since_event_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--since requires an event ID"))?,
                );
            }
            "--no-snapshot" => include_snapshot = false,
            "--snapshot" => snapshot_only = true,
            value => return Err(invalid_input(format!("unknown events option: {value}")).into()),
        }
    }

    Ok(Command::Events {
        replay_limit,
        since_event_id,
        include_snapshot,
        snapshot_only,
    })
}

fn parse_spawn(args: Vec<String>) -> Result<Command, Box<dyn Error>> {
    let mut role = Vec::new();
    let mut name = None;
    let mut parent = None;
    let mut model = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => {
                name = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--name requires a value"))?,
                );
            }
            "--parent" => {
                parent = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--parent requires an agent ID"))?,
                );
            }
            "--model" => {
                model = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--model requires a model"))?,
                );
            }
            value if value.starts_with("--") => {
                return Err(invalid_input(format!("unknown spawn option: {value}")).into());
            }
            value => role.push(value.to_owned()),
        }
    }

    Ok(Command::Spawn {
        role: required_text(role, "spawn requires a role")?,
        name,
        parent,
        model,
    })
}

fn parse_run(args: Vec<String>) -> Result<Command, Box<dyn Error>> {
    let mut cwd = None;
    let mut task = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--cwd" {
            cwd = Some(PathBuf::from(
                args.next()
                    .ok_or_else(|| invalid_input("--cwd requires a path"))?,
            ));
        } else {
            task.push(arg);
            task.extend(args);
            break;
        }
    }

    Ok(Command::Run {
        cwd,
        task: required_text(task, "run requires a task")?,
    })
}

fn parse_tool(args: Vec<String>) -> Result<Command, Box<dyn Error>> {
    let mut session_id = None;
    let mut agent_id = None;
    let mut cwd = None;
    let mut workspace_id = None;
    let mut explicit_input = None;
    let mut args = args.into_iter();
    let mut rest = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session" => {
                session_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--session requires an ID"))?,
                );
            }
            "--agent" => {
                agent_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--agent requires an agent ID"))?,
                );
            }
            "--cwd" => {
                cwd = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| invalid_input("--cwd requires a path"))?,
                ));
            }
            "--workspace" => {
                workspace_id = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--workspace requires an ID"))?,
                );
            }
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| invalid_input("--input requires JSON"))?;
                explicit_input = Some(serde_json::from_str::<serde_json::Value>(&value)?);
            }
            value => {
                rest.push(value.to_owned());
                rest.extend(args);
                break;
            }
        }
    }

    let tool_name = rest
        .first()
        .ok_or_else(|| invalid_input("tool requires a tool name"))?
        .clone();
    let input = explicit_input.unwrap_or_else(|| tool_input_from_args(&tool_name, &rest[1..]));

    Ok(Command::Tool {
        session_id,
        agent_id,
        cwd,
        workspace_id,
        tool_name,
        input,
    })
}

fn tool_input_from_args(tool_name: &str, args: &[String]) -> serde_json::Value {
    match tool_name {
        "file.read" => serde_json::json!({
            "path": args.first().cloned().unwrap_or_default()
        }),
        "file.search" => serde_json::json!({
            "query": args.first().cloned().unwrap_or_default(),
            "path": args.get(1).cloned().unwrap_or_else(|| ".".to_owned())
        }),
        "git.status" => serde_json::json!({
            "path": args.first().cloned().unwrap_or_else(|| ".".to_owned())
        }),
        "git.diff" => serde_json::json!({
            "path": args.first().cloned().unwrap_or_else(|| ".".to_owned()),
            "pathspec": args.get(1).cloned()
        }),
        "shell.run" => serde_json::json!({
            "command": args.join(" ")
        }),
        _ => serde_json::json!({}),
    }
}

fn required_text(parts: Vec<String>, message: &str) -> Result<String, Box<dyn Error>> {
    let text = parts.join(" ");
    if text.trim().is_empty() {
        Err(invalid_input(message).into())
    } else {
        Ok(text)
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn print_help() {
    println!(
        "cadis {}\n\nUSAGE:\n  cadis [--socket PATH] [--json] <COMMAND>\n\nCOMMANDS:\n  daemon [ARGS...]       Launch cadisd from PATH or sibling target directory\n  status                 Show daemon status\n  doctor                 Check local config and daemon connectivity\n  models                 List model provider options\n  agents                 List daemon-owned agents\n  worker <COMMAND>       Inspect daemon-owned workers\n  workspace <COMMAND>    Manage registered workspaces and grants\n  session <COMMAND>      Manage session event streams\n  voice [COMMAND]        Show daemon-visible voice status or doctor checks\n  events [OPTIONS]       Subscribe to daemon runtime events\n  spawn <ROLE> [OPTIONS] Spawn a child/subagent\n  chat <MESSAGE>         Send a one-shot chat message\n  run [--cwd PATH] <TASK> Send a desktop MVP task as a chat request\n  tool [OPTIONS] <NAME>  Request a daemon-owned tool call\n  approve <ID>           Respond to an approval request\n  deny <ID>              Deny an approval request\n\nWORKER COMMANDS:\n  worker tail <ID> [--lines COUNT]\n\nWORKSPACE COMMANDS:\n  workspace list [--grants]\n  workspace register <ID> <ROOT> [--kind project|documents|sandbox|worktree]\n  workspace grant <ID> [--access read,write,exec,admin] [--agent AGENT]\n  workspace revoke (--grant ID | --workspace ID)\n  workspace doctor [--workspace ID] [--root PATH]\n\nSESSION COMMANDS:\n  session subscribe <ID> [--replay COUNT] [--since EVENT_ID] [--no-snapshot]\n\nVOICE COMMANDS:\n  voice status           Show daemon-visible voice status\n  voice doctor           Show voice doctor and local bridge preflight state\n\nEVENT OPTIONS:\n  --snapshot             Print one daemon-owned state snapshot and exit\n  --replay <COUNT>       Replay up to COUNT buffered events before live events\n  --since <EVENT_ID>     Replay retained events after EVENT_ID\n  --no-snapshot          Subscribe without initial state snapshot\n\nSPAWN OPTIONS:\n  --name <NAME>          Display name for the new agent\n  --parent <AGENT>       Parent agent ID, default main\n  --model <MODEL>        Provider/model identifier\n\nTOOL OPTIONS:\n  --cwd <PATH>           Workspace root for file and git tools\n  --workspace <ID>       Registered workspace ID for file and git tools\n  --session <ID>         Attach the tool call to a session\n  --agent <ID>           Use an agent context for scoped workspace grants\n  --input <JSON>         Structured tool input\n\nGLOBAL OPTIONS:\n  --socket <PATH>        Unix socket path\n  --json                 Print NDJSON server frames\n  --version, -V          Print version\n  --help, -h             Print help",
        env!("CARGO_PKG_VERSION")
    );
}
