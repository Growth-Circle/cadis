use std::env;
use std::error::Error;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cadis_protocol::{
    AgentId, AgentModelSetRequest, AgentRenameRequest, AgentStatus, ApprovalDecision, ApprovalId,
    ApprovalResponseRequest, CadisEvent, ClientId, ClientRequest, ContentKind, DaemonResponse,
    EmptyPayload, ErrorPayload, EventEnvelope, MessageSendRequest, RequestEnvelope, RequestId,
    ServerFrame, UiPreferencesSetRequest, VoicePreferences, VoicePreviewRequest,
};
use cadis_store::load_config;
use eframe::egui::{
    self, Align, Align2, Area, Button, CentralPanel, Color32, ComboBox, Context, FontId, Frame, Id,
    Key, Layout, Margin, Order, Pos2, Rect, RichText, Rounding, ScrollArea, Sense, Slider, Stroke,
    TextEdit, TopBottomPanel, Ui, Vec2, ViewportCommand, Window,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("cadis-hud: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;
    if args.version {
        println!("cadis-hud {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let socket_path = args
        .socket_path
        .or_else(|| env::var_os("CADIS_HUD_SOCKET").map(PathBuf::from))
        .unwrap_or(load_config()?.effective_socket_path());
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
        Box::new(move |_cc| Ok(Box::new(HudApp::new(socket_path)))),
    )?;

    Ok(())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Args {
    socket_path: Option<PathBuf>,
    version: bool,
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
                "--socket" => {
                    parsed.socket_path = Some(PathBuf::from(
                        args.next()
                            .ok_or_else(|| invalid_input("--socket requires a path"))?,
                    ));
                }
                "--version" | "-V" => parsed.version = true,
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

fn print_help() {
    println!(
        "cadis-hud {}\n\nUSAGE:\n  cadis-hud [--socket PATH]\n\nOPTIONS:\n  --socket <PATH>   Unix socket path for cadisd\n  --version, -V     Print version\n  --help, -h        Print help",
        env!("CARGO_PKG_VERSION")
    );
}

struct HudApp {
    socket_path: PathBuf,
    tx: Sender<HudResult>,
    rx: Receiver<HudResult>,
    connected: bool,
    connection_label: String,
    last_status_request: Instant,
    daemon_status: Option<cadis_protocol::DaemonStatusPayload>,
    model_catalog: Vec<ModelOption>,
    agents: Vec<AgentView>,
    approvals: Vec<ApprovalView>,
    messages: Vec<ChatMessage>,
    composer: String,
    pending_assistant: Option<usize>,
    config_open: bool,
    config_tab: ConfigTab,
    rename_target: Option<AgentId>,
    rename_value: String,
    theme: ThemeKey,
    opacity: u8,
    always_on_top: bool,
    selected_voice: String,
    voice_notice: String,
}

impl HudApp {
    fn new(socket_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let app = Self {
            socket_path,
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
        };

        app.request(ClientRequest::DaemonStatus(EmptyPayload::default()));
        app.request(ClientRequest::ModelsList(EmptyPayload::default()));
        app.request(ClientRequest::UiPreferencesGet(EmptyPayload::default()));
        app
    }

    fn request(&self, request: ClientRequest) {
        let tx = self.tx.clone();
        let socket_path = self.socket_path.clone();
        thread::spawn(move || {
            let result = send_request(socket_path, request).map_err(|error| error.to_string());
            let _ = tx.send(HudResult { result });
        });
    }

    fn drain_results(&mut self) {
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
                    self.messages
                        .push(ChatMessage::system(format!("cadisd unavailable: {error}")));
                }
            }
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
        match event.event {
            CadisEvent::SessionStarted(payload) => {
                if let Some(title) = payload.title {
                    self.messages
                        .push(ChatMessage::system(format!("session started: {title}")));
                }
            }
            CadisEvent::MessageDelta(payload) => {
                if self.pending_assistant.is_none() {
                    self.messages.push(ChatMessage::assistant(""));
                    self.pending_assistant = Some(self.messages.len() - 1);
                }
                if let Some(index) = self.pending_assistant {
                    self.messages[index].text.push_str(&payload.delta);
                }
            }
            CadisEvent::MessageCompleted(_) => {
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

    fn send_chat(&mut self) {
        let message = self.composer.trim().to_owned();
        if message.is_empty() || !self.connected {
            return;
        }
        self.messages.push(ChatMessage::user(message.clone()));
        self.composer.clear();
        self.pending_assistant = None;
        self.request(ClientRequest::MessageSend(MessageSendRequest {
            session_id: None,
            target_agent_id: None,
            content: message,
            content_kind: ContentKind::Chat,
        }));
    }

    fn send_rename(&mut self) {
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

    fn set_theme(&mut self, theme: ThemeKey) {
        self.theme = theme;
        self.request(ClientRequest::UiPreferencesSet(UiPreferencesSetRequest {
            patch: serde_json::json!({ "hud": { "theme": theme.key() } }),
        }));
    }

    fn set_opacity(&mut self, opacity: u8) {
        self.opacity = opacity;
        self.request(ClientRequest::UiPreferencesSet(UiPreferencesSetRequest {
            patch: serde_json::json!({ "hud": { "background_opacity": opacity } }),
        }));
    }

    fn set_main_model(&mut self, model: String) {
        self.request(ClientRequest::AgentModelSet(AgentModelSetRequest {
            agent_id: AgentId::from("main"),
            model,
        }));
    }

    fn preview_voice(&mut self) {
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
        self.drain_results();
        if self.last_status_request.elapsed() >= Duration::from_secs(2) {
            self.last_status_request = Instant::now();
            self.request(ClientRequest::DaemonStatus(EmptyPayload::default()));
        }

        let theme = self.theme.palette();
        let opacity = ((self.opacity as f32 / 100.0) * 255.0).round() as u8;
        let background = Color32::from_rgba_premultiplied(6, 8, 10, opacity);

        TopBottomPanel::top("cadis_chrome")
            .frame(Frame::none().fill(background))
            .exact_height(32.0)
            .show(ctx, |ui| self.window_chrome(ctx, ui, &theme));

        CentralPanel::default()
            .frame(Frame::none().fill(background))
            .show(ctx, |ui| {
                self.status_bar(ui, &theme);
                ui.add_space(8.0);

                let chat_height = 280.0;
                let orbital_height = (ui.available_height() - chat_height - 12.0).max(360.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), orbital_height),
                    Layout::top_down(Align::Center),
                    |ui| self.orbital_hud(ui, &theme),
                );
                ui.add_space(8.0);
                self.chat_panel(ui, &theme, chat_height);
            });

        self.approval_stack(ctx, &theme);
        self.config_dialog(ctx, &theme);
        self.rename_dialog(ctx, &theme);
        ctx.request_repaint_after(Duration::from_millis(60));
    }
}

impl HudApp {
    fn window_chrome(&mut self, ctx: &Context, ui: &mut Ui, theme: &Palette) {
        ui.horizontal(|ui| {
            let title_response = ui.add_sized(
                [ui.available_width() - 190.0, 26.0],
                egui::Label::new(
                    RichText::new("CADIS HUD")
                        .monospace()
                        .size(14.0)
                        .color(theme.text),
                )
                .sense(Sense::drag()),
            );
            if title_response.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }

            if ui
                .add(Button::new(RichText::new("CFG").monospace()).fill(theme.panel2))
                .on_hover_text("Open configuration")
                .clicked()
            {
                self.config_open = true;
            }
            if ui
                .add(Button::new(RichText::new("PIN").monospace()).fill(theme.panel2))
                .on_hover_text("Toggle always on top preference")
                .clicked()
            {
                self.always_on_top = !self.always_on_top;
            }
            if ui
                .add(Button::new(RichText::new("_").monospace()).fill(theme.panel2))
                .on_hover_text("Minimize")
                .clicked()
            {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }
            if ui
                .add(Button::new(RichText::new("X").monospace()).fill(theme.err))
                .on_hover_text("Close")
                .clicked()
            {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui, theme: &Palette) {
        let main = self
            .agents
            .iter()
            .find(|agent| agent.id.as_str() == "main")
            .cloned()
            .unwrap_or_else(default_main_agent);
        let active = self
            .agents
            .iter()
            .filter(|agent| agent.status == AgentStatus::Running)
            .count();
        let waiting = self
            .agents
            .iter()
            .filter(|agent| agent.status == AgentStatus::WaitingApproval)
            .count();
        let idle = self.agents.len().saturating_sub(active + waiting);
        let socket = self.socket_path.display().to_string();

        Frame::none()
            .fill(theme.panel)
            .stroke(Stroke::new(1.0, theme.border))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::same(10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(main.name)
                            .monospace()
                            .strong()
                            .size(16.0)
                            .color(theme.accent),
                    );
                    ui.separator();
                    ui.colored_label(
                        if self.connected { theme.ok } else { theme.err },
                        format!("daemon {}", self.connection_label),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("model {}", main.model))
                            .monospace()
                            .color(theme.text),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("active {active} / waiting {waiting} / idle {idle}"))
                            .monospace()
                            .color(theme.dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(socket).monospace().small().color(theme.faint));
                    });
                });
            });
    }

    fn orbital_hud(&mut self, ui: &mut Ui, theme: &Palette) {
        let desired = Vec2::new(ui.available_width(), ui.available_height());
        let (rect, _response) = ui.allocate_exact_size(desired, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, Rounding::same(8.0), theme.panel);
        painter.rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0, theme.border));

        draw_grid(&painter, rect, theme);

        let center = rect.center();
        let radius = rect.height().min(rect.width()) * 0.13;
        painter.circle_stroke(center, radius * 1.8, Stroke::new(1.0, theme.border));
        painter.circle_stroke(center, radius * 2.45, Stroke::new(1.0, theme.border_dark));

        let pulse = (ui.input(|input| input.time).sin() as f32 + 1.0) * 0.5;
        painter.circle_filled(
            center,
            radius,
            Color32::from_rgba_premultiplied(
                theme.accent.r(),
                theme.accent.g(),
                theme.accent.b(),
                (45.0 + pulse * 35.0) as u8,
            ),
        );
        painter.circle_stroke(center, radius, Stroke::new(2.0, theme.accent));

        let main_name = self
            .agents
            .iter()
            .find(|agent| agent.id.as_str() == "main")
            .map(|agent| agent.name.as_str())
            .unwrap_or("CADIS");
        painter.text(
            center,
            Align2::CENTER_CENTER,
            main_name,
            FontId::monospace(30.0),
            theme.text,
        );
        painter.text(
            center + Vec2::new(0.0, radius * 0.52),
            Align2::CENTER_CENTER,
            if self.connected { "ONLINE" } else { "OFFLINE" },
            FontId::monospace(13.0),
            if self.connected { theme.ok } else { theme.err },
        );

        let orb_rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
        let orb_response = ui.interact(orb_rect, Id::new("central_orb"), Sense::click());
        orb_response.context_menu(|ui| {
            if ui.button("Rename main agent").clicked() {
                self.rename_target = Some(AgentId::from("main"));
                self.rename_value = main_name.to_owned();
                ui.close_menu();
            }
        });

        let slots = slot_positions(rect);
        for (index, position) in slots.into_iter().enumerate() {
            painter.line_segment(
                [center, position],
                Stroke::new(1.0, Color32::from_rgba_premultiplied(120, 180, 220, 55)),
            );
            let agent = self
                .agents
                .get(index)
                .cloned()
                .unwrap_or_else(|| placeholder_agent(index));
            self.agent_card(ui, &painter, theme, position, agent);
        }
    }

    fn agent_card(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        theme: &Palette,
        center: Pos2,
        agent: AgentView,
    ) {
        let card = Rect::from_center_size(center, Vec2::new(210.0, 86.0));
        painter.rect_filled(card, Rounding::same(8.0), theme.panel2);
        painter.rect_stroke(card, Rounding::same(8.0), Stroke::new(1.0, theme.border));

        let status_color = status_color(agent.status, theme);
        painter.circle_filled(card.left_top() + Vec2::new(18.0, 18.0), 5.0, status_color);
        painter.text(
            card.left_top() + Vec2::new(31.0, 10.0),
            Align2::LEFT_TOP,
            truncate(&agent.name, 22),
            FontId::monospace(13.5),
            theme.text,
        );
        painter.text(
            card.left_top() + Vec2::new(14.0, 35.0),
            Align2::LEFT_TOP,
            format!("{} / {:?}", agent.role, agent.status).to_lowercase(),
            FontId::monospace(11.0),
            theme.dim,
        );
        let detail = agent.task.as_deref().unwrap_or(&agent.model);
        painter.text(
            card.left_top() + Vec2::new(14.0, 57.0),
            Align2::LEFT_TOP,
            truncate(detail, 30),
            FontId::monospace(10.5),
            theme.faint,
        );

        if !agent.workers.is_empty() {
            painter.text(
                card.right_bottom() - Vec2::new(12.0, 18.0),
                Align2::RIGHT_BOTTOM,
                format!("{} workers", agent.workers.len()),
                FontId::monospace(10.5),
                theme.accent,
            );
        }

        let response = ui.interact(card, Id::new(("agent", agent.id.as_str())), Sense::click());
        response.context_menu(|ui| {
            if ui.button("Rename agent").clicked() {
                self.rename_target = Some(agent.id.clone());
                self.rename_value = agent.name.clone();
                ui.close_menu();
            }
            if ui.button("Use first listed model").clicked() {
                if let Some(model) = self.model_catalog.first() {
                    self.set_main_model(model.model.clone());
                }
                ui.close_menu();
            }
        });
    }

    fn chat_panel(&mut self, ui: &mut Ui, theme: &Palette, height: f32) {
        Frame::none()
            .fill(theme.panel)
            .stroke(Stroke::new(1.0, theme.border))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::same(10.0))
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("COMMAND CHANNEL")
                            .monospace()
                            .strong()
                            .color(theme.accent),
                    );
                    ui.separator();
                    for chip in ["yes", "no", "cancel", "expand"] {
                        if ui
                            .add(Button::new(RichText::new(chip).monospace()).fill(theme.panel2))
                            .clicked()
                        {
                            self.composer = chip.to_owned();
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("models").clicked() {
                            self.config_open = true;
                            self.config_tab = ConfigTab::Models;
                        }
                        if ui.button("voice").clicked() {
                            self.config_open = true;
                            self.config_tab = ConfigTab::Voice;
                        }
                    });
                });
                ui.separator();

                ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(height - 92.0)
                    .show(ui, |ui| {
                        for message in &self.messages {
                            let color = match message.role {
                                ChatRole::User => theme.accent,
                                ChatRole::Assistant => theme.text,
                                ChatRole::System => theme.dim,
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{}  {}",
                                    message.role.label(),
                                    message.text
                                ))
                                .color(color)
                                .monospace(),
                            );
                        }
                    });

                ui.horizontal(|ui| {
                    let input = ui.add_sized(
                        [ui.available_width() - 92.0, 46.0],
                        TextEdit::multiline(&mut self.composer)
                            .hint_text(if self.connected {
                                "type a CADIS command"
                            } else {
                                "cadisd is disconnected"
                            })
                            .desired_rows(2),
                    );
                    let enter_send = input.has_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                    if ui
                        .add_enabled(
                            self.connected,
                            Button::new(RichText::new("SEND").monospace().strong())
                                .fill(theme.accent),
                        )
                        .clicked()
                        || enter_send
                    {
                        self.send_chat();
                    }
                });
            });
    }

    fn approval_stack(&mut self, ctx: &Context, theme: &Palette) {
        if self.approvals.is_empty() {
            return;
        }

        Area::new(Id::new("approval_stack"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_TOP, [-22.0, 84.0])
            .show(ctx, |ui| {
                ui.set_width(360.0);
                for index in 0..self.approvals.len() {
                    let mut decision = None;
                    let approval_id = self.approvals[index].id.clone();
                    let approval = &mut self.approvals[index];
                    Frame::none()
                        .fill(theme.panel2)
                        .stroke(Stroke::new(1.0, theme.warn))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(&approval.title).monospace().color(theme.warn));
                            ui.label(RichText::new(&approval.risk).monospace().small());
                            ui.label(&approval.summary);
                            if !approval.command.is_empty() {
                                ui.label(RichText::new(&approval.command).monospace().small());
                            }
                            if !approval.workspace.is_empty() {
                                ui.label(RichText::new(&approval.workspace).monospace().small());
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        !approval.waiting_resolution,
                                        Button::new("DENY").fill(theme.err),
                                    )
                                    .clicked()
                                {
                                    approval.waiting_resolution = true;
                                    decision = Some(ApprovalDecision::Denied);
                                }
                                if ui
                                    .add_enabled(
                                        !approval.waiting_resolution,
                                        Button::new("APPROVE").fill(theme.warn),
                                    )
                                    .clicked()
                                {
                                    approval.waiting_resolution = true;
                                    decision = Some(ApprovalDecision::Approved);
                                }
                            });
                        });
                    if let Some(decision) = decision {
                        self.request(ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                            approval_id,
                            decision,
                            reason: Some("resolved from CADIS HUD".to_owned()),
                        }));
                    }
                    ui.add_space(8.0);
                }
            });
    }

    fn config_dialog(&mut self, ctx: &Context, theme: &Palette) {
        if !self.config_open {
            return;
        }

        let mut open = self.config_open;
        Window::new("CADIS CONFIG")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(620.0)
            .frame(
                Frame::window(&ctx.style())
                    .fill(theme.panel)
                    .stroke(Stroke::new(1.0, theme.border))
                    .rounding(Rounding::same(8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for tab in ConfigTab::all() {
                        if ui
                            .selectable_label(self.config_tab == tab, tab.label())
                            .clicked()
                        {
                            self.config_tab = tab;
                        }
                    }
                });
                ui.separator();
                match self.config_tab {
                    ConfigTab::Voice => self.voice_tab(ui, theme),
                    ConfigTab::Models => self.models_tab(ui, theme),
                    ConfigTab::Appearance => self.appearance_tab(ui, theme),
                    ConfigTab::Window => self.window_tab(ui, theme),
                }
            });
        self.config_open = open;
    }

    fn voice_tab(&mut self, ui: &mut Ui, theme: &Palette) {
        ui.label(
            RichText::new("voice provider route")
                .monospace()
                .color(theme.dim),
        );
        ComboBox::from_label("voice")
            .selected_text(&self.selected_voice)
            .show_ui(ui, |ui| {
                for voice in VOICES {
                    ui.selectable_value(&mut self.selected_voice, (*voice).to_owned(), *voice);
                }
            });
        ui.horizontal(|ui| {
            if ui.button("TEST VOICE").clicked() {
                self.preview_voice();
            }
            if ui.button("STOP").clicked() {
                self.request(ClientRequest::VoiceStop(EmptyPayload::default()));
            }
        });
        ui.label(
            RichText::new(&self.voice_notice)
                .monospace()
                .color(theme.faint),
        );
    }

    fn models_tab(&mut self, ui: &mut Ui, theme: &Palette) {
        if ui.button("REFRESH MODELS").clicked() {
            self.request(ClientRequest::ModelsList(EmptyPayload::default()));
        }
        ui.separator();
        let current = self
            .agents
            .iter()
            .find(|agent| agent.id.as_str() == "main")
            .map(|agent| agent.model.clone())
            .unwrap_or_else(|| "auto".to_owned());
        let mut selected = current.clone();
        ComboBox::from_label("main agent model")
            .selected_text(&selected)
            .show_ui(ui, |ui| {
                for model in &self.model_catalog {
                    ui.selectable_value(
                        &mut selected,
                        model.model.clone(),
                        format!(
                            "{}/{} - {}",
                            model.provider, model.model, model.display_name
                        ),
                    );
                }
            });
        if selected != current {
            self.set_main_model(selected);
        }
        ui.label(
            RichText::new("model changes are confirmed by agent.model.changed")
                .monospace()
                .small()
                .color(theme.faint),
        );
    }

    fn appearance_tab(&mut self, ui: &mut Ui, theme: &Palette) {
        ui.label(RichText::new("theme").monospace().color(theme.dim));
        ui.horizontal_wrapped(|ui| {
            for key in ThemeKey::all() {
                let palette = key.palette();
                let button = Button::new(key.label())
                    .fill(palette.accent)
                    .stroke(Stroke::new(
                        if self.theme == key { 2.0 } else { 1.0 },
                        theme.text,
                    ));
                if ui.add(button).clicked() {
                    self.set_theme(key);
                }
            }
        });
        let mut opacity = self.opacity as f32;
        if ui
            .add(Slider::new(&mut opacity, 45.0..=100.0).text("background opacity"))
            .changed()
        {
            self.set_opacity(opacity.round() as u8);
        }
    }

    fn window_tab(&mut self, ui: &mut Ui, theme: &Palette) {
        ui.checkbox(&mut self.always_on_top, "always on top preference");
        ui.label(
            RichText::new("window preference is UI-local in this prototype")
                .monospace()
                .small()
                .color(theme.faint),
        );
        if ui.button("RECHECK DAEMON").clicked() {
            self.request(ClientRequest::DaemonStatus(EmptyPayload::default()));
        }
    }

    fn rename_dialog(&mut self, ctx: &Context, theme: &Palette) {
        if self.rename_target.is_none() {
            return;
        }

        let target = self
            .rename_target
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        Window::new(format!("RENAME {target}"))
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .frame(
                Frame::window(&ctx.style())
                    .fill(theme.panel)
                    .stroke(Stroke::new(1.0, theme.border))
                    .rounding(Rounding::same(8.0)),
            )
            .show(ctx, |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.rename_value).hint_text("agent display name"),
                );
                ui.horizontal(|ui| {
                    if ui.button("CANCEL").clicked() {
                        self.rename_target = None;
                        self.rename_value.clear();
                    }
                    if ui
                        .add_enabled(self.connected, Button::new("SAVE").fill(theme.accent))
                        .clicked()
                    {
                        self.send_rename();
                    }
                });
                if !self.connected {
                    ui.label(RichText::new("cadisd disconnected").color(theme.err));
                }
            });
    }
}

