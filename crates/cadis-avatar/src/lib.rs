//! CADIS-native avatar state engine.
//!
//! This crate owns renderer-independent avatar state for the Wulan avatar path.
//! It intentionally avoids depending on Tauri, web UI, `wgpu`, Bevy, or camera
//! capture crates. Renderer adapters can consume [`AvatarFrame`] through the
//! [`AvatarRenderer`] trait, and direct `wgpu` renderers can consume
//! [`WgpuAvatarUniforms`] without pulling `wgpu` into this state crate.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(feature = "wgpu-renderer")]
pub mod wgpu_renderer;

/// Identifier used by the Wulan avatar prototype and native engine.
pub const WULAN_AVATAR_ID: &str = "wulan_arc";

/// Avatar style used when native Wulan rendering cannot produce a frame.
pub const CADIS_ORB_AVATAR_ID: &str = "orb";

/// Supported avatar renderer backend targets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererBackend {
    /// Headless renderer contract used by tests and daemon-side planning.
    Headless,
    /// Rust-native renderer target using direct `wgpu` integration.
    #[default]
    WgpuNative,
    /// Bevy scene renderer target for richer body and scene orchestration.
    BevyScene,
}

/// HUD avatar target used when a native renderer fails or is unavailable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarFallbackTarget {
    /// Stable default CADIS orb path.
    #[default]
    #[serde(rename = "orb")]
    CadisOrb,
    /// Static Wulan portrait texture without native animation.
    StaticWulanTexture,
}

impl AvatarFallbackTarget {
    /// Returns the daemon/HUD avatar style identifier for this fallback target.
    pub fn avatar_id(self) -> &'static str {
        match self {
            Self::CadisOrb => CADIS_ORB_AVATAR_ID,
            Self::StaticWulanTexture => WULAN_AVATAR_ID,
        }
    }
}

/// Renderer fallback behavior promised by native avatar adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AvatarFallbackContract {
    /// HUD avatar target to show when native rendering fails.
    pub target: AvatarFallbackTarget,
    /// Whether renderer failure must leave the HUD launch path available.
    pub preserves_hud_launch: bool,
    /// Whether fallback events should carry a coarse reason code.
    pub reason_code_required: bool,
    /// Whether reduced-motion state should be preserved through fallback.
    pub reduced_motion_passthrough: bool,
}

impl Default for AvatarFallbackContract {
    fn default() -> Self {
        Self {
            target: AvatarFallbackTarget::CadisOrb,
            preserves_hud_launch: true,
            reason_code_required: true,
            reduced_motion_passthrough: true,
        }
    }
}

/// Coarse reason a renderer adapter fell back instead of drawing Wulan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarFallbackReason {
    /// Renderer implementation was not available.
    #[default]
    RendererUnavailable,
    /// Renderer lost or could not acquire its target surface.
    SurfaceLost,
    /// Renderer returned an error while drawing a frame.
    RenderError,
    /// Requested backend is not supported by the adapter.
    UnsupportedBackend,
}

/// Minimal state a HUD adapter needs to show a fallback avatar.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AvatarFallbackState {
    /// Fallback avatar style to show.
    pub target: AvatarFallbackTarget,
    /// Coarse fallback reason suitable for HUD state and tests.
    pub reason: AvatarFallbackReason,
    /// Renderer backend that failed or was unavailable.
    pub failed_backend: RendererBackend,
    /// Runtime mode being rendered when fallback was selected.
    pub mode: AvatarMode,
    /// Whether reduced-motion state remains active in fallback.
    pub reduced_motion: bool,
    /// Monotonic timestamp in milliseconds from the source frame.
    pub time_ms: u64,
}

impl RendererBackend {
    /// Returns why this backend is useful for CADIS.
    pub fn rationale(self) -> &'static str {
        match self {
            Self::Headless => "test and protocol contract without GPU dependencies",
            Self::WgpuNative => {
                "smallest native GPU path for HUD embedding and tight frame control"
            }
            Self::BevyScene => "richer avatar scene graph, gestures, and animation systems",
        }
    }
}

/// High-level avatar mode derived from CADIS runtime state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarMode {
    /// Resting state.
    #[default]
    Idle,
    /// User microphone input is active.
    Listening,
    /// CADIS is reasoning.
    Thinking,
    /// CADIS is speaking.
    Speaking,
    /// CADIS is doing code or tool work.
    Coding,
    /// CADIS is waiting for user approval.
    Approval,
    /// Error or blocked state.
    Error,
}

/// Body gesture selected by the avatar engine.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyGesture {
    /// Small breathing and idle stance.
    #[default]
    IdleBreath,
    /// Slight forward attention pose.
    AttentiveLean,
    /// Slight forward lean while listening.
    ListeningLean,
    /// Short affirmative head nod.
    Nod,
    /// Lateral gaze shift for scanning or curiosity.
    GazeShift,
    /// Orbiting scan / thinking pose.
    ThinkingOrbit,
    /// Scanning sweep during thinking.
    ThinkingScan,
    /// Pulse synced to speech amplitude.
    SpeakingPulse,
    /// Emphatic hand or body motion while speaking.
    SpeakingEmphasis,
    /// Focused coding pose with restrained motion.
    CodingFocus,
    /// Approval hold pose.
    ApprovalHold,
    /// Hand cue acknowledging approval.
    ApprovalHandCue,
    /// Alert pose for errors.
    ErrorAlert,
    /// Recoil motion on error or rejection.
    ErrorRecoil,
}

/// Gesture priority used when renderers blend or interrupt body motion.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyGesturePriority {
    /// Ambient loops that may be interrupted by any active state.
    #[default]
    Ambient,
    /// Normal activity state such as listening, thinking, speaking, or coding.
    Activity,
    /// User attention state such as waiting for approval.
    Interaction,
    /// Safety or error state that should interrupt decorative animation.
    Safety,
}

