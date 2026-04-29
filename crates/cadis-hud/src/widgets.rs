use cadis_protocol::{
    AgentId, ApprovalDecision, ApprovalResponseRequest, ClientRequest, EmptyPayload,
};
use eframe::egui::{
    self, Align, Align2, Area, Button, Color32, ComboBox, Context, FontId, Id, Key, Layout, Order,
    Pos2, Rect, RichText, Rounding, ScrollArea, Sense, Slider, Stroke, TextEdit, Ui, Vec2,
    ViewportCommand, Window,
};

use crate::theme::{draw_grid, glass_frame, status_color, Palette, ThemeKey};
use crate::types::{placeholder_agent, ChatRole, ConfigTab};
use crate::HudApp;

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

pub(crate) fn window_chrome(app: &mut HudApp, ctx: &Context, ui: &mut Ui, theme: &Palette) {
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
            app.config_open = true;
        }
        if ui
            .add(Button::new(RichText::new("PIN").monospace()).fill(theme.panel2))
            .on_hover_text("Toggle always on top preference")
            .clicked()
        {
            app.always_on_top = !app.always_on_top;
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

pub(crate) fn status_bar(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    let main = app
        .agents
        .iter()
        .find(|agent| agent.id.as_str() == "main")
        .cloned()
        .unwrap_or_else(crate::types::default_main_agent);
    let active = app
        .agents
        .iter()
        .filter(|agent| agent.status == cadis_protocol::AgentStatus::Running)
        .count();
    let waiting = app
        .agents
        .iter()
        .filter(|agent| agent.status == cadis_protocol::AgentStatus::WaitingApproval)
        .count();
    let idle = app.agents.len().saturating_sub(active + waiting);
    let socket = app.transport.to_string();

    glass_frame(theme).show(ui, |ui| {
        ui.horizontal(|ui| {
            // Connection status dot
            let dot_color = if app.daemon_status.is_some() && app.connected {
                theme.ok
            } else if app.connected {
                theme.warn
            } else {
                theme.err
            };
            let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter()
                .circle_filled(dot_rect.center(), 5.0, dot_color);

            ui.label(
                RichText::new(main.name)
                    .monospace()
                    .strong()
                    .size(16.0)
                    .color(theme.accent),
            );
            ui.separator();
            ui.colored_label(
                if app.connected { theme.ok } else { theme.err },
                format!("daemon {}", app.connection_label),
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

pub(crate) fn orbital_hud(app: &mut HudApp, _ctx: &Context, ui: &mut Ui, theme: &Palette) {
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

    // Animated pulsing orb
    let time = ui.input(|input| input.time) as f32;
    let pulse = time.sin() * 0.5 + 0.5;
    let orb_radius = 38.0 + pulse * 4.0;

    let orb_color = app.current_orb_color();

    for (i, alpha) in [40u8, 25, 12].iter().enumerate() {
        let glow_r = orb_radius + (i as f32 + 1.0) * 8.0;
        painter.circle_filled(
            center,
            glow_r,
            Color32::from_rgba_premultiplied(orb_color.r(), orb_color.g(), orb_color.b(), *alpha),
        );
    }

    painter.circle_filled(
        center,
        orb_radius,
        Color32::from_rgba_premultiplied(
            orb_color.r(),
            orb_color.g(),
            orb_color.b(),
            (60.0 + pulse * 40.0) as u8,
        ),
    );
    painter.circle_stroke(center, orb_radius, Stroke::new(2.0, orb_color));

    let main_name = app
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
        center + Vec2::new(0.0, orb_radius * 0.52),
        Align2::CENTER_CENTER,
        if app.connected { "ONLINE" } else { "OFFLINE" },
        FontId::monospace(13.0),
        if app.connected { theme.ok } else { theme.err },
    );

    let orb_rect = Rect::from_center_size(center, Vec2::splat(orb_radius * 2.0));
    let orb_response = ui.interact(orb_rect, Id::new("central_orb"), Sense::click());
    orb_response.context_menu(|ui| {
        if ui.button("Rename main agent").clicked() {
            app.rename_target = Some(AgentId::from("main"));
            app.rename_value = main_name.to_owned();
            ui.close_menu();
        }
    });

    // Collect agent ids to avoid borrow issues
    let agent_ids: Vec<String> = app.agents.iter().map(|a| a.id.to_string()).collect();

    // Ensure layouts exist for all agents
    for id in &agent_ids {
        let _ = app.agent_layout_mut(id);
    }

    for agent_id in &agent_ids {
        let layout = app.agent_layouts.get(agent_id).cloned();
        let Some(layout) = layout else { continue };
        if !layout.visible {
            continue;
        }

        let pos = Pos2::new(
            rect.left() + layout.position.x * rect.width(),
            rect.top() + layout.position.y * rect.height(),
        );

        // Connection line
        painter.line_segment(
            [center, pos],
            Stroke::new(1.0, Color32::from_rgba_premultiplied(120, 180, 220, 55)),
        );

        let agent = app
            .agents
            .iter()
            .find(|a| a.id.as_str() == agent_id.as_str())
            .cloned()
            .unwrap_or_else(|| placeholder_agent(0));

        // Agent card
        let card_size = Vec2::new(210.0, 86.0);
        let card = Rect::from_center_size(pos, card_size);
        painter.rect_filled(card, Rounding::same(8.0), theme.panel2);
        painter.rect_stroke(card, Rounding::same(8.0), Stroke::new(1.0, theme.border));

        let sc = status_color(agent.status, theme);
        painter.circle_filled(card.left_top() + Vec2::new(18.0, 18.0), 5.0, sc);
        painter.text(
            card.left_top() + Vec2::new(31.0, 10.0),
            Align2::LEFT_TOP,
            truncate(&agent.name, 20),
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
            truncate(detail, 28),
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

        // Hide button (×) at top-right of card
        let close_rect =
            Rect::from_min_size(card.right_top() + Vec2::new(-22.0, 4.0), Vec2::splat(18.0));
        painter.text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            FontId::monospace(14.0),
            theme.dim,
        );
        let close_resp = ui.interact(
            close_rect,
            Id::new(("hide", agent_id.as_str())),
            Sense::click(),
        );
        if close_resp.clicked() {
            if let Some(l) = app.agent_layouts.get_mut(agent_id) {
                l.visible = false;
            }
        }

        // Drag interaction
        let drag_resp = ui.interact(card, Id::new(("drag", agent_id.as_str())), Sense::drag());
        if drag_resp.dragged() {
            let delta = drag_resp.drag_delta();
            if let Some(l) = app.agent_layouts.get_mut(agent_id) {
                l.position.x = (l.position.x + delta.x / rect.width()).clamp(0.05, 0.95);
                l.position.y = (l.position.y + delta.y / rect.height()).clamp(0.05, 0.95);
            }
        }

        // Context menu
        let click_resp = ui.interact(card, Id::new(("agent", agent_id.as_str())), Sense::click());
        click_resp.context_menu(|ui| {
            if ui.button("Rename agent").clicked() {
                app.rename_target = Some(AgentId::from(agent_id.as_str()));
                app.rename_value = agent.name.clone();
                ui.close_menu();
            }
            if ui.button("Use first listed model").clicked() {
                if let Some(model) = app.model_catalog.first() {
                    app.set_main_model(model.model.clone());
                }
                ui.close_menu();
            }
        });
    }
}

pub(crate) fn agent_tray(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    let hidden: Vec<String> = app
        .agent_layouts
        .iter()
        .filter(|(_, l)| !l.visible)
        .map(|(id, _)| id.clone())
        .collect();
    if hidden.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("hidden:")
                .monospace()
                .small()
                .color(theme.dim),
        );
        for id in &hidden {
            let name = app
                .agents
                .iter()
                .find(|a| a.id.as_str() == id.as_str())
                .map(|a| a.name.as_str())
                .unwrap_or(id.as_str());
            if ui
                .add(Button::new(RichText::new(name).monospace().small()).fill(theme.panel2))
                .clicked()
            {
                if let Some(l) = app.agent_layouts.get_mut(id) {
                    l.visible = true;
                }
            }
        }
    });
}

pub(crate) fn chat_panel(app: &mut HudApp, ui: &mut Ui, theme: &Palette, height: f32) {
    glass_frame(theme).show(ui, |ui| {
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
                    app.composer = chip.to_owned();
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(Button::new(RichText::new("CLR").monospace()).fill(theme.panel2))
                    .on_hover_text("Clear chat")
                    .clicked()
                {
                    app.clear_chat();
                }
                if ui.button("models").clicked() {
                    app.config_open = true;
                    app.config_tab = ConfigTab::Models;
                }
                if ui.button("voice").clicked() {
                    app.config_open = true;
                    app.config_tab = ConfigTab::Voice;
                }
            });
        });
        ui.separator();

        let now = std::time::Instant::now();
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(height - 92.0)
            .show(ui, |ui| {
                for message in &app.messages {
                    let elapsed = now.duration_since(message.timestamp);
                    let mins = elapsed.as_secs() / 60;
                    let secs = elapsed.as_secs() % 60;

                    let (badge_color, text_color) = match message.role {
                        ChatRole::User => (theme.accent, theme.accent),
                        ChatRole::Assistant => (theme.ok, theme.text),
                        ChatRole::System => (theme.dim, theme.dim),
                    };

                    let badge = match &message.agent_name {
                        Some(name) => format!("{} ({})", message.role.label(), name),
                        None => message.role.label().to_owned(),
                    };

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{mins:02}:{secs:02}"))
                                .monospace()
                                .small()
                                .color(theme.faint),
                        );
                        ui.label(
                            RichText::new(&badge)
                                .monospace()
                                .strong()
                                .color(badge_color),
                        );
                    });

                    // Render text with code block detection
                    render_message_body(ui, &message.text, text_color, theme);
                    ui.add_space(4.0);
                }

                // Typing indicator
                if app.typing_indicator {
                    let time = ui.input(|i| i.time);
                    let dots = match ((time * 3.0) as usize) % 4 {
                        0 => ".",
                        1 => "..",
                        2 => "...",
                        _ => "",
                    };
                    ui.label(
                        RichText::new(format!("CADIS is typing{dots}"))
                            .monospace()
                            .color(theme.dim),
                    );
                }

                if app.scroll_to_bottom {
                    app.scroll_to_bottom = false;
                    ui.scroll_to_cursor(Some(Align::BOTTOM));
                }
            });

        ui.horizontal(|ui| {
            let input = ui.add_sized(
                [ui.available_width() - 92.0, 46.0],
                TextEdit::multiline(&mut app.composer)
                    .hint_text(if app.connected {
                        "type a CADIS command"
                    } else {
                        "cadisd is disconnected"
                    })
                    .desired_rows(2),
            );
            let enter_send =
                input.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
            if ui
                .add_enabled(
                    app.connected,
                    Button::new(RichText::new("SEND").monospace().strong()).fill(theme.accent),
                )
                .clicked()
                || enter_send
            {
                app.send_chat();
            }
        });
    });
}

