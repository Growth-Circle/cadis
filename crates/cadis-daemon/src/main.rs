use std::collections::VecDeque;
use std::env;
use std::error::Error;
#[cfg(unix)]
use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(all(test, unix))]
use std::io::{BufReader, BufWriter};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

#[cfg(all(test, unix))]
use std::thread;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener as TokioTcpListener;
#[cfg(unix)]
use tokio::net::UnixListener as TokioUnixListener;
use tokio::sync::mpsc as tokio_mpsc;

use cadis_core::{parse_tool_call_directives, PendingMessageGeneration, Runtime, RuntimeOptions};
use cadis_models::{
    provider_from_config, ModelError, ModelInvocation, ModelRequest, ModelStreamControl,
    ModelStreamEvent,
};
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime should build");
    if let Err(error) = rt.block_on(run()) {
        eprintln!("cadisd: {error}");
        process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;

    if args.version {
        println!("cadisd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut config = load_config()?;
    apply_args_to_config(&args, &mut config);
    ensure_layout(&config)?;

    let use_tcp = args.tcp_port.is_some() || cfg!(windows);
    let tcp_port = args.tcp_port.or(config.tcp_port);
    let socket_path = args
        .socket_path
        .clone()
        .or_else(|| config.effective_socket_path());

    if args.check {
        print_check(&config, socket_path.as_deref(), tcp_port);
        return Ok(());
    }

    let runtime = build_runtime(&config, socket_path.clone());
    let event_log = EventLog::new(&config);
    let event_bus = EventBus::new(EVENT_REPLAY_LIMIT);

    if args.stdio {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let shutdown = AtomicBool::new(false);
        serve_lines_sync(
            stdin.lock(),
            stdout.lock(),
            runtime,
            event_log,
            event_bus,
            &shutdown,
        )?;
        return Ok(());
    }

    if use_tcp {
        let addr = config.effective_tcp_address();
        let addr = tcp_port.map(|p| format!("127.0.0.1:{p}")).unwrap_or(addr);
        return run_tcp(&addr, runtime, event_log, event_bus).await;
    }

    #[cfg(unix)]
    {
        let socket_path = socket_path
            .ok_or_else(|| invalid_input("socket path required (use --tcp-port on Windows)"))?;
        run_socket(socket_path, runtime, event_log, event_bus).await
    }

    #[cfg(not(unix))]
    {
        let addr = tcp_port
            .map(|p| format!("127.0.0.1:{p}"))
            .unwrap_or_else(|| config.effective_tcp_address());
        run_tcp(&addr, runtime, event_log, event_bus).await
    }
}

fn build_runtime(config: &CadisConfig, socket_path: Option<PathBuf>) -> Arc<Mutex<Runtime>> {
    let openai_api_key = openai_api_key_from_env();
    let provider = provider_from_config(
        &config.model.provider,
        &config.model.ollama_endpoint,
        &config.model.ollama_model,
        &config.model.openai_base_url,
        &config.model.openai_model,
        openai_api_key.as_deref(),
    );

    Arc::new(Mutex::new(Runtime::new(
        RuntimeOptions {
            cadis_home: config.cadis_home.clone(),
            profile_id: config.profile.default_profile.clone(),
            socket_path,
            model_provider: config.model.provider.clone(),
            ollama_model: config.model.ollama_model.clone(),
            openai_model: config.model.openai_model.clone(),
            openai_api_key_configured: openai_api_key.is_some(),
            ui_preferences: config.ui_preferences(),
        },
        provider,
    )))
}

async fn run_tcp(
    addr: &str,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
    event_bus: EventBus,
) -> Result<(), Box<dyn Error>> {
    let listener = TokioTcpListener::bind(addr).await?;
    eprintln!("cadisd listening on tcp://{addr}");

    let shutdown = Arc::new(AtomicBool::new(false));

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let runtime = Arc::clone(&runtime);
                        let event_log = event_log.clone();
                        let event_bus = event_bus.clone();
                        let shutdown = Arc::clone(&shutdown);
                        tokio::spawn(async move {
                            let (reader, writer) = stream.into_split();
                            let reader = tokio::io::BufReader::new(reader);
                            if let Err(error) = serve_connection(
                                reader, writer, runtime, event_log, event_bus, shutdown,
                            ).await {
                                eprintln!("cadisd client error: {error}");
                            }
                        });
                    }
                    Err(error) => {
                        eprintln!("cadisd accept error: {error}");
                        break;
                    }
                }
            }
        }
    }

    eprintln!("cadisd shutting down");
    Ok(())
}

#[cfg(unix)]
async fn run_socket(
    socket_path: PathBuf,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
    event_bus: EventBus,
) -> Result<(), Box<dyn Error>> {
    prepare_socket_path(&socket_path)?;
    let listener = TokioUnixListener::bind(&socket_path)?;
    eprintln!("cadisd listening on {}", socket_path.display());

    let shutdown = Arc::new(AtomicBool::new(false));

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let runtime = Arc::clone(&runtime);
                        let event_log = event_log.clone();
                        let event_bus = event_bus.clone();
                        let shutdown = Arc::clone(&shutdown);
                        tokio::spawn(async move {
                            let (reader, writer) = stream.into_split();
                            let reader = tokio::io::BufReader::new(reader);
                            if let Err(error) = serve_connection(
                                reader, writer, runtime, event_log, event_bus, shutdown,
                            ).await {
                                eprintln!("cadisd client error: {error}");
                            }
                        });
                    }
                    Err(error) => {
                        eprintln!("cadisd accept error: {error}");
                        break;
                    }
                }
            }
        }
    }

    eprintln!("cadisd shutting down");
    let _ = fs::remove_file(&socket_path);
    Ok(())
}

