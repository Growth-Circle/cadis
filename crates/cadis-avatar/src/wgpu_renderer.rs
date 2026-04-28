//! Adapter-ready Rust/wgpu avatar renderer spike.
//!
//! This module is compiled only with the `wgpu-renderer` feature. It deliberately
//! keeps the crate free of a hard `wgpu` dependency: the public shape is a
//! deterministic render plan that a concrete GPU adapter can translate into
//! pipelines, bind groups, buffers, and draw calls.

use serde::{Deserialize, Serialize};

use crate::{
    AvatarFrame, AvatarRenderError, AvatarRenderReceipt, AvatarRenderer, RendererBackend,
    WgpuAlphaModeHint, WgpuAvatarPrimitive, WgpuAvatarRenderer, WgpuAvatarUniforms,
    WgpuRendererContract, WgpuSurfaceTarget, WgpuTextureFormatHint, WGPU_AVATAR_UNIFORM_VERSION,
};

/// Feature-gated renderer identifier for diagnostics and fixtures.
pub const WGPU_SPIKE_RENDERER_ID: &str = "cadis-wulan-wgpu-spike";

/// First shader contract label expected by a concrete direct-wgpu adapter.
pub const WGPU_SPIKE_SHADER_LABEL: &str = "wulan-avatar-spike-v1";

/// CPU-side readiness and budget config for the wgpu renderer spike.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuAvatarRendererConfig {
    /// Target viewport size in physical pixels.
    pub target_size: [u32; 2],
    /// Whether the HUD/native adapter has an available surface or texture.
    pub surface_available: bool,
    /// Whether the Wulan portrait texture has been uploaded or otherwise bound.
    pub portrait_texture_ready: bool,
    /// Alpha cutoff used by the portrait plane.
    pub alpha_cutoff: f32,
    /// Upper particle count budget before reduced-motion handling.
    pub max_particles: u16,
}

impl Default for WgpuAvatarRendererConfig {
    fn default() -> Self {
        Self {
            target_size: [512, 512],
            surface_available: true,
            portrait_texture_ready: true,
            alpha_cutoff: 0.08,
            max_particles: 96,
        }
    }
}

/// Primitive-specific portrait plane state.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuPortraitPlane {
    /// Center position in normalized device coordinates.
    pub center: [f32; 2],
    /// Plane size in normalized device coordinates.
    pub size: [f32; 2],
    /// Alpha cutoff for transparent portrait pixels.
    pub alpha_cutoff: f32,
    /// Tint derived from the avatar material state.
    pub tint_rgb: [f32; 3],
}

/// Primitive-specific reticle and ring state.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuReticleRings {
    /// Number of rings the adapter should draw.
    pub ring_count: u8,
    /// Outer radius in normalized device coordinates.
    pub outer_radius: f32,
    /// Rotation rate in radians per second.
    pub rotation_rate: f32,
    /// Ring opacity after reduced-motion handling.
    pub opacity: f32,
}

/// Primitive-specific particle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WgpuParticleField {
    /// Particle budget for this frame.
    pub count: u16,
    /// Stable seed derived from frame time and mode.
    pub seed: u32,
}

/// Primitive-specific face overlay state.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuFaceOverlay {
    /// Gaze x/y, blink left/right, mouth open, and brow raise.
    pub face: [f32; 6],
    /// Whether face values came from local-only tracking.
    pub tracked_face: bool,
}

/// Primitive-specific body gesture rig state.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuBodyGestureRig {
    /// Head yaw and pitch.
    pub head: [f32; 2],
    /// Left and right hand emphasis.
    pub hands: [f32; 2],
    /// Current gesture intensity after accessibility handling.
    pub intensity: f32,
}