fn render_message_body(ui: &mut Ui, text: &str, text_color: Color32, theme: &Palette) {
    let mut in_code = false;
    let mut code_buf = String::new();

    for line in text.split('\n') {
        if line.trim_start().starts_with("```") {
            if in_code {
                // Close code block
                egui::Frame::none()
                    .fill(theme.panel2)
                    .rounding(Rounding::same(4.0))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut code_buf.as_str())
                                .font(FontId::monospace(12.0))
                                .desired_width(f32::INFINITY)
                                .text_color(theme.text),
                        );
                    });
                code_buf.clear();
                in_code = false;
            } else {
                in_code = true;
            }
        } else if in_code {
            if !code_buf.is_empty() {
                code_buf.push('\n');
            }
            code_buf.push_str(line);
        } else {
            ui.add(
                egui::Label::new(RichText::new(line).monospace().color(text_color))
                    .selectable(true),
            );
        }
    }

    // Unclosed code block — render what we have
    if in_code && !code_buf.is_empty() {
        egui::Frame::none()
            .fill(theme.panel2)
            .rounding(Rounding::same(4.0))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.add(
                    TextEdit::multiline(&mut code_buf.as_str())
                        .font(FontId::monospace(12.0))
                        .desired_width(f32::INFINITY)
                        .text_color(theme.text),
                );
            });
    }
}