fn send_request(
    socket_path: PathBuf,
    request: ClientRequest,
) -> Result<Vec<ServerFrame>, Box<dyn Error>> {
    let mut stream = UnixStream::connect(&socket_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not connect to cadisd at {}: {error}",
                socket_path.display()
            ),
        )
    })?;
    let envelope = RequestEnvelope::new(next_request_id(), ClientId::from("hud_main"), request);
    serde_json::to_writer(&mut stream, &envelope)?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let reader = BufReader::new(stream);
    let mut frames = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if !line.trim().is_empty() {
            frames.push(serde_json::from_str::<ServerFrame>(&line)?);
        }
    }
    Ok(frames)
}

fn next_request_id() -> RequestId {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    RequestId::from(format!("req_hud_{}_{}", process::id(), millis))
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn format_error(error: &ErrorPayload) -> String {
    format!("{}: {}", error.code, error.message)
}

fn draw_grid(painter: &egui::Painter, rect: Rect, theme: &Palette) {
    let spacing = 42.0;
    let mut x = rect.left();
    while x <= rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(
                1.0,
                Color32::from_rgba_premultiplied(
                    theme.faint.r(),
                    theme.faint.g(),
                    theme.faint.b(),
                    22,
                ),
            ),
        );
        x += spacing;
    }
    let mut y = rect.top();
    while y <= rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(
                1.0,
                Color32::from_rgba_premultiplied(
                    theme.faint.r(),
                    theme.faint.g(),
                    theme.faint.b(),
                    18,
                ),
            ),
        );
        y += spacing;
    }
}