/// Deterministic render plan consumed by a concrete direct-wgpu adapter.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WgpuAvatarRenderPlan {
    /// Renderer spike identifier.
    pub renderer_id: String,
    /// Shader contract label.
    pub shader_label: String,
    /// Uniform payload copied from the renderer-neutral frame.
    pub uniforms: WgpuAvatarUniforms,
    /// Surface target shape.
    pub surface_target: WgpuSurfaceTarget,
    /// Texture format hint.
    pub texture_format: WgpuTextureFormatHint,
    /// Alpha mode hint.
    pub alpha_mode: WgpuAlphaModeHint,
    /// Target viewport size in physical pixels.
    pub target_size: [u32; 2],
    /// Primitive families expected by the adapter.
    pub primitives: Vec<WgpuAvatarPrimitive>,
    /// Portrait plane draw state.
    pub portrait: WgpuPortraitPlane,
    /// Reticle ring draw state.
    pub reticles: WgpuReticleRings,
    /// Particle field draw state.
    pub particles: WgpuParticleField,
    /// Face overlay draw state.
    pub face_overlay: WgpuFaceOverlay,
    /// Body gesture draw state.
    pub body_rig: WgpuBodyGestureRig,
    /// Whether reduced-motion rules were applied to this plan.
    pub reduced_motion: bool,
}

impl WgpuAvatarRenderPlan {
    /// Builds a deterministic adapter plan from a renderer-neutral frame.
    pub fn from_frame(
        frame: &AvatarFrame,
        contract: &WgpuRendererContract,
        config: WgpuAvatarRendererConfig,
    ) -> Result<Self, AvatarRenderError> {
        validate_contract(contract, config)?;

        let uniforms = frame.wgpu_uniforms();
        let reduced_motion = uniforms.flags.reduced_motion;
        let motion_scale = if reduced_motion { 0.25 } else { 1.0 };
        let intensity = uniforms.gesture_intensity;
        let portrait_width = 1.08 + (intensity * 0.06 * motion_scale);
        let portrait_height = 1.42 + (intensity * 0.08 * motion_scale);

        Ok(Self {
            renderer_id: WGPU_SPIKE_RENDERER_ID.to_owned(),
            shader_label: WGPU_SPIKE_SHADER_LABEL.to_owned(),
            uniforms,
            surface_target: contract.target,
            texture_format: contract.texture_format,
            alpha_mode: contract.alpha_mode,
            target_size: config.target_size,
            primitives: contract.primitives.clone(),
            portrait: WgpuPortraitPlane {
                center: [uniforms.head[0] * 0.035 * motion_scale, -0.03],
                size: [portrait_width, portrait_height],
                alpha_cutoff: config.alpha_cutoff.clamp(0.0, 1.0),
                tint_rgb: frame.material.primary_rgb,
            },
            reticles: WgpuReticleRings {
                ring_count: 2,
                outer_radius: 0.73,
                rotation_rate: reticle_rotation_rate(uniforms.mode_id, intensity, reduced_motion),
                opacity: if reduced_motion {
                    0.28
                } else {
                    0.52 + intensity * 0.20
                },
            },
            particles: WgpuParticleField {
                count: particle_count(uniforms.mode_id, config.max_particles, reduced_motion),
                seed: particle_seed(frame.time_ms, uniforms.mode_id),
            },
            face_overlay: WgpuFaceOverlay {
                face: uniforms.face,
                tracked_face: uniforms.flags.tracked_face,
            },
            body_rig: WgpuBodyGestureRig {
                head: uniforms.head,
                hands: if reduced_motion {
                    [uniforms.hands[0] * 0.35, uniforms.hands[1] * 0.35]
                } else {
                    uniforms.hands
                },
                intensity,
            },
            reduced_motion,
        })
    }
}

/// Placeholder for the Wulan portrait shader pipeline.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WgpuPortraitShader {
    /// Shader label for diagnostics.
    pub label: String,
    /// Alpha cutoff for transparent portrait pixels.
    pub alpha_cutoff: f32,
    /// Whether the shader is loaded and ready.
    pub ready: bool,
}

/// Placeholder for the Wulan particle system.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WgpuParticleSystem {
    /// Maximum particle budget.
    pub max_particles: u16,
    /// Particle lifetime in milliseconds.
    pub lifetime_ms: u32,
    /// Whether reduced-motion rules apply.
    pub reduced_motion: bool,
}

/// Placeholder for the Wulan reticle renderer.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WgpuReticleRenderer {
    /// Number of concentric rings.
    pub ring_count: u8,
    /// Outer radius in NDC.
    pub outer_radius: f32,
    /// Base rotation rate in radians per second.
    pub rotation_rate: f32,
}

/// Placeholder for the Wulan eye overlay.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WgpuEyeOverlay {
    /// Gaze target x in -1..1.
    pub gaze_x: f32,
    /// Gaze target y in -1..1.
    pub gaze_y: f32,
    /// Left eyelid openness in 0..1.
    pub blink_left: f32,
    /// Right eyelid openness in 0..1.
    pub blink_right: f32,
}