pub(crate) fn approval_stack(app: &mut HudApp, ctx: &Context, theme: &Palette) {
    if app.approvals.is_empty() {
        return;
    }

    Area::new(Id::new("approval_stack"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_TOP, [-22.0, 84.0])
        .show(ctx, |ui| {
            ui.set_width(360.0);
            for index in 0..app.approvals.len() {
                let mut decision = None;
                let approval_id = app.approvals[index].id.clone();
                let approval = &mut app.approvals[index];
                glass_frame(theme)
                    .stroke(Stroke::new(1.0, theme.warn))
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
                    app.request(ClientRequest::ApprovalRespond(ApprovalResponseRequest {
                        approval_id,
                        decision,
                        reason: Some("resolved from CADIS HUD".to_owned()),
                    }));
                }
                ui.add_space(8.0);
            }
        });
}

pub(crate) fn config_dialog(app: &mut HudApp, ctx: &Context, theme: &Palette) {
    if !app.config_open {
        return;
    }

    let mut open = app.config_open;
    Window::new("CADIS CONFIG")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(620.0)
        .frame(glass_frame(theme).stroke(Stroke::new(1.0, theme.border)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for tab in ConfigTab::all() {
                    if ui
                        .selectable_label(app.config_tab == tab, tab.label())
                        .clicked()
                    {
                        app.config_tab = tab;
                    }
                }
            });
            ui.separator();
            match app.config_tab {
                ConfigTab::Voice => voice_tab(app, ui, theme),
                ConfigTab::Models => models_tab(app, ui, theme),
                ConfigTab::Appearance => appearance_tab(app, ui, theme),
                ConfigTab::Window => window_tab(app, ui, theme),
                ConfigTab::Debug => debug_tab(app, ui, theme),
            }
        });
    app.config_open = open;
}