/// Stateful body gesture selected by the avatar engine.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct BodyGestureState {
    /// Current gesture.
    pub gesture: BodyGesture,
    /// Priority used by renderers when blending or interrupting motion.
    pub priority: BodyGesturePriority,
    /// Gesture intensity in 0..1 after reduced-motion handling.
    pub intensity: f32,
    /// Elapsed time since this gesture became active.
    pub elapsed_ms: u64,
    /// Whether this frame started a different gesture than the previous frame.
    pub interrupted: bool,
    /// Whether large body motion has been reduced for accessibility.
    pub reduced_motion: bool,
}

/// Optional face tracking mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceTrackingMode {
    /// No camera or face tracking input is used.
    #[default]
    Off,
    /// UI may ask for permission before sending local-only face tracking frames.
    PermissionRequired,
    /// Renderer may consume local-only face tracking frames.
    LocalOnly,
}

/// Privacy status attached to a face tracking sample.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceTrackingConsent {
    /// User has not been asked yet.
    #[default]
    NotRequested,
    /// User denied face tracking.
    Denied,
    /// User granted local-only face tracking.
    GrantedLocalOnly,
}

/// Optional face tracking controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FaceTrackingConfig {
    /// Face tracking mode.
    pub mode: FaceTrackingMode,
    /// Whether UI permission is required before camera access.
    pub permission_required: bool,
    /// Whether the HUD must show a camera-active indicator while tracking.
    pub camera_indicator_required: bool,
    /// Whether the HUD must provide a one-click disable action.
    pub one_click_disable_required: bool,
    /// Minimum confidence, as a percent, before a tracking frame can drive pose.
    pub min_confidence_percent: u8,
}

impl Default for FaceTrackingConfig {
    fn default() -> Self {
        Self {
            mode: FaceTrackingMode::Off,
            permission_required: true,
            camera_indicator_required: true,
            one_click_disable_required: true,
            min_confidence_percent: 35,
        }
    }
}

impl FaceTrackingConfig {
    /// Returns true when camera-derived pose may be considered by the engine.
    pub fn accepts(self, frame: FaceTrackingFrame) -> bool {
        self.mode == FaceTrackingMode::LocalOnly
            && frame.consent == FaceTrackingConsent::GrantedLocalOnly
            && frame.confidence >= f32::from(self.min_confidence_percent.min(100)) / 100.0
    }
}

/// Avatar privacy policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AvatarPrivacy {
    /// Whether face tracking frames must remain local to the renderer process.
    pub local_only_face_tracking: bool,
    /// Whether raw face tracking frames may be persisted.
    pub persist_raw_face_frames: bool,
    /// Whether derived landmarks may be persisted.
    pub persist_face_landmarks: bool,
    /// Whether face tracking data may be sent outside the local process.
    pub allow_remote_face_tracking: bool,
    /// Whether identity recognition or biometric templates may be used.
    pub allow_face_identity: bool,
}

impl Default for AvatarPrivacy {
    fn default() -> Self {
        Self {
            local_only_face_tracking: true,
            persist_raw_face_frames: false,
            persist_face_landmarks: false,
            allow_remote_face_tracking: false,
            allow_face_identity: false,
        }
    }
}

/// Engine configuration for the Wulan native avatar.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AvatarEngineConfig {
    /// Avatar asset or rig identifier.
    pub avatar_id: String,
    /// Preferred native renderer backend.
    pub renderer: RendererBackend,
    /// Fallback avatar target when the native renderer fails.
    pub renderer_fallback: AvatarFallbackTarget,
    /// Face tracking config.
    pub face_tracking: FaceTrackingConfig,
    /// Privacy policy for any face tracking data.
    pub privacy: AvatarPrivacy,
    /// Whether large body gestures should be reduced for accessibility.
    pub reduced_motion: bool,
    /// Maximum simulation delta accepted by the engine.
    pub max_delta_ms: u32,
}

impl Default for AvatarEngineConfig {
    fn default() -> Self {
        Self {
            avatar_id: WULAN_AVATAR_ID.to_owned(),
            renderer: RendererBackend::WgpuNative,
            renderer_fallback: AvatarFallbackTarget::CadisOrb,
            face_tracking: FaceTrackingConfig::default(),
            privacy: AvatarPrivacy::default(),
            reduced_motion: false,
            max_delta_ms: 250,
        }
    }
}

impl AvatarEngineConfig {
    /// Validates privacy-sensitive avatar config.
    pub fn validate_privacy(&self) -> Result<(), AvatarConfigError> {
        if self.face_tracking.mode == FaceTrackingMode::Off {
            return Ok(());
        }

        if !self.face_tracking.permission_required {
            return Err(AvatarConfigError::FaceTrackingRequiresPermission);
        }
        if !self.face_tracking.camera_indicator_required {
            return Err(AvatarConfigError::FaceTrackingRequiresCameraIndicator);
        }
        if !self.face_tracking.one_click_disable_required {
            return Err(AvatarConfigError::FaceTrackingRequiresDisableAction);
        }
        if !self.privacy.local_only_face_tracking || self.privacy.allow_remote_face_tracking {
            return Err(AvatarConfigError::FaceTrackingMustStayLocal);
        }
        if self.privacy.persist_raw_face_frames || self.privacy.persist_face_landmarks {
            return Err(AvatarConfigError::FaceTrackingMustNotPersist);
        }
        if self.privacy.allow_face_identity {
            return Err(AvatarConfigError::FaceIdentityNotAllowed);
        }

        Ok(())
    }

    /// Returns the renderer contract implied by this configuration.
    pub fn renderer_contract(&self) -> AvatarRendererContract {
        AvatarRendererContract::for_backend(self.renderer).with_fallback(self.renderer_fallback)
    }
}

/// Invalid avatar configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AvatarConfigError {
    /// Face tracking cannot be enabled without explicit permission.
    FaceTrackingRequiresPermission,
    /// Face tracking cannot be enabled without a visible active-camera indicator.
    FaceTrackingRequiresCameraIndicator,
    /// Face tracking cannot be enabled without a one-click disable action.
    FaceTrackingRequiresDisableAction,
    /// Face tracking data must stay local.
    FaceTrackingMustStayLocal,
    /// Face frames and landmarks must not be persisted by default.
    FaceTrackingMustNotPersist,
    /// Identity recognition and biometric templates are not allowed.
    FaceIdentityNotAllowed,
}

