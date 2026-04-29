use std::io;
use std::path::PathBuf;

use cadis_protocol::{AgentId, AgentStatus, ApprovalId, ServerFrame};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Args {
    pub(crate) socket_path: Option<PathBuf>,
    pub(crate) version: bool,
}

impl Args {
    pub(crate) fn parse<I>(args: I) -> Result<Self, Box<dyn std::error::Error>>
    where
        I: IntoIterator<Item = String>,
    {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--socket" => {
                    parsed.socket_path = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| invalid_input("--socket requires a path"))?,
                    ));
                }
                "--version" | "-V" => parsed.version = true,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(invalid_input(format!("unknown argument: {other}")).into()),
            }
        }
        Ok(parsed)
    }
}

fn print_help() {
    println!(
        "cadis-hud {}\n\nUSAGE:\n  cadis-hud [--socket PATH]\n\nOPTIONS:\n  --socket <PATH>   Unix socket path for cadisd\n  --version, -V     Print version\n  --help, -h        Print help",
        env!("CARGO_PKG_VERSION")
    );
}

pub(crate) fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[derive(Clone, Debug)]
pub(crate) struct HudResult {
    pub(crate) result: Result<Vec<ServerFrame>, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentView {
    pub(crate) id: AgentId,
    pub(crate) name: String,
    pub(crate) role: String,
    pub(crate) status: AgentStatus,
    pub(crate) task: Option<String>,
    pub(crate) model: String,
    pub(crate) workers: Vec<String>,
}

pub(crate) fn default_agents() -> Vec<AgentView> {
    vec![
        default_main_agent(),
        AgentView {
            id: AgentId::from("coder"),
            name: "Coder".to_owned(),
            role: "worker".to_owned(),
            status: AgentStatus::Idle,
            task: None,
            model: "auto".to_owned(),
            workers: vec!["tester idle".to_owned(), "reviewer idle".to_owned()],
        },
        AgentView {
            id: AgentId::from("researcher"),
            name: "Researcher".to_owned(),
            role: "agent".to_owned(),
            status: AgentStatus::Idle,
            task: None,
            model: "auto".to_owned(),
            workers: Vec::new(),
        },
    ]
}

pub(crate) fn default_main_agent() -> AgentView {
    AgentView {
        id: AgentId::from("main"),
        name: "CADIS".to_owned(),
        role: "main".to_owned(),
        status: AgentStatus::Idle,
        task: None,
        model: "auto".to_owned(),
        workers: Vec::new(),
    }
}

pub(crate) fn placeholder_agent(index: usize) -> AgentView {
    AgentView {
        id: AgentId::from(format!("slot_{index}")),
        name: format!("Slot {}", index + 1),
        role: "reserved".to_owned(),
        status: AgentStatus::Idle,
        task: None,
        model: "waiting".to_owned(),
        workers: Vec::new(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ApprovalView {
    pub(crate) id: ApprovalId,
    pub(crate) risk: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) command: String,
    pub(crate) workspace: String,
    pub(crate) waiting_resolution: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ModelOption {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) display_name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ChatMessage {
    pub(crate) role: ChatRole,
    pub(crate) text: String,
}

impl ChatMessage {
    pub(crate) fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            text: text.into(),
        }
    }

    pub(crate) fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            text: text.into(),
        }
    }

    pub(crate) fn system(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            text: text.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChatRole {
    User,
    Assistant,
    System,
}

impl ChatRole {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Assistant => "CADIS",
            Self::System => "SYS",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigTab {
    Voice,
    Models,
    Appearance,
    Window,
}

impl ConfigTab {
    pub(crate) fn all() -> [Self; 4] {
        [Self::Voice, Self::Models, Self::Appearance, Self::Window]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Voice => "Voice",
            Self::Models => "Models",
            Self::Appearance => "Appearance",
            Self::Window => "Window",
        }
    }
}
