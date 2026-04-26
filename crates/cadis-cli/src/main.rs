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
    ErrorPayload, MessageSendRequest, ModelsListPayload, RequestEnvelope, RequestId, ServerFrame,
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
    println!("cadis doctor");
    println!("cadis_home: {}", config.cadis_home.display());
    println!("config: {}", config.config_path().display());
    println!("socket: {}", cli.socket_path()?.display());

    let frames = send_request(cli, ClientRequest::DaemonStatus(EmptyPayload::default()))?;
    print!("daemon: ");
    match daemon_status(&frames) {
        Some(status) => {
            println!("{}", status.status);
            println!("model_provider: {}", status.model_provider);
            println!("sessions: {}", status.sessions);
            Ok(())
        }
        None => {
            println!("unexpected response");
            render_rejections_or_json(&frames, cli.json)
        }
    }
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
                        "{}\t{}\t{}",
                        model.provider, model.model, model.display_name
                    );
                }
            }
        }
    }
    Ok(())
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
            Some("spawn") => parse_spawn(args.collect())?,
            Some("chat") => {
                Command::Chat(required_text(args.collect(), "chat requires a message")?)
            }
            Some("run") => parse_run(args.collect())?,
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
    Approve(String),
    Deny(String),
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
        "cadis {}\n\nUSAGE:\n  cadis [--socket PATH] [--json] <COMMAND>\n\nCOMMANDS:\n  daemon [ARGS...]       Launch cadisd from PATH or sibling target directory\n  status                 Show daemon status\n  doctor                 Check local config and daemon connectivity\n  models                 List model provider options\n  agents                 List daemon-owned agents\n  spawn <ROLE> [OPTIONS] Spawn a child/subagent\n  chat <MESSAGE>         Send a one-shot chat message\n  run [--cwd PATH] <TASK> Send a desktop MVP task as a chat request\n  approve <ID>           Respond to an approval request\n  deny <ID>              Deny an approval request\n\nSPAWN OPTIONS:\n  --name <NAME>          Display name for the new agent\n  --parent <AGENT>       Parent agent ID, default main\n  --model <MODEL>        Provider/model identifier\n\nGLOBAL OPTIONS:\n  --socket <PATH>        Unix socket path\n  --json                 Print NDJSON server frames\n  --version, -V          Print version\n  --help, -h             Print help",
        env!("CARGO_PKG_VERSION")
    );
}
