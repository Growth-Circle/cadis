mod connection;
mod theme;
mod types;
mod widgets;

use std::env;
use std::error::Error;
use std::process;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use cadis_protocol::{
    AgentId, AgentModelSetRequest, AgentRenameRequest, AgentStatus, CadisEvent, ClientRequest,
    ContentKind, DaemonResponse, EmptyPayload, ErrorPayload, EventEnvelope,
    EventSubscriptionRequest, MessageSendRequest, ServerFrame, UiPreferencesSetRequest,
    VoicePreferences, VoicePreviewRequest,
};
use eframe::egui::{
    self, Align, CentralPanel, Color32, Context, Frame, Layout, TopBottomPanel, Vec2,
};

use connection::{resolve_transport, spawn_request, spawn_subscription, Transport};
use theme::ThemeKey;

const MAX_MESSAGES: usize = 500;
use std::collections::VecDeque;
use types::{
    default_agents, AgentView, ApprovalView, ChatMessage, ConfigTab, DebugEvent, HudResult,
    ModelOption,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("cadis-hud: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = types::Args::parse(env::args().skip(1))?;
    if args.version {
        println!("cadis-hud {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let transport = resolve_transport(&args);
    let viewport = egui::ViewportBuilder::default()
        .with_title("CADIS HUD")
        .with_inner_size([1600.0, 1000.0])
        .with_min_inner_size([1200.0, 760.0])
        .with_decorations(false)
        .with_transparent(true);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "CADIS HUD",
        native_options,
        Box::new(move |_cc| Ok(Box::new(HudApp::new(transport)))),
    )?;

    Ok(())
}

pub(crate) struct HudApp {
    pub(crate) transport: Transport,
    pub(crate) tx: Sender<HudResult>,
    pub(crate) rx: Receiver<HudResult>,
    pub(crate) connected: bool,
    pub(crate) connection_label: String,
    pub(crate) last_status_request: Instant,
    pub(crate) daemon_status: Option<cadis_protocol::DaemonStatusPayload>,
    pub(crate) model_catalog: Vec<ModelOption>,
    pub(crate) agents: Vec<AgentView>,
    pub(crate) approvals: Vec<ApprovalView>,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) composer: String,
    pub(crate) pending_assistant: Option<usize>,
    pub(crate) config_open: bool,
    pub(crate) config_tab: ConfigTab,
    pub(crate) rename_target: Option<AgentId>,
    pub(crate) rename_value: String,
    pub(crate) theme: ThemeKey,
    pub(crate) opacity: u8,
    pub(crate) always_on_top: bool,
    pub(crate) selected_voice: String,
    pub(crate) voice_notice: String,
    pub(crate) events_subscribed: bool,
    pub(crate) scroll_to_bottom: bool,
    pub(crate) hud_started_daemon: bool,
    // Animation state
    pub(crate) animating: bool,
    pub(crate) orb_color_current: Color32,
    pub(crate) orb_color_target: Color32,
    pub(crate) orb_color_start_time: Instant,
    pub(crate) panel_opacity: f32,
    pub(crate) panel_opacity_target: f32,
    pub(crate) panel_opacity_start_time: Instant,
    // Chat UI
    pub(crate) typing_indicator: bool,
    // Draggable agent layouts
    pub(crate) agent_layouts: std::collections::HashMap<String, types::AgentLayout>,
    // Debug mode
    pub(crate) debug_enabled: bool,
    pub(crate) debug_events: VecDeque<DebugEvent>,
    pub(crate) event_count: u64,
    pub(crate) frame_times: VecDeque<Instant>,
}

impl HudApp {
    fn new(transport: Transport) -> Self {
        let (tx, rx) = mpsc::channel();
        let app = Self {
            transport,
            tx,
            rx,
            connected: false,
            connection_label: "connecting".to_owned(),
            last_status_request: Instant::now() - Duration::from_secs(10),
            daemon_status: None,
            model_catalog: Vec::new(),
            agents: default_agents(),
            approvals: Vec::new(),
            messages: vec![ChatMessage::system(
                "CADIS HUD ready. Connect to cadisd, then send a command.",
            )],
            composer: String::new(),
            pending_assistant: None,
            config_open: false,
            config_tab: ConfigTab::Models,
            rename_target: None,
            rename_value: String::new(),
            theme: ThemeKey::Arc,
            opacity: 90,
            always_on_top: false,
            selected_voice: "id-ID-GadisNeural".to_owned(),
            voice_notice: "voice preview is daemon-routed".to_owned(),
            events_subscribed: false,
            scroll_to_bottom: false,
            hud_started_daemon: env::var_os("CADIS_HUD_STARTED_DAEMON").is_some(),
            animating: true,
            orb_color_current: Color32::from_rgb(235, 180, 72),
            orb_color_target: Color32::from_rgb(235, 180, 72),
            orb_color_start_time: Instant::now(),
            panel_opacity: 1.0,
            panel_opacity_target: 1.0,
            panel_opacity_start_time: Instant::now(),
            typing_indicator: false,
            agent_layouts: std::collections::HashMap::new(),
            debug_enabled: false,
            debug_events: VecDeque::new(),
            event_count: 0,
            frame_times: VecDeque::new(),
        };

        app.request(ClientRequest::DaemonStatus(EmptyPayload::default()));
        app.request(ClientRequest::ModelsList(EmptyPayload::default()));
        app.request(ClientRequest::UiPreferencesGet(EmptyPayload::default()));
        app
    }

    pub(crate) fn request(&self, request: ClientRequest) {
        spawn_request(self.tx.clone(), self.transport.clone(), request);
    }

    fn drain_results(&mut self) {
        let prev_count = self.messages.len();
        while let Ok(result) = self.rx.try_recv() {
            match result.result {
                Ok(frames) => {
                    self.connected = true;
                    self.connection_label = "connected".to_owned();
                    for frame in frames {
                        self.apply_frame(frame);
                    }
                }
                Err(error) => {
                    self.connected = false;
                    self.connection_label = "disconnected".to_owned();
                    self.daemon_status = None;
                    self.events_subscribed = false;
                    self.messages
                        .push(ChatMessage::system(format!("cadisd unavailable: {error}")));
                }
            }
        }
        if self.messages.len() != prev_count {
            self.scroll_to_bottom = true;
        }
        // Cap message history (ring buffer: drop oldest when full)
        if self.messages.len() > MAX_MESSAGES {
            let excess = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(..excess);
            // Adjust pending_assistant index after drain
            self.pending_assistant = self.pending_assistant.and_then(|i| i.checked_sub(excess));
        }
        // Subscribe to events once connected
        if self.connected && !self.events_subscribed {
            self.events_subscribed = true;
            spawn_subscription(
                self.tx.clone(),
                self.transport.clone(),
                ClientRequest::EventsSubscribe(EventSubscriptionRequest {
                    replay_limit: Some(50),
                    ..Default::default()
                }),
            );
        }
    }

    fn apply_frame(&mut self, frame: ServerFrame) {
        match frame {
            ServerFrame::Response(response) => match response.response {
                DaemonResponse::DaemonStatus(status) => {
                    self.daemon_status = Some(status);
                }
                DaemonResponse::RequestRejected(error) => {
                    self.messages
                        .push(ChatMessage::system(format_error(&error)));
                }
                DaemonResponse::RequestAccepted(_) => {}
            },
            ServerFrame::Event(event) => self.apply_event(event),
        }
    }

    fn apply_event(&mut self, event: EventEnvelope) {
        self.event_count += 1;
        if self.debug_enabled {
            let label = format!("{:?}", std::mem::discriminant(&event.event));
            self.debug_events.push_back(DebugEvent {
                timestamp: Instant::now(),
                label,
                detail: String::new(),
            });
            if self.debug_events.len() > 20 {
                self.debug_events.pop_front();
            }
        }
        match event.event {
            CadisEvent::SessionStarted(payload) => {
                if let Some(title) = payload.title {
                    self.messages
                        .push(ChatMessage::system(format!("session started: {title}")));
                }
            }
            CadisEvent::MessageDelta(payload) => {
                self.typing_indicator = true;
                if self.pending_assistant.is_none() {
                    let name = payload
                        .agent_name
                        .clone()
                        .or_else(|| payload.agent_id.as_ref().map(|id| id.to_string()));
                    let msg = match name {
                        Some(n) => ChatMessage::assistant_named("", n),
                        None => ChatMessage::assistant(""),
                    };
                    self.messages.push(msg);
                    self.pending_assistant = Some(self.messages.len() - 1);
                }
                if let Some(index) = self.pending_assistant {
                    self.messages[index].text.push_str(&payload.delta);
                    if self.messages[index].agent_name.is_none() {
                        self.messages[index].agent_name = payload
                            .agent_name
                            .or_else(|| payload.agent_id.map(|id| id.to_string()));
                    }
                }
            }
            CadisEvent::MessageCompleted(_) => {
                self.typing_indicator = false;
                self.pending_assistant = None;
            }
            CadisEvent::SessionFailed(error) | CadisEvent::DaemonError(error) => {
                self.messages
                    .push(ChatMessage::system(format_error(&error)));
                self.pending_assistant = None;
            }
            CadisEvent::AgentStatusChanged(payload) => {
                let agent = self.agent_mut(payload.agent_id);
                agent.status = payload.status;
                agent.task = payload.task;
            }
            CadisEvent::AgentRenamed(payload) => {
                let agent = self.agent_mut(payload.agent_id);
                agent.name = payload.display_name;
            }
            CadisEvent::AgentModelChanged(payload) => {
                let agent = self.agent_mut(payload.agent_id);
                agent.model = payload.model;
            }
            CadisEvent::ModelsListResponse(payload) => {
                self.model_catalog = payload
                    .models
                    .into_iter()
                    .map(|model| ModelOption {
                        provider: model.provider,
                        model: model.model,
                        display_name: model.display_name,
                    })
                    .collect();
            }
            CadisEvent::UiPreferencesUpdated(payload) => {
                self.apply_preferences(payload.preferences);
            }
            CadisEvent::ApprovalRequested(payload) => {
                self.approvals.push(ApprovalView {
                    id: payload.approval_id,
                    risk: format!("{:?}", payload.risk_class),
                    title: payload.title,
                    summary: payload.summary,
                    command: payload.command.unwrap_or_default(),
                    workspace: payload.workspace.unwrap_or_default(),
                    waiting_resolution: false,
                });
            }
            CadisEvent::ApprovalResolved(payload) => {
                self.approvals
                    .retain(|approval| approval.id != payload.approval_id);
            }
            CadisEvent::VoicePreviewStarted(_) => {
                self.voice_notice = "voice preview started".to_owned();
            }
            CadisEvent::VoicePreviewCompleted(_) => {
                self.voice_notice = "voice preview completed".to_owned();
            }
            CadisEvent::VoicePreviewFailed(error) => {
                self.voice_notice = format_error(&error);
            }
            _ => {}
        }
    }

    fn apply_preferences(&mut self, preferences: serde_json::Value) {
        if let Some(theme) = preferences
            .pointer("/hud/theme")
            .and_then(serde_json::Value::as_str)
            .and_then(ThemeKey::from_key)
        {
            self.theme = theme;
        }
        if let Some(opacity) = preferences
            .pointer("/hud/background_opacity")
            .and_then(serde_json::Value::as_u64)
        {
            self.opacity = opacity.min(100) as u8;
        }
        if let Some(voice_id) = preferences
            .pointer("/voice/voice_id")
            .and_then(serde_json::Value::as_str)
        {
            self.selected_voice = voice_id.to_owned();
        }
    }

    fn agent_mut(&mut self, agent_id: AgentId) -> &mut AgentView {
        if let Some(index) = self
            .agents
            .iter()
            .position(|agent| agent.id.as_str() == agent_id.as_str())
        {
            return &mut self.agents[index];
        }

        self.agents.push(AgentView {
            id: agent_id.clone(),
            name: agent_id.to_string(),
            role: "agent".to_owned(),
            status: AgentStatus::Idle,
            task: None,
            model: "auto".to_owned(),
            workers: Vec::new(),
        });
        self.agents.last_mut().expect("agent was just pushed")
    }

    pub(crate) fn send_chat(&mut self) {
        let message = self.composer.trim().to_owned();
        if message.is_empty() || !self.connected {
            return;
        }
        self.messages.push(ChatMessage::user(message.clone()));
        self.composer.clear();
        self.pending_assistant = None;

        // Parse @mention targeting: "@agent_name rest of message"
        let (target_agent_id, content) = if message.starts_with('@') {
            if let Some(space) = message.find(|c: char| c.is_whitespace()) {
                let agent = &message[1..space];
                let rest = message[space..].trim_start().to_owned();
                if rest.is_empty() {
                    (None, message)
                } else {
                    (Some(AgentId::from(agent)), rest)
                }
            } else {
                (None, message)
            }
        } else {
            (None, message)
        };

        self.request(ClientRequest::MessageSend(MessageSendRequest {
            session_id: None,
            target_agent_id,
            content,
            content_kind: ContentKind::Chat,
        }));
    }

    pub(crate) fn clear_chat(&mut self) {
        self.messages.clear();
        self.pending_assistant = None;
        self.typing_indicator = false;
        self.messages.push(ChatMessage::system("Chat cleared."));
    }

    pub(crate) fn agent_layout_mut(&mut self, id: &str) -> &mut types::AgentLayout {
        let count = self.agents.len().max(1);
        let index = self
            .agents
            .iter()
            .position(|a| a.id.as_str() == id)
            .unwrap_or(0);
        self.agent_layouts.entry(id.to_owned()).or_insert_with(|| {
            let angle =
                index as f32 * std::f32::consts::TAU / count as f32 - std::f32::consts::FRAC_PI_2;
            types::AgentLayout {
                position: egui::Pos2::new(0.5 + 0.35 * angle.cos(), 0.5 + 0.35 * angle.sin()),
                visible: true,
            }
        })
    }

    pub(crate) fn send_rename(&mut self) {
        let Some(agent_id) = self.rename_target.take() else {
            return;
        };
        let display_name = normalize_agent_name(&self.rename_value, &agent_id);
        self.request(ClientRequest::AgentRename(AgentRenameRequest {
            agent_id,
            display_name,
        }));
        self.rename_value.clear();
    }

    pub(crate) fn set_theme(&mut self, theme: ThemeKey) {
        self.theme = theme;
        self.request(ClientRequest::UiPreferencesSet(UiPreferencesSetRequest {
            patch: serde_json::json!({ "hud": { "theme": theme.key() } }),
        }));
    }

    pub(crate) fn set_opacity(&mut self, opacity: u8) {
        self.opacity = opacity;
        self.request(ClientRequest::UiPreferencesSet(UiPreferencesSetRequest {
            patch: serde_json::json!({ "hud": { "background_opacity": opacity } }),
        }));
    }

    pub(crate) fn set_main_model(&mut self, model: String) {
        self.request(ClientRequest::AgentModelSet(AgentModelSetRequest {
            agent_id: AgentId::from("main"),
            model,
        }));
    }

    pub(crate) fn preview_voice(&mut self) {
        self.request(ClientRequest::VoicePreview(VoicePreviewRequest {
            text: "Halo, saya CADIS. Audio test berhasil.".to_owned(),
            prefs: Some(VoicePreferences {
                voice_id: self.selected_voice.clone(),
                rate: 0,
                pitch: 0,
                volume: 0,
            }),
        }));
    }
}

impl eframe::App for HudApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if self.debug_enabled {
            self.frame_times.push_back(Instant::now());
            if self.frame_times.len() > 60 {
                self.frame_times.pop_front();
            }
        }
        self.drain_results();
        if self.last_status_request.elapsed() >= Duration::from_secs(2) {
            self.last_status_request = Instant::now();
            self.request(ClientRequest::DaemonStatus(EmptyPayload::default()));
        }

        // Update orb color target based on daemon status
        let theme = self.theme.palette();
        let new_orb_target = if self.daemon_status.is_some() && self.connected {
            theme.ok
        } else if self.connected {
            theme.warn
        } else {
            theme.err
        };
        if new_orb_target != self.orb_color_target {
            self.orb_color_current = self.current_orb_color();
            self.orb_color_target = new_orb_target;
            self.orb_color_start_time = Instant::now();
        }

        // Determine if animating (orb always pulses, plus color transitions)
        let color_transitioning = self.orb_color_start_time.elapsed() < Duration::from_millis(300);
        let panel_transitioning =
            self.panel_opacity_start_time.elapsed() < Duration::from_millis(300);
        self.animating =
            color_transitioning || panel_transitioning || self.pending_assistant.is_some();

        // Lerp panel opacity for smooth show/hide
        let panel_t = (self.panel_opacity_start_time.elapsed().as_secs_f32() / 0.3).min(1.0);
        self.panel_opacity += (self.panel_opacity_target - self.panel_opacity) * panel_t;

        let opacity = ((self.opacity as f32 / 100.0) * 255.0).round() as u8;
        let background = Color32::from_rgba_premultiplied(6, 8, 10, opacity);

        TopBottomPanel::top("cadis_chrome")
            .frame(Frame::none().fill(background))
            .exact_height(32.0)
            .show(ctx, |ui| widgets::window_chrome(self, ctx, ui, &theme));

        CentralPanel::default()
            .frame(Frame::none().fill(background))
            .show(ctx, |ui| {
                widgets::status_bar(self, ui, &theme);
                ui.add_space(8.0);

                let chat_height = 280.0;
                let orbital_height = (ui.available_height() - chat_height - 12.0).max(360.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), orbital_height),
                    Layout::top_down(Align::Center),
                    |ui| widgets::orbital_hud(self, ctx, ui, &theme),
                );
                ui.add_space(4.0);
                widgets::agent_tray(self, ui, &theme);
                ui.add_space(4.0);
                widgets::chat_panel(self, ui, &theme, chat_height);
            });

        widgets::approval_stack(self, ctx, &theme);
        widgets::config_dialog(self, ctx, &theme);
        widgets::rename_dialog(self, ctx, &theme);

        // Adaptive framerate
        if self.animating {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.hud_started_daemon {
            let _ = connection::send_request(
                &self.transport,
                ClientRequest::DaemonShutdown(EmptyPayload::default()),
            );
        }
    }
}

impl HudApp {
    pub(crate) fn current_orb_color(&self) -> Color32 {
        let elapsed = self.orb_color_start_time.elapsed().as_secs_f32();
        let t = (elapsed / 0.3).min(1.0);
        theme::lerp_color(self.orb_color_current, self.orb_color_target, t)
    }
}

fn format_error(error: &ErrorPayload) -> String {
    format!("{}: {}", error.code, error.message)
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
