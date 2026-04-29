use cadis_protocol::AgentStatus;
use eframe::egui::{Color32, Frame, Margin, Pos2, Rect, Rounding, Stroke};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThemeKey {
    Arc,
    Amber,
    Phosphor,
    Violet,
    Alert,
    Ice,
}

impl ThemeKey {
    pub(crate) fn all() -> [Self; 6] {
        [
            Self::Arc,
            Self::Amber,
            Self::Phosphor,
            Self::Violet,
            Self::Alert,
            Self::Ice,
        ]
    }

    pub(crate) fn from_key(value: &str) -> Option<Self> {
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

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Arc => "arc",
            Self::Amber => "amber",
            Self::Phosphor => "phosphor",
            Self::Violet => "violet",
            Self::Alert => "alert",
            Self::Ice => "ice",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Arc => "ARC",
            Self::Amber => "AMBER",
            Self::Phosphor => "PHOSPHOR",
            Self::Violet => "VIOLET",
            Self::Alert => "ALERT",
            Self::Ice => "ICE",
        }
    }

    pub(crate) fn palette(self) -> Palette {
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
pub(crate) struct Palette {
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) panel2: Color32,
    pub(crate) border: Color32,
    pub(crate) border_dark: Color32,
    pub(crate) text: Color32,
    pub(crate) dim: Color32,
    pub(crate) faint: Color32,
    pub(crate) accent: Color32,
    pub(crate) ok: Color32,
    pub(crate) warn: Color32,
    pub(crate) err: Color32,
}

impl Palette {
    fn new(bg: (u8, u8, u8), accent: (u8, u8, u8)) -> Self {
        Self {
            bg: Color32::from_rgb(bg.0, bg.1, bg.2),
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

pub(crate) fn status_color(status: AgentStatus, theme: &Palette) -> Color32 {
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

pub(crate) fn draw_grid(painter: &eframe::egui::Painter, rect: Rect, theme: &Palette) {
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

pub(crate) fn slot_positions(rect: Rect) -> [Pos2; 12] {
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

pub(crate) fn glass_frame(theme: &Palette) -> Frame {
    Frame::none()
        .fill(Color32::from_rgba_premultiplied(
            theme.bg.r(),
            theme.bg.g(),
            theme.bg.b(),
            200,
        ))
        .rounding(Rounding::same(12.0))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(30)))
        .inner_margin(Margin::same(16.0))
}

pub(crate) fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color32::from_rgba_premultiplied(
        lerp(from.r(), to.r()),
        lerp(from.g(), to.g()),
        lerp(from.b(), to.b()),
        lerp(from.a(), to.a()),
    )
}