impl fmt::Display for AvatarConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::FaceTrackingRequiresPermission => {
                "face tracking requires explicit user permission"
            }
            Self::FaceTrackingRequiresCameraIndicator => {
                "face tracking requires a visible camera-active indicator"
            }
            Self::FaceTrackingRequiresDisableAction => {
                "face tracking requires a one-click disable action"
            }
            Self::FaceTrackingMustStayLocal => "face tracking data must remain local",
            Self::FaceTrackingMustNotPersist => {
                "face tracking frames and landmarks must not be persisted"
            }
            Self::FaceIdentityNotAllowed => {
                "face identity recognition and biometric templates are not allowed"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for AvatarConfigError {}

/// Runtime input for one avatar engine tick.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AvatarInput {
    /// Runtime mode.
    pub mode: AvatarMode,
    /// Audio amplitude in the range 0..1.
    pub audio_level: f32,
    /// Optional local-only face tracking frame.
    pub face_tracking: Option<FaceTrackingFrame>,
    /// Monotonic timestamp in milliseconds.
    pub now_ms: u64,
}

impl Default for AvatarInput {
    fn default() -> Self {
        Self {
            mode: AvatarMode::Idle,
            audio_level: 0.0,
            face_tracking: None,
            now_ms: 0,
        }
    }
}

/// Local-only face tracking signal.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FaceTrackingFrame {
    /// Permission state associated with this sample.
    pub consent: FaceTrackingConsent,
    /// Horizontal gaze target in -1..1.
    pub gaze_x: f32,
    /// Vertical gaze target in -1..1.
    pub gaze_y: f32,
    /// Left eyelid openness in 0..1.
    pub blink_left: f32,
    /// Right eyelid openness in 0..1.
    pub blink_right: f32,
    /// Mouth openness in 0..1.
    pub mouth_open: f32,
    /// Brow raise in 0..1.
    pub brow_raise: f32,
    /// Head yaw in -1..1.
    pub head_yaw: f32,
    /// Head pitch in -1..1.
    pub head_pitch: f32,
    /// Tracker confidence in 0..1.
    pub confidence: f32,
}

impl Default for FaceTrackingFrame {
    fn default() -> Self {
        Self {
            consent: FaceTrackingConsent::NotRequested,
            gaze_x: 0.0,
            gaze_y: 0.0,
            blink_left: 1.0,
            blink_right: 1.0,
            mouth_open: 0.0,
            brow_raise: 0.0,
            head_yaw: 0.0,
            head_pitch: 0.0,
            confidence: 0.0,
        }
    }
}

/// Body pose sent to a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct BodyPose {
    /// Gesture selected by mode and audio state.
    pub gesture: BodyGesture,
    /// Gesture intensity in 0..1.
    pub intensity: f32,
    /// Head yaw in -1..1.
    pub head_yaw: f32,
    /// Head pitch in -1..1.
    pub head_pitch: f32,
    /// Shoulder roll in -1..1.
    pub shoulder_roll: f32,
    /// Left hand emphasis in 0..1.
    pub left_hand: f32,
    /// Right hand emphasis in 0..1.
    pub right_hand: f32,
}

/// Face expression sent to a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct FacePose {
    /// Horizontal gaze target in -1..1.
    pub gaze_x: f32,
    /// Vertical gaze target in -1..1.
    pub gaze_y: f32,
    /// Left eyelid openness in 0..1.
    pub blink_left: f32,
    /// Right eyelid openness in 0..1.
    pub blink_right: f32,
    /// Mouth openness in 0..1.
    pub mouth_open: f32,
    /// Brow raise in 0..1.
    pub brow_raise: f32,
    /// Whether this pose came from local face tracking.
    pub tracked: bool,
}

/// Material and shader hints for avatar renderers.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct AvatarMaterial {
    /// Primary RGB color.
    pub primary_rgb: [f32; 3],
    /// Secondary RGB color.
    pub secondary_rgb: [f32; 3],
    /// Glow intensity in 0..1.
    pub glow: f32,
    /// Scanline intensity in 0..1.
    pub scanline: f32,
}

/// Renderable frame generated by the avatar engine.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AvatarFrame {
    /// Avatar asset or rig identifier.
    pub avatar_id: String,
    /// Preferred renderer backend for this frame.
    pub renderer: RendererBackend,
    /// Runtime mode.
    pub mode: AvatarMode,
    /// Stateful body gesture metadata.
    pub body_state: BodyGestureState,
    /// Body pose.
    pub body: BodyPose,
    /// Face pose.
    pub face: FacePose,
    /// Material hints.
    pub material: AvatarMaterial,
    /// Privacy policy attached to the frame.
    pub privacy: AvatarPrivacy,
    /// Monotonic timestamp in milliseconds.
    pub time_ms: u64,
}

impl AvatarFrame {
    /// Converts the frame to a compact direct-wgpu uniform payload.
    pub fn wgpu_uniforms(&self) -> WgpuAvatarUniforms {
        WgpuAvatarUniforms {
            version: WGPU_AVATAR_UNIFORM_VERSION,
            time_seconds: self.time_ms as f32 / 1000.0,
            mode_id: mode_id(self.mode),
            gesture_id: gesture_id(self.body_state.gesture),
            gesture_priority: priority_id(self.body_state.priority),
            gesture_intensity: self.body_state.intensity,
            head: [self.body.head_yaw, self.body.head_pitch],
            hands: [self.body.left_hand, self.body.right_hand],
            face: [
                self.face.gaze_x,
                self.face.gaze_y,
                self.face.blink_left,
                self.face.blink_right,
                self.face.mouth_open,
                self.face.brow_raise,
            ],
            primary_rgb: self.material.primary_rgb,
            secondary_rgb: self.material.secondary_rgb,
            glow_scanline: [self.material.glow, self.material.scanline],
            flags: WgpuAvatarUniformFlags {
                tracked_face: self.face.tracked,
                reduced_motion: self.body_state.reduced_motion,
            },
        }
    }
}