fn slot_positions(rect: Rect) -> [Pos2; 12] {
    let w = rect.width();
    let h = rect.height();
    let p = |x: f32, y: f32| Pos2::new(rect.left() + w * x, rect.top() + h * y);
    [
        p(0.18, 0.13),
        p(0.39, 0.10),
        p(0.61, 0.10),
        p(0.82, 0.13),
        p(0.09, 0.36),
        p(0.91, 0.36),
        p(0.09, 0.62),
        p(0.91, 0.62),
        p(0.18, 0.86),
        p(0.39, 0.90),
        p(0.61, 0.90),
        p(0.82, 0.86),
    ]
}

fn status_color(status: AgentStatus, theme: &Palette) -> Color32 {
    match status {
        AgentStatus::Spawning => theme.warn,
        AgentStatus::Idle => theme.dim,
        AgentStatus::Running => theme.ok,
        AgentStatus::WaitingApproval => theme.warn,
        AgentStatus::Completed => theme.accent,
        AgentStatus::Failed => theme.err,
        AgentStatus::Cancelled => theme.dim,
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut result = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        result.push('…');
    }
    result
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

#[derive(Clone, Debug)]
struct HudResult {
    result: Result<Vec<ServerFrame>, String>,
}

#[derive(Clone, Debug)]
struct AgentView {
    id: AgentId,
    name: String,
    role: String,
    status: AgentStatus,
    task: Option<String>,
    model: String,
    workers: Vec<String>,
}

fn default_agents() -> Vec<AgentView> {
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

fn default_main_agent() -> AgentView {
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

fn placeholder_agent(index: usize) -> AgentView {
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
struct ApprovalView {
    id: ApprovalId,
    risk: String,
    title: String,
    summary: String,
    command: String,
    workspace: String,
    waiting_resolution: bool,
}

#[derive(Clone, Debug)]
struct ModelOption {
    provider: String,
    model: String,
    display_name: String,
}

#[derive(Clone, Debug)]
struct ChatMessage {
    role: ChatRole,
    text: String,
}

impl ChatMessage {
    fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            text: text.into(),
        }
    }

    fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            text: text.into(),
        }
    }

    fn system(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            text: text.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChatRole {
    User,
    Assistant,
    System,
}

impl ChatRole {
    fn label(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Assistant => "CADIS",
            Self::System => "SYS",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigTab {
    Voice,
    Models,
    Appearance,
    Window,
}

impl ConfigTab {
    fn all() -> [Self; 4] {
        [Self::Voice, Self::Models, Self::Appearance, Self::Window]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Voice => "Voice",
            Self::Models => "Models",
            Self::Appearance => "Appearance",
            Self::Window => "Window",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThemeKey {
    Arc,
    Amber,
    Phosphor,
    Violet,
    Alert,
    Ice,
}

impl ThemeKey {
    fn all() -> [Self; 6] {
        [
            Self::Arc,
            Self::Amber,
            Self::Phosphor,
            Self::Violet,
            Self::Alert,
            Self::Ice,
        ]
    }

    fn from_key(value: &str) -> Option<Self> {
        Some(match value {
            "arc" => Self::Arc,
            "amber" => Self::Amber,
            "phosphor" => Self::Phosphor,
            "violet" => Self::Violet,
            "alert" => Self::Alert,
            "ice" => Self::Ice,
            _ => return None,
        })
    }

    fn key(self) -> &'static str {
        match self {
            Self::Arc => "arc",
            Self::Amber => "amber",
            Self::Phosphor => "phosphor",
            Self::Violet => "violet",
            Self::Alert => "alert",
            Self::Ice => "ice",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Arc => "ARC",
            Self::Amber => "AMBER",
            Self::Phosphor => "PHOSPHOR",
            Self::Violet => "VIOLET",
            Self::Alert => "ALERT",
            Self::Ice => "ICE",
        }
    }

    fn palette(self) -> Palette {
        match self {
            Self::Arc => Palette::new((20, 28, 36), (65, 185, 240)),
            Self::Amber => Palette::new((34, 26, 16), (238, 174, 64)),
            Self::Phosphor => Palette::new((14, 28, 20), (91, 220, 130)),
            Self::Violet => Palette::new((27, 20, 36), (190, 118, 240)),
            Self::Alert => Palette::new((38, 18, 16), (245, 96, 68)),
            Self::Ice => Palette::new((16, 25, 42), (145, 176, 255)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Palette {
    panel: Color32,
    panel2: Color32,
    border: Color32,
    border_dark: Color32,
    text: Color32,
    dim: Color32,
    faint: Color32,
    accent: Color32,
    ok: Color32,
    warn: Color32,
    err: Color32,
}

impl Palette {
    fn new(bg: (u8, u8, u8), accent: (u8, u8, u8)) -> Self {
        Self {
            panel: Color32::from_rgba_premultiplied(bg.0, bg.1, bg.2, 186),
            panel2: Color32::from_rgba_premultiplied(
                bg.0.saturating_add(12),
                bg.1.saturating_add(12),
                bg.2.saturating_add(12),
                212,
            ),
            border: Color32::from_rgba_premultiplied(accent.0, accent.1, accent.2, 120),
            border_dark: Color32::from_rgba_premultiplied(accent.0, accent.1, accent.2, 64),
            text: Color32::from_rgb(224, 232, 235),
            dim: Color32::from_rgb(148, 166, 174),
            faint: Color32::from_rgb(96, 116, 126),
            accent: Color32::from_rgb(accent.0, accent.1, accent.2),
            ok: Color32::from_rgb(84, 218, 130),
            warn: Color32::from_rgb(235, 180, 72),
            err: Color32::from_rgb(238, 80, 74),
        }
    }
}

const VOICES: &[&str] = &[
    "id-ID-ArdiNeural",
    "id-ID-GadisNeural",
    "ms-MY-OsmanNeural",
    "ms-MY-YasminNeural",
    "en-US-AvaNeural",
    "en-US-AndrewNeural",
    "en-US-EmmaNeural",
    "en-US-BrianNeural",
    "en-GB-SoniaNeural",
    "en-GB-RyanNeural",
];