/// Placeholder for the Wulan mouth overlay.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct WgpuMouthOverlay {
    /// Mouth openness in 0..1.
    pub mouth_open: f32,
    /// Whether mouth is driven by speech audio.
    pub speech_driven: bool,
}

/// Feature-gated native wgpu renderer spike.
#[derive(Clone, Debug)]
pub struct WgpuAvatarSpikeRenderer {
    contract: WgpuRendererContract,
    config: WgpuAvatarRendererConfig,
    last_plan: Option<WgpuAvatarRenderPlan>,
}

impl WgpuAvatarSpikeRenderer {
    /// Creates a renderer spike using the default direct-wgpu contract.
    pub fn new(config: WgpuAvatarRendererConfig) -> Self {
        Self::with_contract(WgpuRendererContract::default(), config)
    }

    /// Creates a renderer spike with an explicit contract.
    pub fn with_contract(contract: WgpuRendererContract, config: WgpuAvatarRendererConfig) -> Self {
        Self {
            contract,
            config,
            last_plan: None,
        }
    }

    /// Returns the last adapter plan accepted by this renderer.
    pub fn last_plan(&self) -> Option<&WgpuAvatarRenderPlan> {
        self.last_plan.as_ref()
    }

    /// Converts an [`AvatarFrame`] into a deterministic render plan.
    ///
    /// This is the connection point between the renderer-neutral avatar state
    /// engine and the direct-wgpu rendering path.
    pub fn render_frame(
        &mut self,
        frame: &AvatarFrame,
    ) -> Result<WgpuAvatarRenderPlan, AvatarRenderError> {
        let plan = WgpuAvatarRenderPlan::from_frame(frame, &self.contract, self.config)?;
        self.last_plan = Some(plan.clone());
        Ok(plan)
    }
}

impl Default for WgpuAvatarSpikeRenderer {
    fn default() -> Self {
        Self::new(WgpuAvatarRendererConfig::default())
    }
}

impl AvatarRenderer for WgpuAvatarSpikeRenderer {
    fn backend(&self) -> RendererBackend {
        RendererBackend::WgpuNative
    }

    fn render(&mut self, frame: &AvatarFrame) -> Result<AvatarRenderReceipt, AvatarRenderError> {
        let plan = WgpuAvatarRenderPlan::from_frame(frame, &self.contract, self.config)?;
        self.last_plan = Some(plan);
        Ok(AvatarRenderReceipt {
            backend: self.backend(),
            time_ms: frame.time_ms,
        })
    }
}

impl WgpuAvatarRenderer for WgpuAvatarSpikeRenderer {
    fn wgpu_contract(&self) -> WgpuRendererContract {
        self.contract.clone()
    }
}

fn validate_contract(
    contract: &WgpuRendererContract,
    config: WgpuAvatarRendererConfig,
) -> Result<(), AvatarRenderError> {
    if contract.uniform_version != WGPU_AVATAR_UNIFORM_VERSION {
        return Err(AvatarRenderError::new(
            "wgpu avatar uniform contract version mismatch",
        ));
    }
    if contract.requires_camera {
        return Err(AvatarRenderError::new(
            "wgpu avatar renderer must not require camera access",
        ));
    }
    if config.target_size[0] == 0 || config.target_size[1] == 0 {
        return Err(AvatarRenderError::new(
            "wgpu avatar target size must be non-zero",
        ));
    }
    if !config.surface_available {
        return Err(AvatarRenderError::new("wgpu avatar surface lost"));
    }
    if !config.portrait_texture_ready {
        return Err(AvatarRenderError::new(
            "wulan portrait texture is not ready",
        ));
    }
    Ok(())
}

fn reticle_rotation_rate(mode_id: u32, intensity: f32, reduced_motion: bool) -> f32 {
    if reduced_motion {
        return 0.0;
    }

    match mode_id {
        2 => 0.90 + intensity * 0.45,
        3 => 0.55 + intensity * 0.30,
        4 => 0.36,
        5 => 0.18,
        6 => 1.25,
        _ => 0.24,
    }
}