impl AvatarFallbackContract {
    /// Converts a failed render attempt into minimal fallback state for the HUD.
    pub fn state_for_frame(
        self,
        failed_backend: RendererBackend,
        frame: &AvatarFrame,
        reason: AvatarFallbackReason,
    ) -> AvatarFallbackState {
        AvatarFallbackState {
            target: self.target,
            reason,
            failed_backend,
            mode: frame.mode,
            reduced_motion: self.reduced_motion_passthrough && frame.body_state.reduced_motion,
            time_ms: frame.time_ms,
        }
    }
}

/// Version of the direct-wgpu avatar uniform payload.
pub const WGPU_AVATAR_UNIFORM_VERSION: u16 = 1;

/// Renderer-independent target shape for the future direct `wgpu` renderer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WgpuSurfaceTarget {
    /// Draw into the HUD compositing surface.
    #[default]
    HudComposite,
    /// Draw into an offscreen texture owned by the HUD.
    OffscreenTexture,
    /// Draw into an externally supplied native surface.
    ExternalSurface,
}

/// Texture format hint for direct `wgpu` renderers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WgpuTextureFormatHint {
    /// Standard sRGB BGRA swapchain format.
    #[default]
    Bgra8UnormSrgb,
    /// Standard sRGB RGBA texture format.
    Rgba8UnormSrgb,
}

/// Alpha compositing hint for transparent HUD rendering.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WgpuAlphaModeHint {
    /// Premultiplied alpha for transparent HUD composition.
    #[default]
    Premultiplied,
    /// Opaque target.
    Opaque,
}

/// Required primitive family for the first native Wulan renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WgpuAvatarPrimitive {
    /// Wulan portrait or rig plane.
    PortraitPlane,
    /// Hologram material and scanline pass.
    HologramMaterial,
    /// Reticle and orbit rings.
    ReticleRings,
    /// Lightweight particles.
    Particles,
    /// Eyes, blink, gaze, and mouth overlays.
    FaceOverlay,
    /// Body gesture pose controls.
    BodyGestureRig,
}

/// Direct `wgpu` renderer contract without depending on the `wgpu` crate.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WgpuRendererContract {
    /// Uniform payload version expected by the renderer.
    pub uniform_version: u16,
    /// Target shape expected by CADIS HUD integration.
    pub target: WgpuSurfaceTarget,
    /// Texture format hint.
    pub texture_format: WgpuTextureFormatHint,
    /// Alpha mode hint.
    pub alpha_mode: WgpuAlphaModeHint,
    /// Primitive families the renderer should implement first.
    pub primitives: Vec<WgpuAvatarPrimitive>,
    /// Whether camera access is required for this renderer.
    pub requires_camera: bool,
}

impl Default for WgpuRendererContract {
    fn default() -> Self {
        Self {
            uniform_version: WGPU_AVATAR_UNIFORM_VERSION,
            target: WgpuSurfaceTarget::HudComposite,
            texture_format: WgpuTextureFormatHint::Bgra8UnormSrgb,
            alpha_mode: WgpuAlphaModeHint::Premultiplied,
            primitives: vec![
                WgpuAvatarPrimitive::PortraitPlane,
                WgpuAvatarPrimitive::HologramMaterial,
                WgpuAvatarPrimitive::ReticleRings,
                WgpuAvatarPrimitive::Particles,
                WgpuAvatarPrimitive::FaceOverlay,
                WgpuAvatarPrimitive::BodyGestureRig,
            ],
            requires_camera: false,
        }
    }
}

/// Bevy renderer status for the avatar plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BevyRendererStatus {
    /// Bevy is intentionally deferred until CADIS accepts a broader 3D decision.
    #[default]
    Deferred,
    /// Bevy can be provided behind the optional `bevy-renderer` feature later.
    OptionalFeature,
}

/// Bevy renderer contract placeholder.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BevyRendererContract {
    /// Current integration status.
    pub status: BevyRendererStatus,
    /// Cargo feature name reserved for a future Bevy adapter.
    pub feature_name: String,
    /// Why Bevy is not the first production path.
    pub rationale: String,
}

impl Default for BevyRendererContract {
    fn default() -> Self {
        Self {
            status: BevyRendererStatus::Deferred,
            feature_name: "bevy-renderer".to_owned(),
            rationale: "deferred until CADIS needs a broader 3D scene engine".to_owned(),
        }
    }
}

/// Full renderer contract exposed by the avatar state crate.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AvatarRendererContract {
    /// Preferred renderer backend.
    pub preferred_backend: RendererBackend,
    /// Direct `wgpu` renderer contract.
    pub wgpu: WgpuRendererContract,
    /// Deferred Bevy renderer contract.
    pub bevy: BevyRendererContract,
    /// Required fallback behavior when native rendering fails.
    pub fallback: AvatarFallbackContract,
}

impl AvatarRendererContract {
    /// Builds a renderer contract for a preferred backend.
    pub fn for_backend(preferred_backend: RendererBackend) -> Self {
        Self {
            preferred_backend,
            wgpu: WgpuRendererContract::default(),
            bevy: BevyRendererContract::default(),
            fallback: AvatarFallbackContract::default(),
        }
    }

    /// Overrides the fallback target while preserving fallback safety rules.
    pub fn with_fallback(mut self, target: AvatarFallbackTarget) -> Self {
        self.fallback.target = target;
        self
    }

    /// Builds the HUD fallback state for a frame that could not be rendered.
    pub fn fallback_for_frame(
        &self,
        failed_backend: RendererBackend,
        frame: &AvatarFrame,
        reason: AvatarFallbackReason,
    ) -> AvatarFallbackState {
        self.fallback.state_for_frame(failed_backend, frame, reason)
    }
}

/// Flags carried in [`WgpuAvatarUniforms`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WgpuAvatarUniformFlags {
    /// Whether face values came from local-only face tracking.
    pub tracked_face: bool,
    /// Whether reduced-motion handling is active.
    pub reduced_motion: bool,
}

