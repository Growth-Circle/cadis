use std::collections::VecDeque;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use cadis_core::{PendingMessageGeneration, Runtime, RuntimeOptions};
use cadis_models::{provider_from_config, ModelError, ModelRequest, ModelStreamEvent};
use cadis_protocol::{
    ClientRequest, DaemonResponse, ErrorPayload, EventEnvelope, EventId, EventSubscriptionRequest,
    RequestEnvelope, RequestId, ResponseEnvelope, ServerFrame, SessionId,
    SessionSubscriptionRequest,
};
use cadis_store::{
    ensure_layout, load_config, openai_api_key_from_env, redact, CadisConfig, EventLog,
};

const EVENT_REPLAY_LIMIT: usize = 256;

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
    let event_bus = EventBus::new(EVENT_REPLAY_LIMIT);

    if args.stdio {
        let stdin = io::stdin();
        let stdout = io::stdout();
        serve_lines(stdin.lock(), stdout.lock(), runtime, event_log, event_bus)?;
        return Ok(());
    }

    run_socket(socket_path, runtime, event_log, event_bus)
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
            profile_id: config.profile.default_profile.clone(),
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
    event_bus: EventBus,
) -> Result<(), Box<dyn Error>> {
    prepare_socket_path(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    eprintln!("cadisd listening on {}", socket_path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let runtime = Arc::clone(&runtime);
                let event_log = event_log.clone();
                let event_bus = event_bus.clone();
                thread::spawn(move || {
                    if let Err(error) = serve_unix_stream(stream, runtime, event_log, event_bus) {
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
    event_bus: EventBus,
) -> Result<(), Box<dyn Error>> {
    let reader = BufReader::new(stream.try_clone()?);
    let writer = BufWriter::new(stream);
    serve_lines(reader, writer, runtime, event_log, event_bus)
}

fn serve_lines<R, W>(
    reader: R,
    writer: W,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
    event_bus: EventBus,
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
                let subscription = match &envelope.request {
                    ClientRequest::EventsSubscribe(request) => {
                        Some(EventBusSubscription::all(request))
                    }
                    ClientRequest::SessionSubscribe(request) => {
                        Some(EventBusSubscription::session(request))
                    }
                    _ => None,
                };
                let snapshot_only = matches!(envelope.request, ClientRequest::EventsSnapshot(_));
                if matches!(envelope.request, ClientRequest::MessageSend(_)) {
                    let pending = runtime
                        .lock()
                        .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                        .begin_message_request(envelope);
                    match pending {
                        Ok(pending) => {
                            serve_pending_message_generation(
                                &mut writer,
                                Arc::clone(&runtime),
                                &event_log,
                                &event_bus,
                                pending,
                            )?;
                        }
                        Err(outcome) => {
                            let outcome = *outcome;
                            write_frame(&mut writer, &ServerFrame::Response(outcome.response))?;
                            for event in outcome.events {
                                emit_event(&mut writer, &event_log, &event_bus, event)?;
                            }
                        }
                    }
                    continue;
                }
                let outcome = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                    .handle_request(envelope);
                let accepted_subscription = matches!(
                    &outcome.response.response,
                    DaemonResponse::RequestAccepted(_)
                )
                .then_some(subscription)
                .flatten();

                write_frame(&mut writer, &ServerFrame::Response(outcome.response))?;

                if let Some(subscription) = accepted_subscription {
                    for event in outcome.events {
                        write_frame(&mut writer, &ServerFrame::Event(event))?;
                    }

                    let (replay, receiver) = event_bus.subscribe(subscription);
                    for event in replay {
                        write_frame(&mut writer, &ServerFrame::Event(event))?;
                    }
                    for event in receiver {
                        write_frame(&mut writer, &ServerFrame::Event(event))?;
                    }
                    return Ok(());
                }

                if snapshot_only {
                    for event in outcome.events {
                        write_frame(&mut writer, &ServerFrame::Event(event))?;
                    }
                    continue;
                }

                for event in outcome.events {
                    emit_event(&mut writer, &event_log, &event_bus, event)?;
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

fn serve_pending_message_generation<W: Write>(
    writer: &mut W,
    runtime: Arc<Mutex<Runtime>>,
    event_log: &EventLog,
    event_bus: &EventBus,
    pending: PendingMessageGeneration,
) -> Result<(), Box<dyn Error>> {
    write_frame(writer, &ServerFrame::Response(pending.response.clone()))?;
    for event in pending.initial_events.clone() {
        emit_event(writer, event_log, event_bus, event)?;
    }

    let provider = Arc::clone(&pending.provider);
    let mut invocation = None;
    let mut final_content = String::new();
    let mut emitted_delta = false;
    let stream_result = provider.stream_chat(
        ModelRequest::new(&pending.prompt).with_selected_model(pending.selected_model.as_deref()),
        &mut |event| {
            match event {
                ModelStreamEvent::Started(started) | ModelStreamEvent::Completed(started) => {
                    invocation = Some(started);
                }
                ModelStreamEvent::Delta(delta) => {
                    final_content.push_str(&delta);
                    emitted_delta = true;
                    let event = runtime
                        .lock()
                        .map_err(|_| {
                            ModelError::with_code(
                                "runtime_lock_failed",
                                "runtime mutex was poisoned",
                                false,
                            )
                        })?
                        .message_delta_event(&pending, delta, invocation.as_ref());
                    emit_event(writer, event_log, event_bus, event).map_err(|error| {
                        ModelError::with_code("event_write_failed", error.to_string(), false)
                    })?;
                }
                ModelStreamEvent::Failed(_) => {}
            }
            Ok(())
        },
    );

    let final_events = match stream_result {
        Ok(response) => runtime
            .lock()
            .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
            .complete_message_generation(pending, response, final_content, emitted_delta),
        Err(error) => runtime
            .lock()
            .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
            .fail_message_generation(pending, error),
    };

    for event in final_events {
        emit_event(writer, event_log, event_bus, event)?;
    }

    Ok(())
}

fn emit_event<W: Write>(
    writer: &mut W,
    event_log: &EventLog,
    event_bus: &EventBus,
    event: EventEnvelope,
) -> Result<(), Box<dyn Error>> {
    publish_event(event_log, event_bus, &event);
    write_frame(writer, &ServerFrame::Event(event))
}

fn publish_event(event_log: &EventLog, event_bus: &EventBus, event: &EventEnvelope) {
    if let Err(error) = event_log.append_event(event) {
        eprintln!("cadisd log error: {error}");
    }
    event_bus.publish(event.clone());
}

#[derive(Clone)]
struct EventBus {
    inner: Arc<Mutex<EventBusInner>>,
    max_replay: usize,
}

struct EventBusInner {
    replay: VecDeque<EventEnvelope>,
    subscribers: Vec<EventSubscriber>,
}

struct EventSubscriber {
    subscription: EventBusSubscription,
    sender: mpsc::Sender<EventEnvelope>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EventBusSubscription {
    since_event_id: Option<EventId>,
    replay_limit: Option<u32>,
    filter: EventFilter,
}

impl EventBusSubscription {
    fn all(request: &EventSubscriptionRequest) -> Self {
        Self {
            since_event_id: request.since_event_id.clone(),
            replay_limit: request.replay_limit,
            filter: EventFilter::All,
        }
    }

    fn session(request: &SessionSubscriptionRequest) -> Self {
        Self {
            since_event_id: request.since_event_id.clone(),
            replay_limit: request.replay_limit,
            filter: EventFilter::Session(request.session_id.clone()),
        }
    }

    fn matches(&self, event: &EventEnvelope) -> bool {
        self.filter.matches(event)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EventFilter {
    All,
    Session(SessionId),
}

impl EventFilter {
    fn matches(&self, event: &EventEnvelope) -> bool {
        match self {
            Self::All => true,
            Self::Session(session_id) => event.session_id.as_ref() == Some(session_id),
        }
    }
}

impl EventBus {
    fn new(max_replay: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventBusInner {
                replay: VecDeque::with_capacity(max_replay),
                subscribers: Vec::new(),
            })),
            max_replay,
        }
    }

    fn publish(&self, event: EventEnvelope) {
        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("cadisd event bus error: event bus mutex was poisoned");
            return;
        };

        if self.max_replay > 0 {
            while inner.replay.len() >= self.max_replay {
                inner.replay.pop_front();
            }
            inner.replay.push_back(event.clone());
        }

        inner.subscribers.retain(|subscriber| {
            !subscriber.subscription.matches(&event)
                || subscriber.sender.send(event.clone()).is_ok()
        });
    }

    fn subscribe(
        &self,
        subscription: EventBusSubscription,
    ) -> (Vec<EventEnvelope>, mpsc::Receiver<EventEnvelope>) {
        let (sender, receiver) = mpsc::channel();
        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("cadisd event bus error: event bus mutex was poisoned");
            return (Vec::new(), receiver);
        };

        let replay = bounded_replay(
            &inner.replay,
            subscription.since_event_id.as_ref(),
            subscription.replay_limit,
            self.max_replay,
            &subscription.filter,
        );
        inner.subscribers.push(EventSubscriber {
            subscription,
            sender,
        });
        (replay, receiver)
    }
}

fn bounded_replay(
    replay: &VecDeque<EventEnvelope>,
    since_event_id: Option<&EventId>,
    replay_limit: Option<u32>,
    max_replay: usize,
    filter: &EventFilter,
) -> Vec<EventEnvelope> {
    let limit = replay_limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(max_replay)
        .min(max_replay);
    if limit == 0 || replay.is_empty() {
        return Vec::new();
    }

    let start_index = since_event_id
        .and_then(|event_id| replay.iter().position(|event| &event.event_id == event_id))
        .map(|index| index + 1)
        .unwrap_or(0);
    let available = replay
        .iter()
        .skip(start_index)
        .filter(|event| filter.matches(event))
        .cloned()
        .collect::<Vec<_>>();
    let start = available.len().saturating_sub(limit);
    available[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadis_models::{ModelInvocation, ModelProvider, ModelResponse};
    use cadis_protocol::{
        CadisEvent, ClientId, ContentKind, DaemonResponse, EmptyPayload, MessageSendRequest,
        RequestEnvelope, SessionEventPayload, Timestamp,
    };
    use std::sync::Condvar;

    #[test]
    fn bounded_replay_returns_events_after_retained_event_id() {
        let replay = VecDeque::from(vec![
            event("evt_000001"),
            event("evt_000002"),
            event("evt_000003"),
        ]);

        let events = bounded_replay(
            &replay,
            Some(&EventId::from("evt_000001")),
            Some(1),
            8,
            &EventFilter::All,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id.as_str(), "evt_000003");
    }

    #[test]
    fn event_bus_fans_out_published_runtime_events() {
        let bus = EventBus::new(8);
        let (_replay, receiver) =
            bus.subscribe(EventBusSubscription::all(&EventSubscriptionRequest {
                include_snapshot: false,
                replay_limit: Some(8),
                since_event_id: None,
            }));

        bus.publish(event("evt_000001"));

        let received = receiver
            .try_recv()
            .expect("subscriber should receive event");
        assert_eq!(received.event_id.as_str(), "evt_000001");
    }

    #[test]
    fn event_bus_replays_only_matching_session_events() {
        let bus = EventBus::new(8);
        bus.publish(session_event("evt_000001", "ses_target"));
        bus.publish(session_event("evt_000002", "ses_other"));
        bus.publish(session_event("evt_000003", "ses_target"));

        let (replay, _receiver) = bus.subscribe(EventBusSubscription {
            since_event_id: Some(EventId::from("evt_000001")),
            replay_limit: Some(8),
            filter: EventFilter::Session(SessionId::from("ses_target")),
        });

        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].event_id.as_str(), "evt_000003");
    }

    #[test]
    fn event_bus_fans_out_session_events_to_two_filtered_clients() {
        let bus = EventBus::new(8);
        let subscription = EventBusSubscription {
            since_event_id: None,
            replay_limit: Some(8),
            filter: EventFilter::Session(SessionId::from("ses_target")),
        };
        let (_replay, left) = bus.subscribe(subscription.clone());
        let (_replay, right) = bus.subscribe(subscription);

        bus.publish(session_event("evt_000001", "ses_other"));
        assert!(left.try_recv().is_err());
        assert!(right.try_recv().is_err());

        bus.publish(session_event("evt_000002", "ses_target"));

        let left_event = left
            .try_recv()
            .expect("left subscriber should receive event");
        let right_event = right
            .try_recv()
            .expect("right subscriber should receive event");
        assert_eq!(left_event.event_id.as_str(), "evt_000002");
        assert_eq!(right_event.event_id.as_str(), "evt_000002");
    }

    #[test]
    fn event_bus_keeps_unmatched_filtered_subscribers() {
        let bus = EventBus::new(8);
        let (_replay, receiver) = bus.subscribe(EventBusSubscription {
            since_event_id: None,
            replay_limit: Some(8),
            filter: EventFilter::Session(SessionId::from("ses_target")),
        });

        bus.publish(session_event("evt_000001", "ses_other"));
        bus.publish(session_event("evt_000002", "ses_target"));

        let received = receiver
            .try_recv()
            .expect("subscriber should remain attached until a matching event");
        assert_eq!(received.event_id.as_str(), "evt_000002");
    }

    #[test]
    fn pending_message_generation_leaves_runtime_mutex_available() {
        let entered = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let runtime = Arc::new(Mutex::new(Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cadis-daemon-runtime-lock"),
                profile_id: "default".to_owned(),
                socket_path: None,
                model_provider: "waiting".to_owned(),
                ui_preferences: serde_json::json!({}),
            },
            Box::new(WaitingProvider {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
        )));
        let pending = runtime
            .lock()
            .expect("runtime mutex should lock")
            .begin_message_request(RequestEnvelope::new(
                RequestId::from("req_chat"),
                ClientId::from("cli_1"),
                ClientRequest::MessageSend(MessageSendRequest {
                    session_id: None,
                    target_agent_id: None,
                    content: "hello".to_owned(),
                    content_kind: ContentKind::Chat,
                }),
            ))
            .expect("message generation should be prepared");
        let selected_model = pending.selected_model.clone();
        let prompt = pending.prompt.clone();
        let provider = Arc::clone(&pending.provider);
        let worker = thread::spawn(move || {
            provider
                .stream_chat(
                    ModelRequest::new(&prompt).with_selected_model(selected_model.as_deref()),
                    &mut |_event| Ok(()),
                )
                .expect("waiting provider should complete")
        });

        wait_for_flag(&entered);
        let status = runtime
            .try_lock()
            .expect("runtime mutex should not be held during provider generation")
            .handle_request(RequestEnvelope::new(
                RequestId::from("req_status"),
                ClientId::from("cli_2"),
                ClientRequest::DaemonStatus(EmptyPayload::default()),
            ));
        assert!(matches!(
            status.response.response,
            DaemonResponse::DaemonStatus(_)
        ));

        set_flag(&release);
        let response = worker.join().expect("provider thread should not panic");
        assert_eq!(response.deltas, vec!["done".to_owned()]);
    }

    #[derive(Clone, Debug)]
    struct WaitingProvider {
        entered: Arc<(Mutex<bool>, Condvar)>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl ModelProvider for WaitingProvider {
        fn name(&self) -> &str {
            "waiting"
        }

        fn chat(&self, _prompt: &str) -> Result<Vec<String>, ModelError> {
            Ok(vec!["done".to_owned()])
        }

        fn stream_chat(
            &self,
            request: ModelRequest<'_>,
            callback: &mut cadis_models::ModelStreamCallback<'_>,
        ) -> Result<ModelResponse, ModelError> {
            set_flag(&self.entered);
            wait_for_flag(&self.release);
            let invocation = ModelInvocation {
                requested_model: request.selected_model.map(ToOwned::to_owned),
                effective_provider: "waiting".to_owned(),
                effective_model: "unit-test".to_owned(),
                fallback: false,
                fallback_reason: None,
            };
            callback(ModelStreamEvent::Started(invocation.clone()))?;
            callback(ModelStreamEvent::Delta("done".to_owned()))?;
            callback(ModelStreamEvent::Completed(invocation.clone()))?;
            Ok(ModelResponse {
                deltas: vec!["done".to_owned()],
                invocation,
            })
        }
    }

    fn wait_for_flag(flag: &Arc<(Mutex<bool>, Condvar)>) {
        let (lock, condvar) = &**flag;
        let mut ready = lock.lock().expect("flag mutex should lock");
        while !*ready {
            ready = condvar
                .wait(ready)
                .expect("flag mutex should not be poisoned");
        }
    }

    fn set_flag(flag: &Arc<(Mutex<bool>, Condvar)>) {
        let (lock, condvar) = &**flag;
        *lock.lock().expect("flag mutex should lock") = true;
        condvar.notify_all();
    }

    fn test_workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{name}-{}-{}",
            process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("test workspace should be created");
        path
    }

    fn session_event(event_id: &str, session_id: &str) -> EventEnvelope {
        let session_id = SessionId::from(session_id);
        EventEnvelope::new(
            EventId::from(event_id),
            Timestamp::new_utc("2026-04-26T00:00:00Z").expect("timestamp should parse"),
            "cadisd",
            Some(session_id.clone()),
            CadisEvent::SessionStarted(SessionEventPayload {
                session_id,
                title: None,
            }),
        )
    }

    fn event(event_id: &str) -> EventEnvelope {
        EventEnvelope::new(
            EventId::from(event_id),
            Timestamp::new_utc("2026-04-26T00:00:00Z").expect("timestamp should parse"),
            "cadisd",
            None,
            CadisEvent::DaemonStarted(EmptyPayload::default()),
        )
    }
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
