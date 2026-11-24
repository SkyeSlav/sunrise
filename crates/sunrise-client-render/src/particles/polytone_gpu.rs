//! How resource packs' Polytone particles are drawn (#11486): the rotation modes of custom
//! particles, and GPU particles (wiki "GPU Particles"). A GPU particle never ticks: its spawn is a
//! record (position, velocity, spawn time, lifetime, seed, colour, size, roll), and where it is,
//! how big, what colour and which frame are a function of that record and the time. Here the
//! function runs when the frame's particle quads are built, beside the engine's own particles.
//! A type with its own `shader` instead hands its records to that shader ([`ShaderBatch`]),
//! which computes the quads on the graphics card as the wiki's 1.21.11 format describes. The
//! renderer decides each frame, before the quads are built, whether it draws the shader
//! (`Renderer::decide_polytone_shaders`); a type whose shader it can't draw keeps its quads here.
use super::{ParticleLevel, ParticleLightCache};
use crate::hud::Rect;
use crate::mesher::Vertex;
use glam::{DVec3, Mat3, Quat, Vec3 as FVec3};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use sunrise_polytone::gpu_particle::{
    BlockFrame, GPU_BLOCK_SIZE, HeightmapArea, Surface, gpu_block,
};
use sunrise_polytone::particles::{CustomParticle, Gpu, RenderType};

/// How a particle quad turns (`rotation_mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Orientation {
    /// `look_at_xyz`: faces the camera (what every other mode falls back to).
    #[default]
    Camera,
    /// `look_at_y`: turns around Y only.
    AroundY,
    /// `look_up`: lies flat, facing up.
    Flat,
    /// `movement_aligned`: stands along its velocity, facing the camera around it.
    Movement,
    /// `none`: fixed in the world's axes (its +Z facing south).
    Fixed,
}

impl Orientation {
    pub(super) fn parse(mode: &str) -> Self {
        match mode {
            "look_at_y" => Self::AroundY,
            "look_up" => Self::Flat,
            "movement_aligned" => Self::Movement,
            "none" => Self::Fixed,
            _ => Self::Camera,
        }
    }
}

