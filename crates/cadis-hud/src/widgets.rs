use cadis_protocol::{
    AgentId, ApprovalDecision, ApprovalResponseRequest, ClientRequest, EmptyPayload,
};
use eframe::egui::{
    self, Align, Align2, Area, Button, Color32, ComboBox, Context, FontId, Id, Key, Layout, Order,
    Pos2, Rect, RichText, Rounding, ScrollArea, Sense, Slider, Stroke, TextEdit, Ui, Vec2,
    ViewportCommand, Window,
};

use crate::theme::{draw_grid, glass_frame, slot_positions, status_color, Palette, ThemeKey};
use crate::types::{placeholder_agent, AgentView, ChatRole, ConfigTab};
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
    let pulse = time.sin() * 0.5 + 0.5; // 0..1
    let orb_radius = 38.0 + pulse * 4.0; // 38..42

    let orb_color = app.current_orb_color();

    // Glow: 3 concentric circles with decreasing alpha
    for (i, alpha) in [40u8, 25, 12].iter().enumerate() {
        let glow_r = orb_radius + (i as f32 + 1.0) * 8.0;
        painter.circle_filled(
            center,
            glow_r,
            Color32::from_rgba_premultiplied(orb_color.r(), orb_color.g(), orb_color.b(), *alpha),
        );
    }

    // Core orb fill
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

    let slots = slot_positions(rect);
    for (index, position) in slots.into_iter().enumerate() {
        painter.line_segment(
            [center, position],
            Stroke::new(1.0, Color32::from_rgba_premultiplied(120, 180, 220, 55)),
        );
        let agent = app
            .agents
            .get(index)
            .cloned()
            .unwrap_or_else(|| placeholder_agent(index));
        agent_card(app, ui, &painter, theme, position, agent);
    }
}

pub(crate) fn agent_card(
    app: &mut HudApp,
    ui: &mut Ui,
    painter: &egui::Painter,
    theme: &Palette,
    center: Pos2,
    agent: AgentView,
) {
    let card = Rect::from_center_size(center, Vec2::new(210.0, 86.0));
    painter.rect_filled(card, Rounding::same(8.0), theme.panel2);
    painter.rect_stroke(card, Rounding::same(8.0), Stroke::new(1.0, theme.border));

    let sc = status_color(agent.status, theme);
    painter.circle_filled(card.left_top() + Vec2::new(18.0, 18.0), 5.0, sc);
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
            app.rename_target = Some(agent.id.clone());
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

        ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(height - 92.0)
            .show(ui, |ui| {
                for message in &app.messages {
                    let color = match message.role {
                        ChatRole::User => theme.accent,
                        ChatRole::Assistant => theme.text,
                        ChatRole::System => theme.dim,
                    };
                    ui.label(
                        RichText::new(format!("{}  {}", message.role.label(), message.text))
                            .color(color)
                            .monospace(),
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