fn serve_lines_sync<R, W>(
    reader: R,
    writer: W,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
    event_bus: EventBus,
    shutdown: &AtomicBool,
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
                if matches!(envelope.request, ClientRequest::DaemonShutdown(_)) {
                    let response = ResponseEnvelope::new(
                        envelope.request_id,
                        DaemonResponse::RequestAccepted(cadis_protocol::RequestAcceptedPayload {
                            request_id: RequestId::from("daemon.shutdown"),
                        }),
                    );
                    write_frame(&mut writer, &ServerFrame::Response(response))?;
                    shutdown.store(true, Ordering::SeqCst);
                    return Ok(());
                }
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

async fn serve_connection<R, W>(
    reader: R,
    mut writer: W,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
    event_bus: EventBus,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
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
                if matches!(envelope.request, ClientRequest::DaemonShutdown(_)) {
                    let response = ResponseEnvelope::new(
                        envelope.request_id,
                        DaemonResponse::RequestAccepted(cadis_protocol::RequestAcceptedPayload {
                            request_id: RequestId::from("daemon.shutdown"),
                        }),
                    );
                    write_frame_async(&mut writer, &ServerFrame::Response(response)).await?;
                    shutdown.store(true, Ordering::SeqCst);
                    return Ok(());
                }
                if matches!(envelope.request, ClientRequest::MessageSend(_)) {
                    let pending = runtime
                        .lock()
                        .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                        .begin_message_request(envelope);
                    match pending {
                        Ok(pending) => {
                            serve_pending_message_generation_async(
                                &mut writer,
                                Arc::clone(&runtime),
                                &event_log,
                                &event_bus,
                                pending,
                            )
                            .await?;
                        }
                        Err(outcome) => {
                            let outcome = *outcome;
                            write_frame_async(
                                &mut writer,
                                &ServerFrame::Response(outcome.response),
                            )
                            .await?;
                            for event in outcome.events {
                                emit_event_async(&mut writer, &event_log, &event_bus, event)
                                    .await?;
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

                write_frame_async(&mut writer, &ServerFrame::Response(outcome.response)).await?;

                if let Some(subscription) = accepted_subscription {
                    for event in outcome.events {
                        write_frame_async(&mut writer, &ServerFrame::Event(event)).await?;
                    }

                    let (replay, receiver) = event_bus.subscribe(subscription);
                    for event in replay {
                        write_frame_async(&mut writer, &ServerFrame::Event(event)).await?;
                    }
                    for event in receiver {
                        write_frame_async(&mut writer, &ServerFrame::Event(event)).await?;
                    }
                    return Ok(());
                }

                if snapshot_only {
                    for event in outcome.events {
                        write_frame_async(&mut writer, &ServerFrame::Event(event)).await?;
                    }
                    continue;
                }

                for event in outcome.events {
                    emit_event_async(&mut writer, &event_log, &event_bus, event).await?;
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
                write_frame_async(&mut writer, &ServerFrame::Response(response)).await?;
            }
        }
    }

    Ok(())
}

/// Result of one tool loop iteration, used to avoid holding MutexGuard across await points.
enum ToolLoopResult {
    Break(Vec<EventEnvelope>),
    Continue {
        events: Vec<EventEnvelope>,
        follow_up_prompt: String,
        pending: Box<PendingMessageGeneration>,
    },
}

async fn serve_pending_message_generation_async<W>(
    writer: &mut W,
    runtime: Arc<Mutex<Runtime>>,
    event_log: &EventLog,
    event_bus: &EventBus,
    pending: PendingMessageGeneration,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    write_frame_async(writer, &ServerFrame::Response(pending.response.clone())).await?;
    for event in pending.initial_events.clone() {
        emit_event_async(writer, event_log, event_bus, event).await?;
    }

    let mut current_prompt = pending.prompt.clone();
    let mut current_selected_model = pending.selected_model.clone();
    let mut current_pending = pending;

    loop {
        let handle = current_pending.handle();
        let (delta_tx, mut delta_rx) = tokio_mpsc::channel::<ModelStreamEvent>(64);

        let provider = Arc::clone(&current_pending.provider);
        let rt = Arc::clone(&runtime);
        let prompt_for_blocking = current_prompt.clone();
        let selected_model_for_blocking = current_selected_model.clone();
        let blocking_handle = tokio::task::spawn_blocking(move || {
            let mut invocation = None;
            let mut final_content = String::new();
            let mut emitted_delta = false;
            let stream_result = provider.stream_chat(
                ModelRequest::new(&prompt_for_blocking)
                    .with_selected_model(selected_model_for_blocking.as_deref()),
                &mut |event| {
                    if rt
                        .lock()
                        .map_err(|_| {
                            ModelError::with_code(
                                "runtime_lock_failed",
                                "runtime mutex was poisoned",
                                false,
                            )
                        })?
                        .message_generation_cancelled(&current_pending)
                    {
                        return Ok(ModelStreamControl::Cancel);
                    }

                    match &event {
                        ModelStreamEvent::Started(started)
                        | ModelStreamEvent::Completed(started) => {
                            invocation = Some(started.clone());
                        }
                        ModelStreamEvent::Delta(delta) => {
                            final_content.push_str(delta);
                            emitted_delta = true;
                        }
                        ModelStreamEvent::Failed(_) | ModelStreamEvent::Cancelled(_) => {}
                    }
                    let _ = delta_tx.blocking_send(event);
                    Ok(ModelStreamControl::Continue)
                },
            );
            (stream_result, final_content, emitted_delta, current_pending)
        });

        // Receive streaming events and emit deltas in real-time.
        let mut invocation: Option<ModelInvocation> = None;
        while let Some(event) = delta_rx.recv().await {
            match event {
                ModelStreamEvent::Started(ref started)
                | ModelStreamEvent::Completed(ref started) => {
                    invocation = Some(started.clone());
                }
                ModelStreamEvent::Delta(delta) => {
                    let delta_event = runtime
                        .lock()
                        .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                        .message_delta_event_from_handle(&handle, delta, invocation.as_ref());
                    emit_event_async(writer, event_log, event_bus, delta_event).await?;
                }
                ModelStreamEvent::Failed(_) | ModelStreamEvent::Cancelled(_) => {}
            }
        }

        let (stream_result, final_content, emitted_delta, pending_back) = blocking_handle
            .await
            .map_err(|e| io::Error::other(format!("spawn_blocking join error: {e}")))?;

        match stream_result {
            Ok(response) => {
                // Check for tool call directives before completing.
                let directives = parse_tool_call_directives(&final_content);
                if directives.is_empty() {
                    // No tool calls — complete normally.
                    let final_events = runtime
                        .lock()
                        .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                        .complete_message_generation(
                            pending_back,
                            response,
                            final_content,
                            emitted_delta,
                        );
                    for event in final_events {
                        emit_event_async(writer, event_log, event_bus, event).await?;
                    }
                    break;
                }

                // Tool loop: execute directives and prepare follow-up.
                let tool_loop_result = {
                    let mut rt_guard = runtime
                        .lock()
                        .map_err(|_| io::Error::other("runtime mutex was poisoned"))?;

                    // Consume a step for this tool loop iteration.
                    let (step_event, budget_exceeded) =
                        rt_guard.consume_step_for_tool_loop(&handle.agent_session_id);

                    if budget_exceeded {
                        let mut all_events = step_event.into_iter().collect::<Vec<_>>();
                        let final_events = rt_guard.complete_message_generation(
                            pending_back,
                            response,
                            final_content,
                            emitted_delta,
                        );
                        all_events.extend(final_events);
                        ToolLoopResult::Break(all_events)
                    } else {
                        let mut tool_events: Vec<EventEnvelope> = step_event.into_iter().collect();
                        let mut tool_results: Vec<(String, String)> = Vec::new();
                        let mut approval_blocked = false;

                        for directive in &directives {
                            let (events, summary) = rt_guard.execute_tool_in_loop(
                                &handle.session_id,
                                &handle.agent_id,
                                directive,
                            );
                            let blocked = summary.starts_with("[tool blocked]");
                            tool_events.extend(events);
                            tool_results.push((directive.tool_name.clone(), summary));
                            if blocked {
                                approval_blocked = true;
                                break;
                            }
                        }

                        if approval_blocked {
                            let final_events = rt_guard.complete_message_generation(
                                pending_back,
                                response,
                                final_content,
                                emitted_delta,
                            );
                            tool_events.extend(final_events);
                            ToolLoopResult::Break(tool_events)
                        } else {
                            let follow_up_prompt = rt_guard.build_tool_loop_prompt(
                                &handle.agent_id,
                                &final_content,
                                &tool_results,
                            );
                            ToolLoopResult::Continue {
                                events: tool_events,
                                follow_up_prompt,
                                pending: Box::new(pending_back),
                            }
                        }
                    }
                };

                match tool_loop_result {
                    ToolLoopResult::Break(events) => {
                        for event in events {
                            emit_event_async(writer, event_log, event_bus, event).await?;
                        }
                        break;
                    }
                    ToolLoopResult::Continue {
                        events,
                        follow_up_prompt,
                        mut pending,
                    } => {
                        for event in events {
                            emit_event_async(writer, event_log, event_bus, event).await?;
                        }
                        current_prompt = follow_up_prompt;
                        current_selected_model = pending.selected_model.clone();
                        pending.prompt = current_prompt.clone();
                        current_pending = *pending;
                    }
                }
            }
            Err(error) => {
                let final_events = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                    .fail_message_generation(pending_back, error);
                for event in final_events {
                    emit_event_async(writer, event_log, event_bus, event).await?;
                }
                break;
            }
        }
    }

    Ok(())
}

async fn emit_event_async<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    event_log: &EventLog,
    event_bus: &EventBus,
    event: EventEnvelope,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    publish_event(event_log, event_bus, &event);
    write_frame_async(writer, &ServerFrame::Event(event)).await
}

async fn write_frame_async<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &ServerFrame,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut buf = serde_json::to_vec(frame)?;
    buf.push(b'\n');
    writer.write_all(&buf).await?;
    writer.flush().await?;
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

    let mut current_pending = pending;

    loop {
        let provider = Arc::clone(&current_pending.provider);
        let mut invocation = None;
        let mut final_content = String::new();
        let mut emitted_delta = false;
        let stream_result = provider.stream_chat(
            ModelRequest::new(&current_pending.prompt)
                .with_selected_model(current_pending.selected_model.as_deref()),
            &mut |event| {
                if runtime
                    .lock()
                    .map_err(|_| {
                        ModelError::with_code(
                            "runtime_lock_failed",
                            "runtime mutex was poisoned",
                            false,
                        )
                    })?
                    .message_generation_cancelled(&current_pending)
                {
                    return Ok(ModelStreamControl::Cancel);
                }

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
                            .message_delta_event(&current_pending, delta, invocation.as_ref());
                        emit_event(writer, event_log, event_bus, event).map_err(|error| {
                            ModelError::with_code("event_write_failed", error.to_string(), false)
                        })?;
                    }
                    ModelStreamEvent::Failed(_) | ModelStreamEvent::Cancelled(_) => {}
                }
                Ok(ModelStreamControl::Continue)
            },
        );

        match stream_result {
            Ok(response) => {
                let directives = parse_tool_call_directives(&final_content);
                if directives.is_empty() {
                    let final_events = runtime
                        .lock()
                        .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                        .complete_message_generation(
                            current_pending,
                            response,
                            final_content,
                            emitted_delta,
                        );
                    for event in final_events {
                        emit_event(writer, event_log, event_bus, event)?;
                    }
                    break;
                }

                // Tool loop iteration.
                let mut rt_guard = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime mutex was poisoned"))?;

                let handle = current_pending.handle();
                let (step_event, budget_exceeded) =
                    rt_guard.consume_step_for_tool_loop(&handle.agent_session_id);
                if let Some(event) = step_event {
                    emit_event(writer, event_log, event_bus, event)?;
                }
                if budget_exceeded {
                    let final_events = rt_guard.complete_message_generation(
                        current_pending,
                        response,
                        final_content,
                        emitted_delta,
                    );
                    drop(rt_guard);
                    for event in final_events {
                        emit_event(writer, event_log, event_bus, event)?;
                    }
                    break;
                }

                let mut tool_results: Vec<(String, String)> = Vec::new();
                let mut approval_blocked = false;
                for directive in &directives {
                    let (events, summary) = rt_guard.execute_tool_in_loop(
                        &handle.session_id,
                        &handle.agent_id,
                        directive,
                    );
                    let blocked = summary.starts_with("[tool blocked]");
                    for event in events {
                        emit_event(writer, event_log, event_bus, event)?;
                    }
                    tool_results.push((directive.tool_name.clone(), summary));
                    if blocked {
                        approval_blocked = true;
                        break;
                    }
                }

                if approval_blocked {
                    let final_events = rt_guard.complete_message_generation(
                        current_pending,
                        response,
                        final_content,
                        emitted_delta,
                    );
                    drop(rt_guard);
                    for event in final_events {
                        emit_event(writer, event_log, event_bus, event)?;
                    }
                    break;
                }

                let follow_up_prompt = rt_guard.build_tool_loop_prompt(
                    &handle.agent_id,
                    &final_content,
                    &tool_results,
                );
                drop(rt_guard);

                current_pending.prompt = follow_up_prompt;
            }
            Err(error) => {
                let final_events = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime mutex was poisoned"))?
                    .fail_message_generation(current_pending, error);
                for event in final_events {
                    emit_event(writer, event_log, event_bus, event)?;
                }
                break;
            }
        }
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
    #[cfg(unix)]
    use cadis_models::{ModelInvocation, ModelProvider, ModelResponse};
    use cadis_protocol::{
        CadisEvent, EmptyPayload, EventId, SessionEventPayload, SessionId, Timestamp,
    };
    #[cfg(unix)]
    use cadis_protocol::{
        ClientId, ContentKind, DaemonResponse, MessageSendRequest, RequestEnvelope, RequestId,
        ServerFrame, SessionCreateRequest, SessionTargetRequest,
    };
    #[cfg(unix)]
    use std::os::unix::net::{UnixListener, UnixStream};
    #[cfg(unix)]
    use std::sync::Condvar;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

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
    #[cfg(unix)]
    fn prepare_socket_path_rejects_existing_regular_file_without_deleting_it() {
        let cadis_home = test_workspace("cd-regfile");
        let socket_path = cadis_home.join("run").join("cd.sock");
        fs::create_dir_all(
            socket_path
                .parent()
                .expect("socket path should have a parent"),
        )
        .expect("socket parent should be created");
        fs::write(&socket_path, "not a socket").expect("regular file should write");

        let error = prepare_socket_path(&socket_path).expect_err("regular file should be rejected");
        let io_error = error
            .downcast_ref::<io::Error>()
            .expect("prepare_socket_path should return an io error");

        assert_eq!(io_error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(&socket_path).expect("regular file should remain readable"),
            "not a socket"
        );
    }

    #[test]
    #[cfg(unix)]
    fn pending_message_generation_leaves_runtime_mutex_available() {
        let entered = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let runtime = Arc::new(Mutex::new(Runtime::new(
            RuntimeOptions {
                cadis_home: test_workspace("cd-lock"),
                profile_id: "default".to_owned(),
                socket_path: None,
                model_provider: "waiting".to_owned(),
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
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
                    &mut |_event| Ok(ModelStreamControl::Continue),
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

    #[test]
    #[cfg(unix)]
    fn socket_clients_share_session_events_while_status_and_agent_list_stay_live() {
        let cadis_home = test_workspace("cd-sub");
        let socket_path = cadis_home.join("run").join("cd.sock");
        let entered = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let config = CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        };
        ensure_layout(&config).expect("test CADIS layout should be created");

        let runtime = Arc::new(Mutex::new(Runtime::new(
            RuntimeOptions {
                cadis_home,
                profile_id: "default".to_owned(),
                socket_path: Some(socket_path.clone()),
                model_provider: "waiting".to_owned(),
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
                ui_preferences: serde_json::json!({}),
            },
            Box::new(WaitingProvider {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
        )));
        let event_log = EventLog::new(&config);
        let event_bus = EventBus::new(32);
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let accept_runtime = Arc::clone(&runtime);
        let accept_event_log = event_log.clone();
        let accept_event_bus = event_bus.clone();
        let accept_thread = thread::spawn(move || {
            let shutdown = Arc::new(AtomicBool::new(false));
            for _ in 0..4 {
                let (stream, _) = listener.accept().expect("test client should connect");
                let runtime = Arc::clone(&accept_runtime);
                let event_log = accept_event_log.clone();
                let event_bus = accept_event_bus.clone();
                let shutdown = Arc::clone(&shutdown);
                thread::spawn(move || {
                    let _ = serve_unix_stream(stream, runtime, event_log, event_bus, &shutdown);
                });
            }
        });

        let mut control = TestClient::connect(&socket_path);
        let mut subscriber_one = TestClient::connect(&socket_path);
        let mut subscriber_two = TestClient::connect(&socket_path);
        let mut messenger = TestClient::connect(&socket_path);
        accept_thread
            .join()
            .expect("test accept thread should not panic");

        control.send(RequestEnvelope::new(
            RequestId::from("req_create_session"),
            ClientId::from("cli_control"),
            ClientRequest::SessionCreate(SessionCreateRequest {
                title: Some("Socket fan-out".to_owned()),
                cwd: None,
            }),
        ));
        assert!(matches!(
            control.read_frame(),
            ServerFrame::Response(ResponseEnvelope {
                response: DaemonResponse::RequestAccepted(_),
                ..
            })
        ));
        let session_id = match control.read_frame() {
            ServerFrame::Event(EventEnvelope {
                event: CadisEvent::SessionStarted(payload),
                ..
            }) => payload.session_id,
            other => panic!("expected session.started event, got {other:?}"),
        };

        let subscribe = |request_id: &str| {
            RequestEnvelope::new(
                RequestId::from(request_id),
                ClientId::from(request_id),
                ClientRequest::SessionSubscribe(SessionSubscriptionRequest {
                    session_id: session_id.clone(),
                    since_event_id: None,
                    replay_limit: Some(16),
                    include_snapshot: false,
                }),
            )
        };
        subscriber_one.send(subscribe("req_subscribe_one"));
        subscriber_two.send(subscribe("req_subscribe_two"));
        assert_accepted_response(&mut subscriber_one);
        assert_accepted_response(&mut subscriber_two);

        messenger.send(RequestEnvelope::new(
            RequestId::from("req_message"),
            ClientId::from("cli_message"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: Some(session_id.clone()),
                target_agent_id: None,
                content: "hold generation open".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        assert_accepted_response(&mut messenger);

        wait_for_flag(&entered);

        let route_one = subscriber_one
            .read_event_matching(|event| matches!(event.event, CadisEvent::OrchestratorRoute(_)));
        let route_two = subscriber_two
            .read_event_matching(|event| matches!(event.event, CadisEvent::OrchestratorRoute(_)));
        assert_eq!(route_one.event_id, route_two.event_id);
        assert_eq!(route_one.session_id.as_ref(), Some(&session_id));
        assert_eq!(route_two.session_id.as_ref(), Some(&session_id));

        let status_one = subscriber_one
            .read_event_matching(|event| matches!(event.event, CadisEvent::AgentStatusChanged(_)));
        let status_two = subscriber_two
            .read_event_matching(|event| matches!(event.event, CadisEvent::AgentStatusChanged(_)));
        assert_eq!(status_one.event_id, status_two.event_id);

        control.send(RequestEnvelope::new(
            RequestId::from("req_status"),
            ClientId::from("cli_control"),
            ClientRequest::DaemonStatus(EmptyPayload::default()),
        ));
        match control.read_frame() {
            ServerFrame::Response(ResponseEnvelope {
                response: DaemonResponse::DaemonStatus(status),
                ..
            }) => assert_eq!(status.status, "ok"),
            other => panic!("expected daemon.status.response during generation, got {other:?}"),
        }

        control.send(RequestEnvelope::new(
            RequestId::from("req_agent_list"),
            ClientId::from("cli_control"),
            ClientRequest::AgentList(EmptyPayload::default()),
        ));
        assert_accepted_response(&mut control);
        let agent_list = control
            .read_event_matching(|event| matches!(event.event, CadisEvent::AgentListResponse(_)));
        assert!(agent_list.session_id.is_none());

        set_flag(&release);

        let delta_one = subscriber_one
            .read_event_matching(|event| matches!(event.event, CadisEvent::MessageDelta(_)));
        let delta_two = subscriber_two
            .read_event_matching(|event| matches!(event.event, CadisEvent::MessageDelta(_)));
        assert_eq!(delta_one.event_id, delta_two.event_id);

        let completed_one = subscriber_one
            .read_event_matching(|event| matches!(event.event, CadisEvent::MessageCompleted(_)));
        let completed_two = subscriber_two
            .read_event_matching(|event| matches!(event.event, CadisEvent::MessageCompleted(_)));
        assert_eq!(completed_one.event_id, completed_two.event_id);
    }

    #[test]
    #[cfg(unix)]
    fn session_cancel_propagates_to_active_provider_stream() {
        let cadis_home = test_workspace("cd-cancel");
        let socket_path = cadis_home.join("run").join("cd.sock");
        let first_delta = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let cancel_seen = Arc::new((Mutex::new(false), Condvar::new()));
        let config = CadisConfig {
            cadis_home: cadis_home.clone(),
            ..CadisConfig::default()
        };
        ensure_layout(&config).expect("test CADIS layout should be created");

        let runtime = Arc::new(Mutex::new(Runtime::new(
            RuntimeOptions {
                cadis_home,
                profile_id: "default".to_owned(),
                socket_path: Some(socket_path.clone()),
                model_provider: "cancellable".to_owned(),
                ollama_model: "llama3.2".to_owned(),
                openai_model: "gpt-5.2".to_owned(),
                openai_api_key_configured: false,
                ui_preferences: serde_json::json!({}),
            },
            Box::new(CancellableProvider {
                first_delta: Arc::clone(&first_delta),
                release: Arc::clone(&release),
                cancel_seen: Arc::clone(&cancel_seen),
            }),
        )));
        let event_log = EventLog::new(&config);
        let event_bus = EventBus::new(32);
        let listener = UnixListener::bind(&socket_path).expect("test socket should bind");
        let accept_runtime = Arc::clone(&runtime);
        let accept_event_log = event_log.clone();
        let accept_event_bus = event_bus.clone();
        let accept_thread = thread::spawn(move || {
            let shutdown = Arc::new(AtomicBool::new(false));
            for _ in 0..2 {
                let (stream, _) = listener.accept().expect("test client should connect");
                let runtime = Arc::clone(&accept_runtime);
                let event_log = accept_event_log.clone();
                let event_bus = accept_event_bus.clone();
                let shutdown = Arc::clone(&shutdown);
                thread::spawn(move || {
                    let _ = serve_unix_stream(stream, runtime, event_log, event_bus, &shutdown);
                });
            }
        });

        let mut messenger = TestClient::connect(&socket_path);
        let mut control = TestClient::connect(&socket_path);
        accept_thread
            .join()
            .expect("test accept thread should not panic");

        messenger.send(RequestEnvelope::new(
            RequestId::from("req_message"),
            ClientId::from("cli_message"),
            ClientRequest::MessageSend(MessageSendRequest {
                session_id: None,
                target_agent_id: None,
                content: "cancel active provider".to_owned(),
                content_kind: ContentKind::Chat,
            }),
        ));
        assert_accepted_response(&mut messenger);

        let session_id = match messenger
            .read_event_matching(|event| matches!(event.event, CadisEvent::SessionStarted(_)))
            .event
        {
            CadisEvent::SessionStarted(payload) => payload.session_id,
            other => panic!("expected session.started event, got {other:?}"),
        };
        messenger.read_event_matching(|event| {
            matches!(
                &event.event,
                CadisEvent::MessageDelta(payload) if payload.delta == "first"
            )
        });
        wait_for_flag(&first_delta);

        control.send(RequestEnvelope::new(
            RequestId::from("req_cancel"),
            ClientId::from("cli_control"),
            ClientRequest::SessionCancel(SessionTargetRequest {
                session_id: session_id.clone(),
            }),
        ));
        assert_accepted_response(&mut control);
        control.read_event_matching(|event| {
            matches!(
                &event.event,
                CadisEvent::AgentSessionCancelled(payload)
                    if payload.session_id == session_id
            )
        });

        set_flag(&release);
        wait_for_flag_timeout(&cancel_seen, Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[derive(Clone, Debug)]
    struct WaitingProvider {
        entered: Arc<(Mutex<bool>, Condvar)>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[derive(Clone, Debug)]
    struct CancellableProvider {
        first_delta: Arc<(Mutex<bool>, Condvar)>,
        release: Arc<(Mutex<bool>, Condvar)>,
        cancel_seen: Arc<(Mutex<bool>, Condvar)>,
    }

    #[cfg(unix)]
    impl ModelProvider for CancellableProvider {
        fn name(&self) -> &str {
            "cancellable"
        }

        fn chat(&self, _prompt: &str) -> Result<Vec<String>, ModelError> {
            Ok(vec!["done".to_owned()])
        }

        fn stream_chat(
            &self,
            request: ModelRequest<'_>,
            callback: &mut cadis_models::ModelStreamCallback<'_>,
        ) -> Result<ModelResponse, ModelError> {
            let invocation = ModelInvocation {
                requested_model: request.selected_model.map(ToOwned::to_owned),
                effective_provider: "cancellable".to_owned(),
                effective_model: "unit-test".to_owned(),
                fallback: false,
                fallback_reason: None,
            };
            callback(ModelStreamEvent::Started(invocation.clone()))?;
            callback(ModelStreamEvent::Delta("first".to_owned()))?;
            set_flag(&self.first_delta);
            wait_for_flag(&self.release);
            if callback(ModelStreamEvent::Delta("second".to_owned()))? == ModelStreamControl::Cancel
            {
                set_flag(&self.cancel_seen);
                return Err(ModelError::cancelled("model request was cancelled")
                    .with_invocation(invocation));
            }
            callback(ModelStreamEvent::Completed(invocation.clone()))?;
            Ok(ModelResponse {
                deltas: vec!["first".to_owned(), "second".to_owned()],
                invocation,
            })
        }
    }

    #[cfg(unix)]
    fn wait_for_flag(flag: &Arc<(Mutex<bool>, Condvar)>) {
        let (lock, condvar) = &**flag;
        let mut ready = lock.lock().expect("flag mutex should lock");
        while !*ready {
            ready = condvar
                .wait(ready)
                .expect("flag mutex should not be poisoned");
        }
    }

    #[cfg(unix)]
    fn set_flag(flag: &Arc<(Mutex<bool>, Condvar)>) {
        let (lock, condvar) = &**flag;
        *lock.lock().expect("flag mutex should lock") = true;
        condvar.notify_all();
    }

    #[cfg(unix)]
    fn wait_for_flag_timeout(flag: &Arc<(Mutex<bool>, Condvar)>, timeout: Duration) {
        let (lock, condvar) = &**flag;
        let deadline = Instant::now() + timeout;
        let mut ready = lock.lock().expect("flag mutex should lock");
        while !*ready {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for flag");
            let wait_for = deadline.saturating_duration_since(now);
            let (guard, result) = condvar
                .wait_timeout(ready, wait_for)
                .expect("flag mutex should not be poisoned");
            ready = guard;
            assert!(!result.timed_out() || *ready, "timed out waiting for flag");
        }
    }

    #[cfg(unix)]
    struct TestClient {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    #[cfg(unix)]
    impl TestClient {
        fn connect(socket_path: &Path) -> Self {
            let stream = UnixStream::connect(socket_path).expect("test client should connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("test client read timeout should be set");
            let reader_stream = stream
                .try_clone()
                .expect("test client stream should clone for reading");
            reader_stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("test client reader timeout should be set");
            Self {
                reader: BufReader::new(reader_stream),
                writer: stream,
            }
        }

        fn send(&mut self, request: RequestEnvelope) {
            serde_json::to_writer(&mut self.writer, &request)
                .expect("request frame should serialize");
            self.writer
                .write_all(b"\n")
                .expect("request frame newline should write");
            self.writer.flush().expect("request frame should flush");
        }

        fn read_frame(&mut self) -> ServerFrame {
            let mut line = String::new();
            self.reader
                .read_line(&mut line)
                .expect("server frame should be readable before timeout");
            assert!(!line.is_empty(), "server closed the test client stream");
            serde_json::from_str(&line).expect("server frame should deserialize")
        }

        fn read_event_matching(
            &mut self,
            mut predicate: impl FnMut(&EventEnvelope) -> bool,
        ) -> EventEnvelope {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let frame = self.read_frame();
                if let ServerFrame::Event(event) = frame {
                    if predicate(&event) {
                        return event;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for matching event"
                );
            }
        }
    }

    #[cfg(unix)]
    fn assert_accepted_response(client: &mut TestClient) {
        assert!(matches!(
            client.read_frame(),
            ServerFrame::Response(ResponseEnvelope {
                response: DaemonResponse::RequestAccepted(_),
                ..
            })
        ));
    }

    #[cfg(unix)]
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

    #[test]
    fn bounded_replay_returns_empty_for_zero_limit() {
        let replay = VecDeque::from(vec![event("evt_000001"), event("evt_000002")]);
        let result = bounded_replay(&replay, None, Some(0), 8, &EventFilter::All);
        assert!(result.is_empty());
    }

    #[test]
    fn bounded_replay_returns_all_when_no_since_id() {
        let replay = VecDeque::from(vec![
            event("evt_000001"),
            event("evt_000002"),
            event("evt_000003"),
        ]);
        let result = bounded_replay(&replay, None, Some(2), 8, &EventFilter::All);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].event_id.as_str(), "evt_000002");
        assert_eq!(result[1].event_id.as_str(), "evt_000003");
    }

    #[test]
    fn event_bus_removes_disconnected_subscribers() {
        let bus = EventBus::new(8);
        let (_replay, receiver) =
            bus.subscribe(EventBusSubscription::all(&EventSubscriptionRequest {
                include_snapshot: false,
                replay_limit: Some(8),
                since_event_id: None,
            }));
        drop(receiver);

        bus.publish(event("evt_000001"));

        let inner = bus.inner.lock().expect("bus mutex should lock");
        assert!(inner.subscribers.is_empty());
    }

    #[test]
    fn args_parse_check_flag() {
        let args = Args::parse(["--check"].map(String::from)).expect("should parse");
        assert!(args.check);
    }

    #[test]
    fn args_parse_version_flag() {
        let args = Args::parse(["--version"].map(String::from)).expect("should parse");
        assert!(args.version);
    }

    #[test]
    fn args_parse_dev_echo_flag() {
        let args = Args::parse(["--dev-echo"].map(String::from)).expect("should parse");
        assert!(args.dev_echo);
    }

    #[test]
    fn args_parse_socket_path() {
        let args =
            Args::parse(["--socket", "/tmp/test.sock"].map(String::from)).expect("should parse");
        assert_eq!(args.socket_path, Some(PathBuf::from("/tmp/test.sock")));
    }

    #[test]
    fn args_parse_tcp_port() {
        let args = Args::parse(["--tcp-port", "7433"].map(String::from)).expect("should parse");
        assert_eq!(args.tcp_port, Some(7433));
    }

    #[test]
    fn args_parse_unknown_flag_errors() {
        let result = Args::parse(["--unknown"].map(String::from));
        assert!(result.is_err());
    }
}

#[cfg(all(unix, test))]
fn serve_unix_stream(
    stream: std::os::unix::net::UnixStream,
    runtime: Arc<Mutex<Runtime>>,
    event_log: EventLog,
    event_bus: EventBus,
    shutdown: &AtomicBool,
) -> Result<(), Box<dyn Error>> {
    let reader = BufReader::new(stream.try_clone()?);
    let writer = BufWriter::new(stream);
    serve_lines_sync(reader, writer, runtime, event_log, event_bus, shutdown)
}

fn write_frame(writer: &mut impl Write, frame: &ServerFrame) -> Result<(), Box<dyn Error>> {
    serde_json::to_writer(&mut *writer, frame)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(unix)]
fn prepare_socket_path(socket_path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            set_private_permissions(parent)?;
        }
    }

    match fs::symlink_metadata(socket_path) {
        Ok(metadata) => {
            if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "daemon already appears to be running at {}",
                        socket_path.display()
                    ),
                )
                .into());
            }

            if metadata.file_type().is_socket() {
                fs::remove_file(socket_path)?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "socket path {} already exists and is not a Unix socket",
                        socket_path.display()
                    ),
                )
                .into());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
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
#[allow(dead_code)]
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
    if let Some(port) = args.tcp_port {
        config.tcp_port = Some(port);
    }
}

fn print_check(config: &CadisConfig, socket_path: Option<&Path>, tcp_port: Option<u16>) {
    println!("cadisd check: ok");
    println!("cadis_home: {}", config.cadis_home.display());
    println!("config: {}", config.config_path().display());
    if let Some(path) = socket_path {
        println!("socket: {}", path.display());
    }
    if let Some(port) = tcp_port {
        println!("tcp: 127.0.0.1:{port}");
    } else if cfg!(windows) {
        println!("tcp: {}", config.effective_tcp_address());
    }
    println!("model_provider: {}", config.model.provider);
    println!("ollama_model: {}", config.model.ollama_model);
    println!("ollama_endpoint: {}", config.model.ollama_endpoint);
    println!("openai_model: {}", config.model.openai_model);
    println!("openai_base_url: {}", redact(&config.model.openai_base_url));
    println!("voice_enabled: {}", config.voice.enabled);
    println!("voice_provider: {}", config.voice.provider);
    println!("voice_id: {}", config.voice.voice_id);
    println!("voice_stt_language: {}", config.voice.stt_language);
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Args {
    check: bool,
    version: bool,
    stdio: bool,
    dev_echo: bool,
    socket_path: Option<PathBuf>,
    tcp_port: Option<u16>,
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
                "--tcp-port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| invalid_input("--tcp-port requires a port number"))?;
                    parsed.tcp_port = Some(value.parse::<u16>().map_err(|e| {
                        invalid_input(format!("--tcp-port requires a valid port number: {e}"))
                    })?);
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
        "cadisd {}\n\nUSAGE:\n  cadisd [OPTIONS]\n\nOPTIONS:\n  --check                    Validate config and local state layout\n  --version, -V              Print version\n  --stdio                    Serve NDJSON protocol on stdin/stdout\n  --socket <PATH>            Unix socket path\n  --tcp-port <PORT>          Listen on TCP 127.0.0.1:<PORT> instead of Unix socket\n  --model-provider <NAME>    auto, codex-cli, openai, ollama, or echo\n  --ollama-model <NAME>      Ollama model name\n  --ollama-endpoint <URL>    Ollama endpoint\n  --dev-echo                 Force credential-free local fallback\n  --help, -h                 Print help",
        env!("CARGO_PKG_VERSION")
    );
}
