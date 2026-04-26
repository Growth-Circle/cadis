use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;

use cadis_core::{Runtime, RuntimeOptions};
use cadis_models::provider_from_config;
use cadis_protocol::{
    DaemonResponse, ErrorPayload, RequestEnvelope, RequestId, ResponseEnvelope, ServerFrame,
};
use cadis_store::{
    ensure_layout, load_config, openai_api_key_from_env, redact, CadisConfig, EventLog,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("cadisd: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;

    if args.version {
        println!("cadisd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut config = load_config()?;
    apply_args_to_config(&args, &mut config);
    ensure_layout(&config)?;
    let socket_path = args
        .socket_path
        .clone()
        .unwrap_or_else(|| config.effective_socket_path());

    if args.check {
        print_check(&config, &socket_path);
        return Ok(());
    }

    let runtime = build_runtime(&config, Some(socket_path.clone()));
    let event_log = EventLog::new(&config);

    if args.stdio {
        let stdin = io::stdin();
        let stdout = io::stdout();
        serve_lines(stdin.lock(), stdout.lock(), runtime, event_log)?;
        return Ok(());
    }

    run_socket(socket_path, runtime, event_log)
}

fn build_runtime(config: &CadisConfig, socket_path: Option<PathBuf>) -> Arc<Mutex<Runtime>> {
    let provider = provider_from_config(
        &config.model.provider,
        &config.model.ollama_endpoint,
        &config.model.ollama_model,
        &config.model.openai_base_url,
        &config.model.openai_model,
        openai_api_key_from_env().as_deref(),
    );

    Arc::new(Mutex::new(Runtime::new(
        RuntimeOptions {
            cadis_home: config.cadis_home.clone(),
            socket_path,
            model_provider: config.model.provider.clone(),
            ui_preferences: config.ui_preferences(),
        },
        provider,
    )))
}

fn run_socket(
    socket_path: PathBuf,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
) -> Result<(), Box<dyn Error>> {
    prepare_socket_path(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    eprintln!("cadisd listening on {}", socket_path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let runtime = Arc::clone(&runtime);
                let event_log = event_log.clone();
                thread::spawn(move || {
                    if let Err(error) = serve_unix_stream(stream, runtime, event_log) {
                        eprintln!("cadisd client error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("cadisd accept error: {error}"),
        }
    }

    Ok(())
}

fn serve_unix_stream(
    stream: UnixStream,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
) -> Result<(), Box<dyn Error>> {
    let reader = BufReader::new(stream.try_clone()?);
    let writer = BufWriter::new(stream);
    serve_lines(reader, writer, runtime, event_log)
}

fn serve_lines<R, W>(
    reader: R,
    writer: W,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
) -> Result<(), Box<dyn Error>>
where
    R: BufRead,
    W: Write,
{
    let mut writer = writer;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<RequestEnvelope>(&line) {
            Ok(envelope) => {
                let outcome = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                    .handle_request(envelope);

                write_frame(&mut writer, &ServerFrame::Response(outcome.response))?;

                for event in outcome.events {
                    if let Err(error) = event_log.append_event(&event) {
                        eprintln!("cadisd log error: {error}");
                    }
                    write_frame(&mut writer, &ServerFrame::Event(event))?;
                }
            }
            Err(error) => {
                let response = ResponseEnvelope::new(
                    RequestId::from("req_invalid"),
                    DaemonResponse::RequestRejected(ErrorPayload {
                        code: "invalid_request".to_owned(),
                        message: format!("request JSON was invalid: {error}"),
                        retryable: false,
                    }),
                );
                write_frame(&mut writer, &ServerFrame::Response(response))?;
            }
        }
    }

    Ok(())
}

fn write_frame(writer: &mut impl Write, frame: &ServerFrame) -> Result<(), Box<dyn Error>> {
    serde_json::to_writer(&mut *writer, frame)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn prepare_socket_path(socket_path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            set_private_permissions(parent)?;
        }
    }

    if socket_path.exists() {
        if UnixStream::connect(socket_path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "daemon already appears to be running at {}",
                    socket_path.display()
                ),
            )
            .into());
        }
        fs::remove_file(socket_path)?;
    }

    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), Box<dyn Error>> {
    Ok(())
}

fn apply_args_to_config(args: &Args, config: &mut CadisConfig) {
    if let Some(provider) = &args.model_provider {
        config.model.provider = provider.clone();
    }
    if args.dev_echo {
        config.model.provider = "echo".to_owned();
    }
    if let Some(model) = &args.ollama_model {
        config.model.ollama_model = model.clone();
    }
    if let Some(endpoint) = &args.ollama_endpoint {
        config.model.ollama_endpoint = endpoint.clone();
    }
}

fn print_check(config: &CadisConfig, socket_path: &Path) {
    println!("cadisd check: ok");
    println!("cadis_home: {}", config.cadis_home.display());
    println!("config: {}", config.config_path().display());
    println!("socket: {}", socket_path.display());
    println!("model_provider: {}", config.model.provider);
    println!("ollama_model: {}", config.model.ollama_model);
    println!("ollama_endpoint: {}", config.model.ollama_endpoint);
    println!("openai_model: {}", config.model.openai_model);
    println!("openai_base_url: {}", redact(&config.model.openai_base_url));
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Args {
    check: bool,
    version: bool,
    stdio: bool,
    dev_echo: bool,
    socket_path: Option<PathBuf>,
    model_provider: Option<String>,
    ollama_model: Option<String>,
    ollama_endpoint: Option<String>,
}

impl Args {
    fn parse<I>(args: I) -> Result<Self, Box<dyn Error>>
    where
        I: IntoIterator<Item = String>,
    {
        let mut parsed = Self::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--check" => parsed.check = true,
                "--version" | "-V" => parsed.version = true,
                "--stdio" => parsed.stdio = true,
                "--dev-echo" => parsed.dev_echo = true,
                "--socket" => {
                    parsed.socket_path = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| invalid_input("--socket requires a path"))?,
                    ));
                }
                "--model-provider" => {
                    parsed.model_provider = Some(
                        args.next()
                            .ok_or_else(|| invalid_input("--model-provider requires a value"))?,
                    );
                }
                "--ollama-model" => {
                    parsed.ollama_model = Some(
                        args.next()
                            .ok_or_else(|| invalid_input("--ollama-model requires a value"))?,
                    );
                }
                "--ollama-endpoint" => {
                    parsed.ollama_endpoint = Some(
                        args.next()
                            .ok_or_else(|| invalid_input("--ollama-endpoint requires a value"))?,
                    );
                }
                "--help" | "-h" => {
                    print_help();
                    process::exit(0);
                }
                other => return Err(invalid_input(format!("unknown argument: {other}")).into()),
            }
        }

        Ok(parsed)
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn print_help() {
    println!(
        "cadisd {}\n\nUSAGE:\n  cadisd [OPTIONS]\n\nOPTIONS:\n  --check                    Validate config and local state layout\n  --version, -V              Print version\n  --stdio                    Serve NDJSON protocol on stdin/stdout\n  --socket <PATH>            Unix socket path\n  --model-provider <NAME>    auto, codex-cli, openai, ollama, or echo\n  --ollama-model <NAME>      Ollama model name\n  --ollama-endpoint <URL>    Ollama endpoint\n  --dev-echo                 Force credential-free local fallback\n  --help, -h                 Print help",
        env!("CARGO_PKG_VERSION")
    );
}