/// Compact dynamic payload for a direct `wgpu` avatar renderer.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuAvatarUniforms {
    /// Uniform layout version.
    pub version: u16,
    /// Monotonic frame time in seconds.
    pub time_seconds: f32,
    /// Stable numeric avatar mode ID.
    pub mode_id: u32,
    /// Stable numeric gesture ID.
    pub gesture_id: u32,
    /// Stable numeric gesture priority ID.
    pub gesture_priority: u32,
    /// Current gesture intensity in 0..1.
    pub gesture_intensity: f32,
    /// Head yaw and pitch.
    pub head: [f32; 2],
    /// Left and right hand emphasis.
    pub hands: [f32; 2],
    /// Gaze x/y, blink left/right, mouth open, and brow raise.
    pub face: [f32; 6],
    /// Primary RGB material color.
    pub primary_rgb: [f32; 3],
    /// Secondary RGB material color.
    pub secondary_rgb: [f32; 3],
    /// Glow and scanline intensity.
    pub glow_scanline: [f32; 2],
    /// Boolean state flags.
    pub flags: WgpuAvatarUniformFlags,
}

/// Render result metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AvatarRenderReceipt {
    /// Backend that rendered the frame.
    pub backend: RendererBackend,
    /// Frame timestamp that was rendered.
    pub time_ms: u64,
}

/// Renderer abstraction implemented by future wgpu or Bevy adapters.
pub trait AvatarRenderer {
    /// Backend implemented by this renderer.
    fn backend(&self) -> RendererBackend;

    /// Renders one avatar frame.
    fn render(&mut self, frame: &AvatarFrame) -> Result<AvatarRenderReceipt, AvatarRenderError>;
}

/// Extension trait for future direct `wgpu` avatar renderers.
pub trait WgpuAvatarRenderer: AvatarRenderer {
    /// Returns the no-dependency `wgpu` contract this renderer implements.
    fn wgpu_contract(&self) -> WgpuRendererContract;
}

/// Result of attempting to render a frame with fallback handling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AvatarRenderAttempt {
    /// The renderer accepted the frame.
    Rendered(AvatarRenderReceipt),
    /// The renderer failed and the HUD should show fallback state.
    Fallback(AvatarFallbackState),
}

/// Renders one frame or returns fallback state without blocking the HUD path.
pub fn render_or_fallback<R>(
    renderer: &mut R,
    frame: &AvatarFrame,
    contract: &AvatarRendererContract,
) -> AvatarRenderAttempt
where
    R: AvatarRenderer + ?Sized,
{
    let backend = renderer.backend();
    match renderer.render(frame) {
        Ok(receipt) => AvatarRenderAttempt::Rendered(receipt),
        Err(_) => AvatarRenderAttempt::Fallback(contract.fallback_for_frame(
            backend,
            frame,
            AvatarFallbackReason::RenderError,
        )),
    }
}

/// Renderer error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarRenderError {
    message: String,
}

impl AvatarRenderError {
    /// Creates a renderer error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AvatarRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AvatarRenderError {}

/// Renderer implementation for tests and non-graphical planning.
#[derive(Debug, Default)]
pub struct HeadlessAvatarRenderer {
    frames: Vec<AvatarFrame>,
}

impl HeadlessAvatarRenderer {
    /// Returns all frames rendered by this test renderer.
    pub fn frames(&self) -> &[AvatarFrame] {
        &self.frames
    }
}

impl AvatarRenderer for HeadlessAvatarRenderer {
    fn backend(&self) -> RendererBackend {
        RendererBackend::Headless
    }

    fn render(&mut self, frame: &AvatarFrame) -> Result<AvatarRenderReceipt, AvatarRenderError> {
        self.frames.push(frame.clone());
        Ok(AvatarRenderReceipt {
            backend: self.backend(),
            time_ms: frame.time_ms,
        })
    }
}

/// Stateful avatar engine.
#[derive(Clone, Debug)]
pub struct AvatarEngine {
    config: AvatarEngineConfig,
    last_ms: Option<u64>,
    current_gesture: BodyGesture,
    gesture_started_ms: u64,
}

impl AvatarEngine {
    /// Creates an avatar engine.
    pub fn new(config: AvatarEngineConfig) -> Self {
        Self {
            config,
            last_ms: None,
            current_gesture: BodyGesture::IdleBreath,
            gesture_started_ms: 0,
        }
    }

    /// Returns engine configuration.
    pub fn config(&self) -> &AvatarEngineConfig {
        &self.config
    }

    /// Advances the avatar engine and returns a renderable frame.
    pub fn update(&mut self, input: AvatarInput) -> AvatarFrame {
        let delta_ms = self
            .last_ms
            .map(|last| input.now_ms.saturating_sub(last))
            .unwrap_or(16)
            .min(u64::from(self.config.max_delta_ms));
        self.last_ms = Some(input.now_ms);

        let audio = clamp01(input.audio_level);
        let phase = (input.now_ms as f32 / 1000.0) + (delta_ms as f32 / 1000.0);
        let face = self.face_pose(input.mode, audio, input.face_tracking, phase);
        let body_state = self.body_state(input.mode, audio, input.now_ms);

        AvatarFrame {
            avatar_id: self.config.avatar_id.clone(),
            renderer: self.config.renderer,
            mode: input.mode,
            body_state,
            body: body_pose(body_state, audio, face, phase),
            face,
            material: material_for_mode(input.mode, audio),
            privacy: self.config.privacy,
            time_ms: input.now_ms,
        }
    }

    fn face_pose(
        &self,
        mode: AvatarMode,
        audio: f32,
        face_tracking: Option<FaceTrackingFrame>,
        phase: f32,
    ) -> FacePose {
        if self.config.face_tracking.mode == FaceTrackingMode::LocalOnly {
            if let Some(frame) = face_tracking {
                if self.config.face_tracking.accepts(frame) {
                    return FacePose {
                        gaze_x: clamp_signed(frame.gaze_x),
                        gaze_y: clamp_signed(frame.gaze_y),
                        blink_left: clamp01(frame.blink_left),
                        blink_right: clamp01(frame.blink_right),
                        mouth_open: clamp01(frame.mouth_open),
                        brow_raise: clamp01(frame.brow_raise),
                        tracked: true,
                    };
                }
            }
        }

        synthetic_face_pose(mode, audio, phase)
    }