fn particle_count(mode_id: u32, max_particles: u16, reduced_motion: bool) -> u16 {
    let desired = match mode_id {
        1 => 72,
        2 => 96,
        3 => 88,
        4 => 80,
        5 => 64,
        6 => 48,
        _ => 44,
    };
    let capped = desired.min(max_particles);
    if reduced_motion {
        capped.min(24)
    } else {
        capped
    }
}

fn particle_seed(time_ms: u64, mode_id: u32) -> u32 {
    let low = time_ms as u32;
    low.rotate_left(7) ^ mode_id.wrapping_mul(0x9e37)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        render_or_fallback, AvatarEngine, AvatarEngineConfig, AvatarFallbackReason,
        AvatarFallbackState, AvatarFallbackTarget, AvatarInput, AvatarMode, AvatarRenderAttempt,
        BodyGesturePriority,
    };

    #[test]
    fn spike_renderer_accepts_wgpu_frames_and_records_plan() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig::default());
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Thinking,
            now_ms: 2_000,
            ..AvatarInput::default()
        });
        let mut renderer = WgpuAvatarSpikeRenderer::default();

        let receipt = renderer.render(&frame).expect("wgpu plan should render");
        let plan = renderer.last_plan().expect("render should store plan");

        assert_eq!(receipt.backend, RendererBackend::WgpuNative);
        assert_eq!(receipt.time_ms, 2_000);
        assert_eq!(plan.renderer_id, WGPU_SPIKE_RENDERER_ID);
        assert_eq!(plan.uniforms.mode_id, 2);
        assert_eq!(plan.reticles.ring_count, 2);
        assert!(plan.reticles.rotation_rate > 0.0);
        assert!(plan.particles.count > 24);
    }

    #[test]
    fn reduced_motion_limits_dynamic_wgpu_primitives() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            reduced_motion: true,
            ..AvatarEngineConfig::default()
        });
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Error,
            now_ms: 333,
            ..AvatarInput::default()
        });
        let plan = WgpuAvatarRenderPlan::from_frame(
            &frame,
            &WgpuRendererContract::default(),
            WgpuAvatarRendererConfig::default(),
        )
        .expect("reduced motion frame should plan");

        assert!(plan.reduced_motion);
        assert_eq!(plan.reticles.rotation_rate, 0.0);
        assert!(plan.reticles.opacity <= 0.28);
        assert!(plan.particles.count <= 24);
        assert!(plan.body_rig.hands[0] < frame.body.left_hand);
        assert_eq!(frame.body_state.priority, BodyGesturePriority::Safety);
    }

    #[test]
    fn unavailable_surface_uses_cadis_orb_fallback() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig {
            reduced_motion: true,
            ..AvatarEngineConfig::default()
        });
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Approval,
            now_ms: 444,
            ..AvatarInput::default()
        });
        let contract = engine.config().renderer_contract();
        let mut renderer = WgpuAvatarSpikeRenderer::new(WgpuAvatarRendererConfig {
            surface_available: false,
            ..WgpuAvatarRendererConfig::default()
        });

        let attempt = render_or_fallback(&mut renderer, &frame, &contract);

        assert_eq!(
            attempt,
            AvatarRenderAttempt::Fallback(AvatarFallbackState {
                target: AvatarFallbackTarget::CadisOrb,
                reason: AvatarFallbackReason::RenderError,
                failed_backend: RendererBackend::WgpuNative,
                mode: AvatarMode::Approval,
                reduced_motion: true,
                time_ms: 444,
            })
        );
        assert!(renderer.last_plan().is_none());
    }

    #[test]
    fn render_frame_converts_avatar_frame_to_plan() {
        let mut engine = AvatarEngine::new(AvatarEngineConfig::default());
        let frame = engine.update(AvatarInput {
            mode: AvatarMode::Speaking,
            audio_level: 0.6,
            now_ms: 500,
            ..AvatarInput::default()
        });
        let mut renderer = WgpuAvatarSpikeRenderer::default();

        let plan = renderer
            .render_frame(&frame)
            .expect("render_frame should produce a plan");

        assert_eq!(plan.renderer_id, WGPU_SPIKE_RENDERER_ID);
        assert_eq!(plan.uniforms.mode_id, 3);
        assert!(plan.particles.count > 0);
        assert!(renderer.last_plan().is_some());
    }
}