/// The rotation of a quad (corners in its XY plane, facing +Z) at `center` (relative to the
/// camera) moving along `velocity`, rolled by `roll`; `camera` is the camera-facing rotation.
pub(super) fn orient(
    mode: Orientation,
    center: FVec3,
    velocity: FVec3,
    roll: f32,
    camera: Quat,
) -> Quat {
    let rolled = |q: Quat| q * Quat::from_rotation_z(roll);
    match mode {
        Orientation::Camera => rolled(camera),
        Orientation::AroundY => rolled(Quat::from_rotation_y((-center.x).atan2(-center.z))),
        Orientation::Flat => rolled(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Orientation::Fixed => rolled(Quat::IDENTITY),
        Orientation::Movement => {
            let up = velocity.normalize_or_zero();
            let right = up.cross((-center).normalize_or_zero()).normalize_or_zero();
            if right == FVec3::ZERO {
                return rolled(camera);
            }
            Quat::from_mat3(&Mat3::from_cols(right, up, right.cross(up)))
        }
    }
}

/// The alpha a particle keeps at `life` (0 to 1 of its lifetime) with `fade_in` and `fade_out`
/// fractions of it spent fading: each ramp eases in and out (smoothstep), as Polytone's are
/// measured to (a linear ramp leaves young rain drops twice as bright).
fn fade(life: f32, fade_in: f32, fade_out: f32) -> f32 {
    let ease = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let mut alpha = 1.0;
    if fade_in > 0.0 {
        alpha *= ease(life / fade_in);
    }
    if fade_out > 0.0 {
        alpha *= ease((1.0 - life) / fade_out);
    }
    alpha
}

/// One GPU particle spawn.
#[derive(Clone, Copy, Debug)]
struct Record {
    position: [f64; 3],
    velocity: [f64; 3],
    /// The engine's tick it spawned on, and its life in ticks.
    spawn: u64,
    lifetime: f32,
    seed: u64,
    rgba: [f32; 4],
    size: f32,
    roll: f32,
    /// The initializer's `custom`, for a custom shader (`Params.z`).
    custom: f32,
}

/// A GPU particle type's records: a ring of `limit` slots, the oldest overwritten.
pub(super) struct Store {
    def: Arc<CustomParticle>,
    gpu: Gpu,
    orientation: Orientation,
    /// The texture's sprite name in the particle atlas (`ns:particle/path`).
    sprite: String,
    records: Vec<Record>,
    next: usize,
    limit: usize,
    /// The type's own shader, when it has one that translated.
    shader: Option<Arc<GpuShader>>,
}

/// The atlas name of a full texture path: `ns:textures/particle/a/b.png` is `ns:particle/a/b`
/// (`particle/a/b` in `minecraft`).
fn sprite_name(texture: &str) -> String {
    let (namespace, path) = texture.split_once(':').unwrap_or(("minecraft", texture));
    let path = path
        .trim_start_matches("textures/")
        .trim_end_matches(".png");
    if namespace == "minecraft" {
        path.to_owned()
    } else {
        format!("{namespace}:{path}")
    }
}

/// SplitMix64: a well-mixed number from `seed` and `i`, for each quad's own randoms.
fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number in 0..1 from `mix`.
fn unit(seed: u64, i: u64) -> f64 {
    (mix(seed, i) >> 11) as f64 / (1u64 << 53) as f64
}

/// Where a record's particle is after `t` ticks, and how fast it moves: gravity pulls down,
/// friction (`friction` kept per tick, as a continuous drag) slows it toward a terminal velocity.
fn motion(gpu: &Gpu, position: [f64; 3], velocity: [f64; 3], t: f64) -> ([f64; 3], [f64; 3]) {
    let drag = if gpu.friction > 0.0 && gpu.friction < 1.0 {
        -gpu.friction.ln()
    } else {
        0.0
    };
    let gravity = [0.0, -gpu.gravity, 0.0];
    let mut at = [0.0; 3];
    let mut moving = [0.0; 3];
    for i in 0..3 {
        let (v, g) = (velocity[i], gravity[i]);
        if drag > 1.0e-9 {
            let e = (-drag * t).exp();
            at[i] = position[i] + v * (1.0 - e) / drag + g * (t / drag - (1.0 - e) / (drag * drag));
            moving[i] = v * e + g * (1.0 - e) / drag;
        } else {
            at[i] = position[i] + v * t + 0.5 * g * t * t;
            moving[i] = v + g * t;
        }
    }
    (at, moving)
}

impl Store {
    pub(super) fn new(def: &Arc<CustomParticle>, shader: Option<Arc<GpuShader>>) -> Option<Self> {
        let gpu = def.gpu.clone()?;
        Some(Self {
            def: def.clone(),
            orientation: Orientation::parse(&def.rotation_mode),
            sprite: sprite_name(&gpu.texture),
            gpu,
            records: Vec::new(),
            next: 0,
            // The wiki's default ring capacity.
            limit: def.limit.map_or(16_384, |l| l.max(1) as usize),
            shader,
        })
    }

    pub(super) fn is(&self, def: &Arc<CustomParticle>) -> bool {
        Arc::ptr_eq(&self.def, def)
    }

    /// Records a spawn from a particle its initializer set up (position, motion, life, look).
    pub(super) fn spawn(&mut self, p: &super::Particle, custom: f64, now: u64, seed: u64) {
        if p.lifetime <= 0 {
            return;
        }
        let record = Record {
            position: p.position,
            velocity: p.velocity,
            spawn: now,
            lifetime: p.lifetime as f32,
            seed,
            rgba: p.rgba,
            size: p.size,
            roll: p.roll,
            custom: custom as f32,
        };
        if self.records.len() < self.limit {
            self.records.push(record);
        } else {
            self.records[self.next] = record;
            self.next = (self.next + 1) % self.limit;
        }
    }

    /// Forgets the records whose life is over.
    pub(super) fn expire(&mut self, now: u64) {
        let before = self.records.len();
        self.records
            .retain(|r| (now.saturating_sub(r.spawn) as f32) < r.lifetime);
        if self.records.len() != before {
            self.next = 0;
        }
    }

    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    /// The quads of every live record at `now + frame.partial` ticks; none while the renderer
    /// draws the type with its own shader.
    pub(super) fn render(
        &self,
        frame: &QuadFrame,
        now: u64,
        light_cache: &mut ParticleLightCache,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        let QuadFrame {
            atlas,
            camera,
            facing,
            partial,
            light_at,
            surface,
        } = *frame;
        if self.def.render_type == RenderType::Invisible
            || self.shader.as_ref().is_some_and(|s| s.drawn())
        {
            return;
        }
        let Some(&(rect, _)) = atlas.get(&self.sprite) else {
            return;
        };
        let gpu = &self.gpu;
        let translucent = matches!(
            self.def.render_type,
            RenderType::Translucent | RenderType::AdditiveTranslucent
        );
        let blend_add = self.def.render_type == RenderType::AdditiveTranslucent;
        let target = if blend_add { additive } else { out };
        let layer = if translucent && !blend_add {
            crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY
        } else {
            0
        };
        let (count, area) = gpu.area.map_or((1, [0.0; 3]), |(size, n)| (n, size));
        let frames = gpu.frames.max(1);
        for r in &self.records {
            let t = (now.saturating_sub(r.spawn)) as f32 + partial;
            if t >= r.lifetime {
                continue;
            }
            let life = (t / r.lifetime).clamp(0.0, 1.0);
            let (at, moving) = motion(gpu, r.position, r.velocity, f64::from(t));
            // Fading in and out, and the colour and size toward their ends.
            let [fade_in, fade_out] = gpu.fade.map(|f| f as f32);
            let mut alpha = r.rgba[3];
            alpha *= fade(life, fade_in, fade_out);
            let mut rgb = [r.rgba[0], r.rgba[1], r.rgba[2]];
            if let Some(end) = gpu.color_end {
                let channel = |shift: u32| ((end >> shift) & 0xff) as f32 / 255.0;
                let to = [channel(16), channel(8), channel(0)];
                for i in 0..3 {
                    rgb[i] += (to[i] - rgb[i]) * life;
                }
                alpha += (alpha * channel(24) - alpha) * life;
            }
            let size = gpu
                .size_end
                .map_or(r.size, |end| r.size + (end as f32 - r.size) * life);
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u32;
            let color = c(alpha) << 24 | c(rgb[2]) << 16 | c(rgb[1]) << 8 | c(rgb[0]);
            let frame = if self.def.random_sprite {
                (mix(r.seed, 0) % u64::from(frames)) as u32
            } else {
                ((life * frames as f32) as u32).min(frames - 1)
            };
            let (dv, v0) = ((rect.v1 - rect.v0) / frames as f32, rect.v0);
            let (u0, u1) = (rect.u0, rect.u1);
            let (v0, v1) = (v0 + dv * frame as f32, v0 + dv * (frame + 1) as f32);
            let velocity = FVec3::new(moving[0] as f32, moving[1] as f32, moving[2] as f32);
            for quad in 0..u64::from(count) {
                // Each quad of an area has its own place in the box, sway phase and roll.
                let offset: [f64; 3] = if count > 1 {
                    std::array::from_fn(|i| (unit(r.seed, quad * 8 + i as u64 + 1) - 0.5) * area[i])
                } else {
                    [0.0; 3]
                };
                let phase = unit(r.seed, quad * 8 + 4) * std::f64::consts::TAU;
                let sway = gpu.sway * (f64::from(t) * 0.1 + phase).sin();
                let sway_z = gpu.sway * (f64::from(t) * 0.07 + phase).cos();
                let world = [
                    at[0] + offset[0] + sway,
                    at[1] + offset[1],
                    at[2] + offset[2] + sway_z,
                ];
                // kill_below_heightmap: under the motion-blocking surface around the camera.
                if gpu.kill_below_heightmap && surface.is_some_and(|s| s.covers(world)) {
                    continue;
                }
                let block = ParticleLevel::block_at(world);
                let packed = light_cache.get(block, light_at);
                let block_light = u32::from((packed & 15).max(self.def.light_level));
                let light = (u32::from(packed >> 4) * 16) | ((block_light * 16) << 8);
                let center = (DVec3::from_array(world) - camera).as_vec3();
                let roll = r.roll
                    + gpu.spin as f32 * t
                    + if count > 1 {
                        (unit(r.seed, quad * 8 + 5) * std::f64::consts::TAU) as f32
                    } else {
                        0.0
                    };
                let rotation = orient(self.orientation, center, velocity, roll, facing);
                let half = FVec3::new(size, size * gpu.aspect as f32, 0.0);
                for (n, uv) in [
                    ((1.0, -1.0), [u1, v1]),
                    ((1.0, 1.0), [u1, v0]),
                    ((-1.0, 1.0), [u0, v0]),
                    ((-1.0, -1.0), [u0, v1]),
                ] {
                    let corner = FVec3::new(n.0 * half.x, n.1 * half.y, 0.0);
                    target.push(Vertex {
                        normal: [0; 4],
                        pos: (center + rotation * corner).to_array(),
                        uv,
                        layer,
                        color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
}

/// A GPU particle type's own shader (`shader`): its program, texture and `uniforms`, and
/// whether the renderer draws with it.
pub struct GpuShader {
    /// The shader's id (`ns:name`), for the log.
    pub id: String,
    pub program: Arc<crate::core_shader::CoreProgram>,
    /// `Sampler0`: the type's `texture` (width, height, RGBA pixels).
    pub texture: Option<(u32, u32, Arc<[u8]>)>,
    pub render_type: RenderType,
    /// Each uniform the pack names, set every frame from its expression.
    pub uniforms: Vec<(String, sunrise_polytone::expr::Program)>,
    /// Whether a stage reads `Sampler1`, the heightmap.
    pub samples_heightmap: bool,
    /// [`UNTRIED`], [`DRAWN`] or [`REFUSED`]: what the renderer made of it.
    state: AtomicU8,
}

/// No renderer decided yet (none draws, or the type just appeared): the type draws its quads.
const UNTRIED: u8 = 0;
/// The renderer draws the type with its shader in this frame: the type draws no quads.
const DRAWN: u8 = 1;
/// The shader doesn't build, or can't draw into this frame: the type draws its quads.
const REFUSED: u8 = 2;

impl GpuShader {
    pub fn new(
        id: String,
        program: Arc<crate::core_shader::CoreProgram>,
        texture: Option<(u32, u32, Arc<[u8]>)>,
        render_type: RenderType,
        uniforms: Vec<(String, sunrise_polytone::expr::Program)>,
    ) -> Self {
        let samples_heightmap = program.reads_binding(crate::core_shader::SAMPLERS[1].1);
        Self {
            id,
            program,
            texture,
            render_type,
            uniforms,
            samples_heightmap,
            state: AtomicU8::new(UNTRIED),
        }
    }

    /// Whether the renderer draws the type with this shader (its quads are left out).
    pub fn drawn(&self) -> bool {
        self.state.load(Ordering::Relaxed) == DRAWN
    }

    /// Whether the renderer can't use this shader (the type draws its quads, and no batch).
    pub fn refused(&self) -> bool {
        self.state.load(Ordering::Relaxed) == REFUSED
    }

    /// The renderer's verdict for this frame, made before the quads are built: it draws the
    /// type with this shader, or it can't.
    pub(crate) fn set_drawn(&self, drawn: bool) {
        let state = if drawn { DRAWN } else { REFUSED };
        self.state.store(state, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for GpuShader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuShader").field("id", &self.id).finish()
    }
}

/// What a custom shader's vertex stage reads (wiki "GPU Particles", 1.21.11 and up): the
/// record, the same on the quad's four corners (`gl_VertexIndex & 3` is the corner).
pub const GLUE: crate::core_shader::Glue = crate::core_shader::Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Velocity", "vec3"),
        ("SpawnLife", "vec2"),
        ("Params", "vec4"),
        ("Color", "vec4"),
        ("UV2", "ivec2"),
    ],
    prelude: "layout(location = 0) in vec3 hz_position;\n\
              layout(location = 1) in vec3 hz_velocity;\n\
              layout(location = 2) in vec2 hz_spawn_life;\n\
              layout(location = 3) in vec4 hz_params;\n\
              layout(location = 4) in vec4 hz_color;\n\
              layout(location = 5) in ivec2 hz_uv2;",
    setup: "Position = hz_position; Velocity = hz_velocity; SpawnLife = hz_spawn_life; \
            Params = hz_params; Color = hz_color; UV2 = hz_uv2;",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

/// One corner of a record for a custom shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderVertex {
    /// Where it spawned, relative to the batch's `Origin`.
    pub position: [f32; 3],
    /// Its starting velocity, blocks per tick.
    pub velocity: [f32; 3],
    /// The tick it spawned on (on the batch's `Time` clock) and its life in ticks.
    pub spawn_life: [f32; 2],
    /// Size (half-width), roll, `custom`, a random seed in 0..1.
    pub params: [f32; 4],
    /// RGBA.
    pub color: [u8; 4],
    /// The light where it spawned, as vanilla's `UV2`: block light and sky light, times 16.
    pub uv2: [i32; 2],
    pad: u32,
}

/// A type's live records for its own shader, with the block and uniforms of this frame.
#[derive(Debug)]
pub struct ShaderBatch {
    pub shader: Arc<GpuShader>,
    /// Four corners a record, the corners of an `area`'s copies after each other.
    pub vertices: Vec<ShaderVertex>,
    pub block: [u8; GPU_BLOCK_SIZE],
    pub uniforms: Vec<(String, f32)>,
}

impl Orientation {
    /// The `Billboard` a custom shader reads: 0 facing the camera, 1 around Y, 2 flat, 3 along
    /// its velocity.
    fn billboard(self) -> i32 {
        match self {
            Self::Camera | Self::Fixed => 0,
            Self::AroundY => 1,
            Self::Flat => 2,
            Self::Movement => 3,
        }
    }
}

/// The most quads a frame hands the renderer for one type: a type's ring of records times its
/// `area`'s copies is the pack's to choose.
const MAX_SHADER_QUADS: usize = 1 << 20;

/// What a frame's quads are built from.
#[derive(Clone, Copy)]
pub(super) struct QuadFrame<'a> {
    /// The particle atlas's sprites.
    pub atlas: &'a HashMap<String, (Rect, bool)>,
    pub camera: DVec3,
    /// The camera-facing rotation.
    pub facing: Quat,
    pub partial: f32,
    pub light_at: &'a dyn Fn([i32; 3]) -> u8,
    /// The heights `kill_below_heightmap` types die under, taken before the quads.
    pub surface: Option<&'a Surface>,
}

/// What a frame's batches are built from.
pub(super) struct BatchFrame<'a> {
    pub camera: DVec3,
    /// The engine's tick and the partial tick drawn.
    pub now: u64,
    pub partial: f32,
    /// The heightmap `Sampler1` holds.
    pub heightmap: Option<HeightmapArea>,
    pub light_at: &'a dyn Fn([i32; 3]) -> u8,
}

impl Store {
    /// The type's own shader, unless the renderer refused it.
    fn usable_shader(&self) -> Option<&Arc<GpuShader>> {
        self.shader.as_ref().filter(|s| !s.refused())
    }

    /// The live records for the type's own shader in `frame`, each repeated for the copies of
    /// its `area` on its own place (the shader spreads them, by `gl_VertexIndex`, as the default
    /// shader does). `None` for a type without one, or one the renderer doesn't draw.
    pub(super) fn shader_batch(
        &self,
        frame: &BatchFrame,
        light_cache: &mut ParticleLightCache,
        uniforms: Vec<(String, f32)>,
    ) -> Option<ShaderBatch> {
        let shader = self.shader.clone().filter(|s| s.drawn())?;
        if self.def.render_type == RenderType::Invisible {
            return None;
        }
        let camera = frame.camera;
        // Positions relative to the camera's block, so they stay exact as floats; the clock
        // from a recent multiple of 65536 ticks, for the same reason.
        let anchor = camera.floor();
        let base = frame.now & !0xFFFF;
        let time = (frame.now - base) as f32 + frame.partial;
        // An `area`'s copies stack on the record (measured on the real mod: nine copies of an
        // 8-block area draw on one spot, three additive ones three times as bright).
        let copies = self.gpu.area.map_or(1, |(_, n)| n as usize);
        let mut vertices =
            Vec::with_capacity((self.records.len() * copies).min(MAX_SHADER_QUADS) * 4);
        for r in &self.records {
            let spawn = (r.spawn as i64 - base as i64) as f32;
            if time - spawn >= r.lifetime {
                continue;
            }
            if vertices.len() / 4 + copies > MAX_SHADER_QUADS {
                break;
            }
            let world = DVec3::from_array(r.position);
            let packed = light_cache.get(ParticleLevel::block_at(r.position), frame.light_at);
            let block_light = i32::from((packed & 15).max(self.def.light_level));
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let vertex = ShaderVertex {
                position: (world - anchor).as_vec3().to_array(),
                velocity: r.velocity.map(|v| v as f32),
                spawn_life: [spawn, r.lifetime],
                params: [r.size, r.roll, r.custom, unit(r.seed, 6) as f32],
                color: r.rgba.map(c),
                uv2: [block_light * 16, i32::from(packed >> 4) * 16],
                pad: 0,
            };
            for _ in 0..copies {
                vertices.extend([vertex; 4]);
            }
        }
        let block = gpu_block(
            &self.gpu,
            self.def.render_type,
            &BlockFrame {
                billboard: self.orientation.billboard(),
                random_sprite: self.def.random_sprite,
                origin: (anchor - camera).as_vec3().to_array(),
                time,
                camera: camera.to_array(),
                heightmap: frame.heightmap,
            },
        );
        Some(ShaderBatch {
            shader,
            vertices,
            block,
            uniforms,
        })
    }
}

/// The packs' GPU particles: each type's records, the ticks they count in and the heightmap
/// they read.
#[derive(Default)]
pub(super) struct GpuParticles {
    stores: Vec<Store>,
    ticks: u64,
    /// The heightmap last asked for (`ParticleEngine::polytone_heightmap_due`), and its tick.
    heightmap: Option<(HeightmapArea, u64)>,
    /// Its heights, for the quads of the types that die under it.
    surface: Option<Surface>,
}

/// A heightmap to fill now: where, and whether a shader samples it (else only the quads drawn
/// here read its heights).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeightmapRefresh {
    pub area: HeightmapArea,
    pub for_shaders: bool,
}

impl GpuParticles {
    /// Records a spawn of type `def` (its shader `shader`, when it has one that translated).
    pub(super) fn spawn(
        &mut self,
        def: &Arc<CustomParticle>,
        shader: impl FnOnce() -> Option<Arc<GpuShader>>,
        particle: &super::Particle,
        custom: f64,
        seed: u64,
    ) {
        let now = self.ticks;
        let store = match self.stores.iter().position(|s| s.is(def)) {
            Some(i) => &mut self.stores[i],
            None => {
                let Some(store) = Store::new(def, shader()) else {
                    return;
                };
                self.stores.push(store);
                let last = self.stores.len() - 1;
                &mut self.stores[last]
            }
        };
        store.spawn(particle, custom, now, seed);
    }

    /// A tick: the records whose life is over go.
    pub(super) fn tick(&mut self) {
        self.ticks += 1;
        for store in &mut self.stores {
            store.expire(self.ticks);
        }
    }

    /// Forgets every record and the heightmap, as the engine's other particles are cleared (a
    /// mod's clear, a level left or switched).
    pub(super) fn clear(&mut self) {
        for store in &mut self.stores {
            store.records.clear();
            store.next = 0;
        }
        self.heightmap = None;
        self.surface = None;
    }

    /// The heights the quads of the types that die under the heightmap read.
    pub(super) fn surface(&self) -> Option<&Surface> {
        self.surface.as_ref()
    }

    pub(super) fn records(&self) -> usize {
        self.stores.iter().map(Store::len).sum()
    }

    /// The quads of the types drawn here (no shader of their own, or one the renderer doesn't
    /// draw this frame), into `out` and `additive`.
    pub(super) fn render(
        &self,
        frame: &QuadFrame,
        light_cache: &mut ParticleLightCache,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        for store in &self.stores {
            store.render(frame, self.ticks, light_cache, out, additive);
        }
    }
}

/// The GPU particle types drawn with their own shaders.
impl super::ParticleEngine {
    /// The shaders of the GPU particle types with live records, for the renderer to decide
    /// on before the frame's quads are built.
    pub fn polytone_shaders(&self) -> Vec<Arc<GpuShader>> {
        self.polytone_gpu
            .stores
            .iter()
            .filter(|s| s.len() > 0)
            .filter_map(|s| s.shader.clone())
            .collect()
    }

    /// The heightmap GPU particles want refilled now (wiki "GPU Particles": the motion-blocking
    /// heights of `size` columns a side around the camera, every half second), with `min_y` the
    /// world's bottom; `None` while no type with live records reads it, or the last one is
    /// fresh. A type reads it when its shader samples it (`Sampler1`), or when it dies under it
    /// and draws its quads here. The caller fills it before the frame's quads are built: for
    /// the shaders' texture when [`HeightmapRefresh::for_shaders`], and
    /// [`Self::set_polytone_surface`] for the quads.
    pub fn polytone_heightmap_due(
        &mut self,
        camera: DVec3,
        size: u32,
        min_y: i32,
    ) -> Option<HeightmapRefresh> {
        let gpu = &mut self.polytone_gpu;
        let live = || gpu.stores.iter().filter(|s| s.len() > 0);
        let for_shaders = live().any(|s| {
            s.usable_shader()
                .is_some_and(|shader| shader.samples_heightmap)
        });
        let for_quads = live().any(|s| {
            s.gpu.kill_below_heightmap && !s.shader.as_ref().is_some_and(|shader| shader.drawn())
        });
        if !for_shaders && !for_quads {
            return None;
        }
        if let Some((area, at)) = gpu.heightmap
            && gpu.ticks < at + 10
            && area.min_y == min_y
        {
            return None;
        }
        let half = (size / 2) as i32;
        let area = HeightmapArea {
            origin: [
                camera.x.floor() as i32 - half,
                camera.z.floor() as i32 - half,
            ],
            size,
            min_y,
        };
        gpu.heightmap = Some((area, gpu.ticks));
        Some(HeightmapRefresh { area, for_shaders })
    }

    /// The heights the caller filled for the area [`Self::polytone_heightmap_due`] asked for,
    /// for this frame's quads on.
    pub fn set_polytone_surface(&mut self, area: HeightmapArea, heights: &[u16]) {
        self.polytone_gpu.surface = Some(Surface {
            area,
            heights: heights.into(),
        });
    }

    /// The GPU particle types the renderer draws with their own shaders this frame (wiki "GPU
    /// Particles"): each one's live records with this frame's block and uniforms.
    pub fn polytone_shader_batches(
        &self,
        camera: DVec3,
        partial: f32,
        light_at: &dyn Fn([i32; 3]) -> u8,
    ) -> Vec<ShaderBatch> {
        let Some(registry) = &self.polytone else {
            return Vec::new();
        };
        let gpu = &self.polytone_gpu;
        let frame = BatchFrame {
            camera,
            now: gpu.ticks,
            partial,
            heightmap: gpu.heightmap.map(|(area, _)| area),
            light_at,
        };
        let mut light_cache = ParticleLightCache::default();
        gpu.stores
            .iter()
            .filter_map(|store| {
                let shader = store.shader.as_ref().filter(|s| s.drawn())?;
                let uniforms = registry.gpu_uniforms(shader, camera.to_array());
                store.shader_batch(&frame, &mut light_cache, uniforms)
            })
            .filter(|batch| !batch.vertices.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(gravity: f64, friction: f64) -> Gpu {
        Gpu {
            texture: String::new(),
            frames: 1,
            gravity,
            friction,
            size_end: None,
            color_end: None,
            fade: [0.0; 2],
            sway: 0.0,
            spin: 0.0,
            aspect: 1.0,
            area: None,
            kill_below_heightmap: false,
        }
    }

    /// A GPU particle type (`area` of `count` copies when given) with its own shader.
    fn store(area_count: Option<u32>) -> Store {
        let mut json = serde_json::json!({
            "type": "gpu", "texture": "test:textures/particle/rain.png", "render_type": "opaque"
        });
        if let Some(count) = area_count {
            json["area"] = serde_json::json!({"size": [8, 8, 8], "count": count});
        }
        let def = Arc::new(CustomParticle::parse(&json).unwrap());
        let program = crate::core_shader::translate_core_with(
            "#version 330\nlayout(location = 0) in vec3 Position;\n\
             void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n",
            "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = vec4(1.0);\n}\n",
            &[],
            &|_| None,
            &GLUE,
            &|_| false,
        )
        .expect("the shaders translate");
        let shader = GpuShader::new(
            "test:rain".to_owned(),
            Arc::new(program),
            None,
            RenderType::Opaque,
            Vec::new(),
        );
        let store = Store::new(&def, Some(Arc::new(shader))).unwrap();
        // As the renderer decides for a frame it draws in.
        store.shader.as_ref().unwrap().set_drawn(true);
        store
    }

    /// A record spawned on tick `spawn`, alive `lifetime` ticks.
    fn record(position: [f64; 3], spawn: u64, lifetime: f32) -> Record {
        Record {
            position,
            velocity: [0.1, -0.5, 0.0],
            spawn,
            lifetime,
            seed: 7,
            rgba: [1.0, 0.5, 0.25, 1.0],
            size: 0.15,
            roll: 0.3,
            custom: 2.0,
        }
    }

    fn batch(store: &Store, camera: DVec3, now: u64, partial: f32) -> Option<ShaderBatch> {
        let frame = BatchFrame {
            camera,
            now,
            partial,
            heightmap: None,
            // Sky light 15, block light 7.
            light_at: &|_| 0xF7,
        };
        let uniforms = vec![("Brightness".to_owned(), 0.5)];
        store.shader_batch(&frame, &mut ParticleLightCache::default(), uniforms)
    }

    /// The block as the shader reads it: little-endian floats.
    fn floats(block: &[u8]) -> Vec<f32> {
        block
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    }

    /// Wiki "GPU Particles": the record is the vertex format, the same on the quad's four
    /// corners: its place relative to the camera's block, its velocity, spawn tick and life on
    /// the batch's clock (65536 ticks back at most), size, roll, `custom`, colour and light. A
    /// record whose life is over is left out, one spawned before the clock's start included;
    /// one spawned before it and still alive keeps a spawn tick below 0.
    #[test]
    fn a_shader_batch_gives_each_live_record_its_four_corners() {
        let mut store = store(None);
        store
            .records
            .push(record([12.25, 71.5, -2.0], 65_540, 20.0));
        store.records.push(record([0.0; 3], 65_500, 20.0));
        store.records.push(record([0.0; 3], 65_530, 20.0));
        let camera = DVec3::new(10.5, 70.25, -3.75);
        let live = batch(&store, camera, 65_546, 0.5).unwrap();
        assert_eq!(live.vertices.len(), 8);
        // Across the rebase: spawned 6 ticks before 65536, 16.5 ticks of its 20 gone.
        assert_eq!(live.vertices[4].spawn_life, [-6.0, 20.0]);
        let v = live.vertices[0];
        assert!(live.vertices[..4].iter().all(|c| *c == v));
        assert_eq!(v.position, [2.25, 1.5, 2.0]);
        assert_eq!(v.velocity, [0.1, -0.5, 0.0]);
        assert_eq!(v.spawn_life, [4.0, 20.0]);
        assert_eq!(v.params[..3], [0.15, 0.3, 2.0]);
        assert!((0.0..1.0).contains(&v.params[3]));
        assert_eq!(v.color, [255, 128, 64, 255]);
        assert_eq!(v.uv2, [112, 240]);
        // `Origin` (the camera's block from the camera) and `Time`.
        assert_eq!(floats(&live.block)[..4], [-0.5, -0.25, -0.25, 10.5]);
        assert_eq!(live.uniforms, [("Brightness".to_owned(), 0.5)]);
        // The second record spawned 36 ticks before the clock's start and lives 20: over.
        store.records.remove(0);
        store.records.pop();
        assert!(
            batch(&store, camera, 65_546, 0.5)
                .unwrap()
                .vertices
                .is_empty()
        );
    }

    /// Measured beside the real mod: an `area`'s copies of a record with its own shader come
    /// on the record's own place, one quad after the other, so a shader spreads them by
    /// `gl_VertexIndex` (as the default shader does) and three additive copies it leaves in
    /// place are three times as bright.
    #[test]
    fn an_areas_copies_stack_on_their_record() {
        let camera = DVec3::new(0.5, 0.5, 0.5);
        let mut stacked = store(Some(3));
        stacked.records.push(record([3.0, 2.0, 1.0], 10, 20.0));
        stacked.records.push(record([5.0, 2.0, 1.0], 10, 20.0));
        let copies = batch(&stacked, camera, 12, 0.0).unwrap();
        assert_eq!(copies.vertices.len(), 2 * 3 * 4);
        let places: Vec<[f32; 3]> = copies.vertices.iter().map(|v| v.position).collect();
        assert!(places[..12].iter().all(|p| *p == [3.0, 2.0, 1.0]));
        assert!(places[12..].iter().all(|p| *p == [5.0, 2.0, 1.0]));
        let mut single = store(None);
        single.records.push(record([3.0, 2.0, 1.0], 10, 20.0));
        assert_eq!(batch(&single, camera, 12, 0.0).unwrap().vertices.len(), 4);
    }

    /// A type with its own shader draws its quads and hands over no batch until the renderer
    /// says it draws the shader in this frame, and again once it says it can't (the shader
    /// doesn't build, or the frame holds linear light).
    #[test]
    fn a_type_keeps_its_quads_unless_the_renderer_draws_its_shader() {
        let json = serde_json::json!({
            "type": "gpu", "texture": "test:textures/particle/rain.png", "render_type": "opaque"
        });
        let def = Arc::new(CustomParticle::parse(&json).unwrap());
        let shader = store(None).shader.unwrap();
        let fresh = GpuShader::new(
            shader.id.clone(),
            shader.program.clone(),
            None,
            RenderType::Opaque,
            Vec::new(),
        );
        let mut store = Store::new(&def, Some(Arc::new(fresh))).unwrap();
        store.records.push(record([3.0, 2.0, 1.0], 10, 20.0));
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        let atlas = HashMap::from([("test:particle/rain".to_owned(), (rect, true))]);
        let quads = |store: &Store| {
            let (mut out, mut additive) = (Vec::new(), Vec::new());
            let frame = QuadFrame {
                atlas: &atlas,
                camera: DVec3::ZERO,
                facing: Quat::IDENTITY,
                partial: 0.0,
                light_at: &|_| 0xF0,
                surface: None,
            };
            let mut light_cache = ParticleLightCache::default();
            store.render(&frame, 12, &mut light_cache, &mut out, &mut additive);
            out.len() / 4
        };
        let shader = store.shader.clone().unwrap();
        assert_eq!(quads(&store), 1, "no renderer decided: its quads");
        assert!(batch(&store, DVec3::ZERO, 12, 0.0).is_none());
        shader.set_drawn(true);
        assert_eq!(quads(&store), 0, "drawn with its shader: no quads");
        assert!(batch(&store, DVec3::ZERO, 12, 0.0).is_some());
        shader.set_drawn(false);
        assert_eq!(quads(&store), 1, "refused: its quads again");
        assert!(batch(&store, DVec3::ZERO, 12, 0.0).is_none());
    }

    /// Wiki "GPU Particles": a `kill_below_heightmap` type drawn as quads hides the ones under
    /// the motion-blocking surface of their column, not the ones where the sky is shaded: a drop
    /// in front of a hill, in its shade, still falls to the grass.
    #[test]
    fn rain_dies_under_the_heightmap_not_in_sky_shade() {
        let json = serde_json::json!({
            "type": "gpu", "texture": "test:textures/particle/rain.png",
            "render_type": "opaque", "kill_below_heightmap": true
        });
        let def = Arc::new(CustomParticle::parse(&json).unwrap());
        let mut store = Store::new(&def, None).unwrap();
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        let atlas = HashMap::from([("test:particle/rain".to_owned(), (rect, true))]);
        // Ground at y 64 over a 4x4 area from (0, 0).
        let surface = Surface {
            area: HeightmapArea {
                origin: [0, 0],
                size: 4,
                min_y: 0,
            },
            heights: vec![64u16; 16].into(),
        };
        let quads = |store: &Store, sky: u8| {
            let (mut out, mut additive) = (Vec::new(), Vec::new());
            let light_at = move |_: [i32; 3]| sky << 4;
            let frame = QuadFrame {
                atlas: &atlas,
                camera: DVec3::ZERO,
                facing: Quat::IDENTITY,
                partial: 0.0,
                light_at: &light_at,
                surface: Some(&surface),
            };
            let mut light_cache = ParticleLightCache::default();
            store.render(&frame, 10, &mut light_cache, &mut out, &mut additive);
            out.len() / 4
        };
        let still = |y: f64| Record {
            velocity: [0.0; 3],
            ..record([1.5, y, 1.5], 10, 20.0)
        };
        store.records.push(still(64.5));
        assert_eq!(quads(&store, 15), 1, "above the ground in the open");
        assert_eq!(quads(&store, 8), 1, "above the ground in a hill's shade");
        store.records[0] = still(63.5);
        assert_eq!(quads(&store, 15), 0, "under the ground");
    }

    #[test]
    fn texture_paths_name_their_atlas_sprites() {
        assert_eq!(
            sprite_name("davidparticles:textures/particle/weather/rain.png"),
            "davidparticles:particle/weather/rain"
        );
        assert_eq!(
            sprite_name("minecraft:textures/particle/generic_7.png"),
            "particle/generic_7"
        );
    }

    #[test]
    fn motion_follows_gravity_and_drag() {
        // No drag: a thrown body, x = v t, y = -g t^2 / 2.
        let (at, v) = motion(&gpu(0.1, 1.0), [0.0; 3], [1.0, 0.0, 0.0], 10.0);
        assert_eq!(at, [10.0, -5.0, 0.0]);
        assert!((v[1] + 1.0).abs() < 1e-12);
        // Drag: the fall speed tends to g / drag (terminal velocity).
        let friction = 0.9;
        let (_, v) = motion(&gpu(0.1, friction), [0.0; 3], [0.0; 3], 1000.0);
        assert!((v[1] + 0.1 / -f64::ln(friction)).abs() < 1e-9);
    }

    /// Measured in Java on a line of particles whose age grows with x: a quarter of the way
    /// through its fade-in a particle has about a sixth of its alpha, not a quarter; half at half.
    #[test]
    fn fades_ease_in_and_out() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(close(fade(0.0, 0.4, 0.2), 0.0));
        assert!(close(fade(0.1, 0.4, 0.2), 0.15625));
        assert!(close(fade(0.2, 0.4, 0.2), 0.5));
        assert!(close(fade(0.5, 0.4, 0.2), 1.0));
        assert!(close(fade(0.9, 0.4, 0.2), 0.5));
        assert!(close(fade(1.0, 0.4, 0.2), 0.0));
        assert!(close(fade(0.3, 0.0, 0.0), 1.0));
    }

    #[test]
    fn orientations_turn_the_quad_normal() {
        let normal = |q: Quat| q * FVec3::Z;
        let close = |a: FVec3, b: FVec3| (a - b).length() < 1e-5;
        // Lying flat: facing up.
        assert!(close(
            normal(orient(
                Orientation::Flat,
                FVec3::ONE,
                FVec3::ZERO,
                0.0,
                Quat::IDENTITY
            )),
            FVec3::Y
        ));
        // Around Y: facing the camera horizontally (the camera at the origin, the quad east).
        assert!(close(
            normal(orient(
                Orientation::AroundY,
                FVec3::new(5.0, 3.0, 0.0),
                FVec3::ZERO,
                0.0,
                Quat::IDENTITY
            )),
            -FVec3::X
        ));
        // Along its motion: its up is the velocity, its face toward the camera.
        let q = orient(
            Orientation::Movement,
            FVec3::new(0.0, 0.0, -4.0),
            FVec3::new(0.0, -2.0, 0.0),
            0.0,
            Quat::IDENTITY,
        );
        assert!(close(q * FVec3::Y, -FVec3::Y));
        assert!(close(normal(q), FVec3::Z));
    }
}