    fn body_state(&mut self, mode: AvatarMode, audio: f32, now_ms: u64) -> BodyGestureState {
        let (gesture, base_intensity) = gesture_for_mode(mode, audio);
        let interrupted = gesture != self.current_gesture;
        if interrupted {
            self.current_gesture = gesture;
            self.gesture_started_ms = now_ms;
        }

        let intensity = if self.config.reduced_motion {
            clamp01(base_intensity * 0.35).min(0.35)
        } else {
            clamp01(base_intensity)
        };

        BodyGestureState {
            gesture,
            priority: gesture_priority(gesture),
            intensity,
            elapsed_ms: now_ms.saturating_sub(self.gesture_started_ms),
            interrupted,
            reduced_motion: self.config.reduced_motion,
        }
    }
}

fn synthetic_face_pose(mode: AvatarMode, audio: f32, phase: f32) -> FacePose {
    let blink_phase = phase % 4.8;
    let blink = if blink_phase > 4.62 {
        0.18
    } else if blink_phase > 4.50 {
        0.45
    } else {
        1.0
    };
    let active = matches!(mode, AvatarMode::Listening | AvatarMode::Speaking);
    let mouth_open = match mode {
        AvatarMode::Speaking => 0.18 + audio * 0.72 + positive_wave(phase * 9.0) * 0.10,
        AvatarMode::Listening => 0.08 + audio * 0.20,
        _ => 0.04 + positive_wave(phase * 1.6) * 0.03,
    };
    FacePose {
        gaze_x: (phase * if active { 0.84 } else { 0.42 }).sin() * 0.16,
        gaze_y: (phase * 0.33 + 1.0).sin() * 0.08,
        blink_left: blink,
        blink_right: blink,
        mouth_open: clamp01(mouth_open),
        brow_raise: match mode {
            AvatarMode::Thinking => 0.22,
            AvatarMode::Approval | AvatarMode::Error => 0.34,
            _ => 0.08,
        },
        tracked: false,
    }
}

fn gesture_for_mode(mode: AvatarMode, audio: f32) -> (BodyGesture, f32) {
    match mode {
        AvatarMode::Idle => (BodyGesture::IdleBreath, 0.18),
        AvatarMode::Listening => (BodyGesture::AttentiveLean, 0.55),
        AvatarMode::Thinking => (BodyGesture::ThinkingOrbit, 0.48),
        AvatarMode::Speaking => (BodyGesture::SpeakingPulse, 0.52 + audio * 0.42),
        AvatarMode::Coding => (BodyGesture::CodingFocus, 0.58),
        AvatarMode::Approval => (BodyGesture::ApprovalHold, 0.70),
        AvatarMode::Error => (BodyGesture::ErrorAlert, 0.90),
    }
}

fn body_pose(state: BodyGestureState, audio: f32, face: FacePose, phase: f32) -> BodyPose {
    let motion_scale = if state.reduced_motion { 0.25 } else { 1.0 };
    BodyPose {
        gesture: state.gesture,
        intensity: state.intensity,
        head_yaw: face.gaze_x * 0.35 * motion_scale,
        head_pitch: face.gaze_y * 0.30 * motion_scale,
        shoulder_roll: phase.sin() * 0.04 * motion_scale,
        left_hand: match state.gesture {
            BodyGesture::SpeakingPulse | BodyGesture::SpeakingEmphasis => {
                clamp01(0.20 + audio * 0.45)
            }
            BodyGesture::ApprovalHold | BodyGesture::ApprovalHandCue => 0.38,
            BodyGesture::ErrorAlert | BodyGesture::ErrorRecoil => 0.72,
            _ => 0.10,
        } * motion_scale,
        right_hand: match state.gesture {
            BodyGesture::CodingFocus => 0.58,
            BodyGesture::SpeakingPulse | BodyGesture::SpeakingEmphasis => {
                clamp01(0.22 + audio * 0.42)
            }
            BodyGesture::ErrorAlert | BodyGesture::ErrorRecoil => 0.72,
            _ => 0.10,
        } * motion_scale,
    }
}

fn gesture_priority(gesture: BodyGesture) -> BodyGesturePriority {
    match gesture {
        BodyGesture::IdleBreath => BodyGesturePriority::Ambient,
        BodyGesture::ApprovalHold | BodyGesture::ApprovalHandCue => {
            BodyGesturePriority::Interaction
        }
        BodyGesture::ErrorAlert | BodyGesture::ErrorRecoil => BodyGesturePriority::Safety,
        BodyGesture::AttentiveLean
        | BodyGesture::ListeningLean
        | BodyGesture::Nod
        | BodyGesture::GazeShift
        | BodyGesture::ThinkingOrbit
        | BodyGesture::ThinkingScan
        | BodyGesture::SpeakingPulse
        | BodyGesture::SpeakingEmphasis
        | BodyGesture::CodingFocus => BodyGesturePriority::Activity,
    }
}

fn material_for_mode(mode: AvatarMode, audio: f32) -> AvatarMaterial {
    let (primary_rgb, secondary_rgb, glow) = match mode {
        AvatarMode::Idle => ([0.14, 0.82, 1.0], [0.43, 0.34, 1.0], 0.42),
        AvatarMode::Listening => ([0.34, 1.0, 0.94], [0.17, 0.55, 1.0], 0.68),
        AvatarMode::Thinking => ([0.18, 0.66, 1.0], [0.60, 0.38, 1.0], 0.62),
        AvatarMode::Speaking => ([0.55, 0.92, 1.0], [1.0, 0.36, 0.95], 0.74 + audio * 0.20),
        AvatarMode::Coding => ([0.25, 1.0, 0.82], [0.29, 0.45, 1.0], 0.58),
        AvatarMode::Approval => ([1.0, 0.72, 0.26], [0.28, 0.88, 1.0], 0.76),
        AvatarMode::Error => ([1.0, 0.25, 0.54], [1.0, 0.72, 0.16], 0.90),
    };
    AvatarMaterial {
        primary_rgb,
        secondary_rgb,
        glow: clamp01(glow),
        scanline: match mode {
            AvatarMode::Idle => 0.18,
            AvatarMode::Error => 0.58,
            _ => 0.34,
        },
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn clamp_signed(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

fn positive_wave(value: f32) -> f32 {
    (value.sin() + 1.0) * 0.5
}

fn mode_id(mode: AvatarMode) -> u32 {
    match mode {
        AvatarMode::Idle => 0,
        AvatarMode::Listening => 1,
        AvatarMode::Thinking => 2,
        AvatarMode::Speaking => 3,
        AvatarMode::Coding => 4,
        AvatarMode::Approval => 5,
        AvatarMode::Error => 6,
    }
}

fn gesture_id(gesture: BodyGesture) -> u32 {
    match gesture {
        BodyGesture::IdleBreath => 0,
        BodyGesture::AttentiveLean => 1,
        BodyGesture::ListeningLean => 2,
        BodyGesture::Nod => 3,
        BodyGesture::GazeShift => 4,
        BodyGesture::ThinkingOrbit => 5,
        BodyGesture::ThinkingScan => 6,
        BodyGesture::SpeakingPulse => 7,
        BodyGesture::SpeakingEmphasis => 8,
        BodyGesture::CodingFocus => 9,
        BodyGesture::ApprovalHold => 10,
        BodyGesture::ApprovalHandCue => 11,
        BodyGesture::ErrorAlert => 12,
        BodyGesture::ErrorRecoil => 13,
    }
}

fn priority_id(priority: BodyGesturePriority) -> u32 {
    match priority {
        BodyGesturePriority::Ambient => 0,
        BodyGesturePriority::Activity => 1,
        BodyGesturePriority::Interaction => 2,
        BodyGesturePriority::Safety => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_engine_targets_native_wgpu_without_face_tracking() {
        let config = AvatarEngineConfig::default();

        assert_eq!(config.avatar_id, WULAN_AVATAR_ID);
        assert_eq!(config.renderer, RendererBackend::WgpuNative);
        assert_eq!(config.renderer_fallback, AvatarFallbackTarget::CadisOrb);
        assert_eq!(config.face_tracking.mode, FaceTrackingMode::Off);
        assert!(config.privacy.local_only_face_tracking);
        assert!(!config.privacy.persist_raw_face_frames);
        assert!(!config.privacy.persist_face_landmarks);
        assert!(!config.privacy.allow_remote_face_tracking);
        assert!(!config.privacy.allow_face_identity);

        let contract = config.renderer_contract();
        assert_eq!(contract.wgpu.uniform_version, WGPU_AVATAR_UNIFORM_VERSION);
        assert_eq!(contract.fallback.target, AvatarFallbackTarget::CadisOrb);
        assert_eq!(contract.fallback.target.avatar_id(), CADIS_ORB_AVATAR_ID);
        assert!(contract.fallback.preserves_hud_launch);
        assert!(contract.fallback.reason_code_required);
        assert!(contract.fallback.reduced_motion_passthrough);
    }

    #[test]
    fn speaking_mode_selects_body_and_face_motion() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            renderer: RendererBackend::Headless,
            ..AvatarEngineConfig::default()
        });

        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Speaking,
            audio_level: 0.8,
            now_ms: 1_000,
            ..AvatarInput::default()
        });

        assert_eq!(frame.body.gesture, BodyGesture::SpeakingPulse);
        assert_eq!(frame.body_state.gesture, BodyGesture::SpeakingPulse);
        assert_eq!(frame.body_state.priority, BodyGesturePriority::Activity);
        assert!(frame.body.intensity > 0.8);
        assert!(frame.face.mouth_open > 0.5);
        assert!(frame.material.glow > 0.8);
    }