pub(crate) fn voice_tab(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    ui.label(
        RichText::new("voice provider route")
            .monospace()
            .color(theme.dim),
    );
    ComboBox::from_label("voice")
        .selected_text(&app.selected_voice)
        .show_ui(ui, |ui| {
            for voice in VOICES {
                ui.selectable_value(&mut app.selected_voice, (*voice).to_owned(), *voice);
            }
        });
    ui.horizontal(|ui| {
        if ui.button("TEST VOICE").clicked() {
            app.preview_voice();
        }
        if ui.button("STOP").clicked() {
            app.request(ClientRequest::VoiceStop(EmptyPayload::default()));
        }
    });
    ui.label(
        RichText::new(&app.voice_notice)
            .monospace()
            .color(theme.faint),
    );
}

pub(crate) fn models_tab(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    if ui.button("REFRESH MODELS").clicked() {
        app.request(ClientRequest::ModelsList(EmptyPayload::default()));
    }
    ui.separator();
    let current = app
        .agents
        .iter()
        .find(|agent| agent.id.as_str() == "main")
        .map(|agent| agent.model.clone())
        .unwrap_or_else(|| "auto".to_owned());
    let mut selected = current.clone();
    ComboBox::from_label("main agent model")
        .selected_text(&selected)
        .show_ui(ui, |ui| {
            for model in &app.model_catalog {
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
        app.set_main_model(selected);
    }
    ui.label(
        RichText::new("model changes are confirmed by agent.model.changed")
            .monospace()
            .small()
            .color(theme.faint),
    );
}

pub(crate) fn appearance_tab(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    ui.label(RichText::new("theme").monospace().color(theme.dim));
    ui.horizontal_wrapped(|ui| {
        for key in ThemeKey::all() {
            let palette = key.palette();
            let button = Button::new(key.label())
                .fill(palette.accent)
                .stroke(Stroke::new(
                    if app.theme == key { 2.0 } else { 1.0 },
                    theme.text,
                ));
            if ui.add(button).clicked() {
                app.set_theme(key);
            }
        }
    });
    let mut opacity = app.opacity as f32;
    if ui
        .add(Slider::new(&mut opacity, 45.0..=100.0).text("background opacity"))
        .changed()
    {
        app.set_opacity(opacity.round() as u8);
    }
}

pub(crate) fn window_tab(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    ui.checkbox(&mut app.always_on_top, "always on top preference");
    ui.label(
        RichText::new("window preference is UI-local in this prototype")
            .monospace()
            .small()
            .color(theme.faint),
    );
    if ui.button("RECHECK DAEMON").clicked() {
        app.request(ClientRequest::DaemonStatus(EmptyPayload::default()));
    }
}

pub(crate) fn debug_tab(app: &mut HudApp, ui: &mut Ui, theme: &Palette) {
    ui.checkbox(&mut app.debug_enabled, "Enable debug mode");
    if !app.debug_enabled {
        return;
    }
    ui.separator();

    // FPS counter
    let fps = if app.frame_times.len() >= 2 {
        let span = app
            .frame_times
            .back()
            .unwrap()
            .duration_since(*app.frame_times.front().unwrap());
        if span.as_secs_f64() > 0.0 {
            (app.frame_times.len() as f64 - 1.0) / span.as_secs_f64()
        } else {
            0.0
        }
    } else {
        0.0
    };
    ui.label(
        RichText::new(format!("FPS: {fps:.1}"))
            .monospace()
            .color(theme.text),
    );
    ui.label(
        RichText::new(format!("Events received: {}", app.event_count))
            .monospace()
            .color(theme.text),
    );
    ui.label(
        RichText::new(format!("Messages: {}", app.messages.len()))
            .monospace()
            .color(theme.text),
    );
    ui.label(
        RichText::new(format!("Agents: {}", app.agents.len()))
            .monospace()
            .color(theme.text),
    );
    ui.separator();

    // Connection info
    ui.label(
        RichText::new(format!("Transport: {}", app.transport))
            .monospace()
            .color(theme.dim),
    );
    ui.label(
        RichText::new(format!("Connected: {}", app.connected))
            .monospace()
            .color(theme.dim),
    );
    ui.separator();

    // Agent list
    ui.label(
        RichText::new("Agents")
            .monospace()
            .strong()
            .color(theme.accent),
    );
    for agent in &app.agents {
        ui.label(
            RichText::new(format!(
                "  {} — {:?} — {}",
                agent.id, agent.status, agent.model
            ))
            .monospace()
            .color(theme.dim),
        );
    }
    ui.separator();

    // Event log
    ui.label(
        RichText::new("Event log (last 20)")
            .monospace()
            .strong()
            .color(theme.accent),
    );
    ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
        let now = std::time::Instant::now();
        for event in app.debug_events.iter().rev() {
            let age = now.duration_since(event.timestamp);
            ui.label(
                RichText::new(format!("{:.1}s ago — {}", age.as_secs_f64(), event.label))
                    .monospace()
                    .small()
                    .color(theme.faint),
            );
        }
    });
    if ui.button("CLEAR EVENTS").clicked() {
        app.debug_events.clear();
        app.event_count = 0;
    }
}

pub(crate) fn rename_dialog(app: &mut HudApp, ctx: &Context, theme: &Palette) {
    if app.rename_target.is_none() {
        return;
    }

    let target = app
        .rename_target
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    Window::new(format!("RENAME {target}"))
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .frame(glass_frame(theme).stroke(Stroke::new(1.0, theme.border)))
        .show(ctx, |ui| {
            ui.add(TextEdit::singleline(&mut app.rename_value).hint_text("agent display name"));
            ui.horizontal(|ui| {
                if ui.button("CANCEL").clicked() {
                    app.rename_target = None;
                    app.rename_value.clear();
                }
                if ui
                    .add_enabled(app.connected, Button::new("SAVE").fill(theme.accent))
                    .clicked()
                {
                    app.send_rename();
                }
            });
            if !app.connected {
                ui.label(RichText::new("cadisd disconnected").color(theme.err));
            }
        });
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut result = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        result.push('…');
    }
    result
}