    #[test]
    fn modes_export_expected_body_gestures_and_priorities() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig::default());
        let cases = [
            (
                AvatarMode::Idle,
                BodyGesture::IdleBreath,
                BodyGesturePriority::Ambient,
            ),
            (
                AvatarMode::Listening,
                BodyGesture::AttentiveLean,
                BodyGesturePriority::Activity,
            ),
            (
                AvatarMode::Thinking,
                BodyGesture::ThinkingOrbit,
                BodyGesturePriority::Activity,
            ),
            (
                AvatarMode::Speaking,
                BodyGesture::SpeakingPulse,
                BodyGesturePriority::Activity,
            ),
            (
                AvatarMode::Coding,
                BodyGesture::CodingFocus,
                BodyGesturePriority::Activity,
            ),
            (
                AvatarMode::Approval,
                BodyGesture::ApprovalHold,
                BodyGesturePriority::Interaction,
            ),
            (
                AvatarMode::Error,
                BodyGesture::ErrorAlert,
                BodyGesturePriority::Safety,
            ),
        ];

        for (index, (mode, gesture, priority)) in cases.into_iter().enumerate() {
            let frame = engine.update(AvatarInput {
                mode,
                audio_level: 0.65,
                now_ms: ((index + 1) * 100) as u64,
                ..AvatarInput::default()
            });

            assert_eq!(frame.body_state.gesture, gesture);
            assert_eq!(frame.body.gesture, gesture);
            assert_eq!(frame.body_state.priority, priority);
        }
    }

    #[test]
    fn local_face_tracking_requires_consent_and_confidence() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            face_tracking: FaceTrackingConfig {
                mode: FaceTrackingMode::LocalOnly,
                ..FaceTrackingConfig::default()
            },
            ..AvatarEngineConfig::default()
        });

        let denied = engine.update(AvatarInput {
            face_tracking: Some(FaceTrackingFrame {
                consent: FaceTrackingConsent::Denied,
                gaze_x: 0.9,
                confidence: 1.0,
                ..FaceTrackingFrame::default()
            }),
            now_ms: 10,
            ..AvatarInput::default()
        });
        assert!(!denied.face.tracked);

        let granted = engine.update(AvatarInput {
            face_tracking: Some(FaceTrackingFrame {
                consent: FaceTrackingConsent::GrantedLocalOnly,
                gaze_x: 2.0,
                gaze_y: -2.0,
                mouth_open: 0.7,
                confidence: 0.9,
                ..FaceTrackingFrame::default()
            }),
            now_ms: 20,
            ..AvatarInput::default()
        });
        assert!(granted.face.tracked);
        assert_eq!(granted.face.gaze_x, 1.0);
        assert_eq!(granted.face.gaze_y, -1.0);
        assert_eq!(granted.face.mouth_open, 0.7);
    }

    #[test]
    fn face_tracking_config_rejects_non_local_privacy() {
        let config = AvatarEngineConfig {
            face_tracking: FaceTrackingConfig {
                mode: FaceTrackingMode::LocalOnly,
                ..FaceTrackingConfig::default()
            },
            privacy: AvatarPrivacy {
                allow_remote_face_tracking: true,
                ..AvatarPrivacy::default()
            },
            ..AvatarEngineConfig::default()
        };

        assert_eq!(
            config.validate_privacy(),
            Err(AvatarConfigError::FaceTrackingMustStayLocal)
        );
    }

    #[test]
    fn reduced_motion_limits_body_pose() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            reduced_motion: true,
            ..AvatarEngineConfig::default()
        });

        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Error,
            now_ms: 50,
            ..AvatarInput::default()
        });

        assert_eq!(frame.body_state.priority, BodyGesturePriority::Safety);
        assert!(frame.body_state.reduced_motion);
        assert!(frame.body.intensity <= 0.35);
        assert!(frame.body.left_hand < 0.25);
    }

    #[test]
    fn frame_exports_direct_wgpu_uniforms() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig::default());
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Approval,
            now_ms: 1_250,
            ..AvatarInput::default()
        });

        let uniforms = frame.wgpu_uniforms();

        assert_eq!(uniforms.version, WGPU_AVATAR_UNIFORM_VERSION);
        assert_eq!(uniforms.mode_id, 5);
        assert_eq!(uniforms.gesture_id, 10);
        assert_eq!(uniforms.gesture_priority, 2);
        assert_eq!(uniforms.time_seconds, 1.25);
        assert!(!uniforms.flags.tracked_face);
    }

    #[test]
    fn headless_renderer_records_frames() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            renderer: RendererBackend::Headless,
            ..AvatarEngineConfig::default()
        });
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Coding,
            now_ms: 42,
            ..AvatarInput::default()
        });
        let mut renderer = HeadlessAvatarRenderer::default();

        let receipt = renderer
            .render(&frame)
            .expect("headless render should pass");

        assert_eq!(receipt.backend, RendererBackend::Headless);
        assert_eq!(receipt.time_ms, 42);
        assert_eq!(renderer.frames(), &[frame]);
    }

    #[derive(Debug, Default)]
    struct FailingWgpuRenderer;

    impl AvatarRenderer for FailingWgpuRenderer {
        fn backend(&self) -> RendererBackend {
            RendererBackend::WgpuNative
        }

        fn render(
            &mut self,
            _frame: &AvatarFrame,
        ) -> Result<AvatarRenderReceipt, AvatarRenderError> {
            Err(AvatarRenderError::new("native surface unavailable"))
        }
    }

    #[test]
    fn renderer_error_returns_cadis_orb_fallback_state() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            reduced_motion: true,
            ..AvatarEngineConfig::default()
        });
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Approval,
            now_ms: 777,
            ..AvatarInput::default()
        });
        let contract = engine.config().renderer_contract();
        let mut renderer = FailingWgpuRenderer;

        let attempt = render_or_fallback(&mut renderer, &frame, &contract);

        assert_eq!(
            attempt,
            AvatarRenderAttempt::Fallback(AvatarFallbackState {
                target: AvatarFallbackTarget::CadisOrb,
                reason: AvatarFallbackReason::RenderError,
                failed_backend: RendererBackend::WgpuNative,
                mode: AvatarMode::Approval,
                reduced_motion: true,
                time_ms: 777,
            })
        );
    }

    #[test]
    fn fallback_state_serializes_for_hud_boundaries() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            renderer_fallback: AvatarFallbackTarget::StaticWulanTexture,
            reduced_motion: true,
            ..AvatarEngineConfig::default()
        });
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Error,
            now_ms: 900,
            ..AvatarInput::default()
        });
        let fallback = engine.config().renderer_contract().fallback_for_frame(
            RendererBackend::WgpuNative,
            &frame,
            AvatarFallbackReason::SurfaceLost,
        );

        let json = serde_json::to_string(&fallback).expect("fallback should serialize");

        assert!(json.contains("\"target\":\"static_wulan_texture\""));
        assert!(json.contains("\"reason\":\"surface_lost\""));
        assert!(json.contains("\"failed_backend\":\"wgpu_native\""));
        assert!(json.contains("\"mode\":\"error\""));
        assert!(json.contains("\"reduced_motion\":true"));
    }

    #[test]
    fn frames_serialize_for_renderer_boundaries() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig::default());
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Approval,
            now_ms: 5,
            ..AvatarInput::default()
        });

        let json = serde_json::to_string(&frame).expect("frame should serialize");
        assert!(json.contains("\"mode\":\"approval\""));
        assert!(json.contains("\"renderer\":\"wgpu_native\""));
    }

    #[test]
    fn face_tracking_defaults_to_off_local_only_permission_gated() {
        let config = FaceTrackingConfig::default();

        assert_eq!(config.mode, FaceTrackingMode::Off);
        assert!(config.permission_required);
        assert!(config.camera_indicator_required);
        assert!(config.one_click_disable_required);

        let privacy = AvatarPrivacy::default();
        assert!(privacy.local_only_face_tracking);
        assert!(!privacy.persist_raw_face_frames);
        assert!(!privacy.persist_face_landmarks);
        assert!(!privacy.allow_remote_face_tracking);
        assert!(!privacy.allow_face_identity);

        // Default engine config must pass privacy validation.
        let engine_config = AvatarEngineConfig::default();
        assert!(engine_config.validate_privacy().is_ok());
    }
}
