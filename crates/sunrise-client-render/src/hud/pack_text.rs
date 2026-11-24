//! A resource pack's `core/text` shader on the GUI's text (`RenderPipelines.GUI_TEXT`, the shader
//! with `IS_GUI`): servers' HUDs that their pack's vertex shader moves to the screen's corners,
//! and text effects in its fragment shader. Without one the text draws natively.

use crate::resource_source::ResourceSource;
use std::sync::Arc;

use crate::core_shader::{CoreProgram, Glue};

use super::{HudBatch, HudVertex};
use crate::gfx::hud_coverage::Pipeline;

/// GUI text's vertex format (`POSITION_TEX_COLOR`) from the HUD's vertices. A glyph is two
/// triangles here, its corners top left, top right, bottom right, top left, bottom right, bottom
/// left; vanilla's quads go top left, bottom left, bottom right, top right, which is what
/// `gl_VertexID % 4` tells a pack's shader. The draw passes its first vertex as its instance.
const GLUE: Glue = Glue {
    inputs: &[("Position", "vec3"), ("Color", "vec4"), ("UV0", "vec2")],
    prelude: "layout(location = 0) in vec2 hz_pos;\n\
              layout(location = 1) in vec2 hz_uv;\n\
              layout(location = 2) in vec4 hz_color;\n\
              const int hz_corner[6] = int[6](0, 3, 2, 0, 2, 1);\n\
              int hz_vertex_id() {\n\
                  int i = int(gl_VertexIndex) - int(gl_InstanceIndex);\n\
                  return i / 6 * 4 + hz_corner[i % 6];\n\
              }",
    setup: "Position = vec3(hz_pos, 0.0); Color = hz_color; UV0 = hz_uv;",
    // GL's clip depth runs from -w to w, wgpu's from 0 to w; GUI text has no depth test.
    finish: "gl_Position.z = 0.5 * gl_Position.w;",
    vertex_id: "hz_vertex_id()",
};

/// The packs' text shader, when a pack replaces either stage.
pub(super) fn load(root: &ResourceSource, defaults: &ResourceSource) -> Option<Arc<CoreProgram>> {
    load_core(root, defaults, "text", &["IS_GUI"], &GLUE)
}

/// The packs' core shader `name` (`shaders/core/<name>.vsh` and `.fsh`) over `defaults` (the
/// game's own resources, which have vanilla's and its includes), when a pack replaces either
/// stage.
pub(super) fn load_core(
    root: &ResourceSource,
    defaults: &ResourceSource,
    name: &str,
    defines: &[&str],
    glue: &Glue,
) -> Option<Arc<CoreProgram>> {
    load_pair(root, defaults, (name, name), defines, glue, &|_| false)
}

/// [`load_core`] for a pipeline whose stages have names of their own (`core/screenquad` and
/// `core/lightmap`) and that gives the blocks `provided` says yes to.
pub(super) fn load_pair(
    root: &ResourceSource,
    defaults: &ResourceSource,
    names: (&str, &str),
    defines: &[&str],
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Option<Arc<CoreProgram>> {
    let name = names.1;
    match translate_pair(root, defaults, names, defines, glue, provided)? {
        Ok(program) => {
            log::info!("the resource pack's {name} shader is used");
            Some(Arc::new(program))
        }
        Err(e) => {
            log::warn!("the resource pack's {name} shader is not used: {e}");
            None
        }
    }
}

/// Why a pack's core shader is refused (`load_pair`), with the file it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A stage or include over the translator's budget for a whole shader
    /// (`core_shader::MAX_EXPANDED`).
    OverBudget { path: String },
    /// A stage or include that isn't UTF-8 text.
    NotText { path: String },
    /// A stage or include that couldn't be read otherwise.
    Unreadable { path: String, error: String },
    /// A stage the selected packs hid (`root` doesn't have it).
    StageHidden { path: String },
    /// An include the selected packs don't have (`ns:path`, as the shader asks for it).
    IncludeMissing { id: String },
    /// The translator refused the GLSL.
    Translation(String),
    /// The translator panicked on it.
    Panicked(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverBudget { path } => write!(
                f,
                "{path} is over the {} bytes a shader may have",
                crate::core_shader::MAX_EXPANDED
            ),
            Self::NotText { path } => write!(f, "{path} is not UTF-8 text"),
            Self::Unreadable { path, error } => write!(f, "{path}: {error}"),
            Self::StageHidden { path } => write!(f, "{path} is not in the selected packs"),
            Self::IncludeMissing { id } => write!(f, "{id} is not in the selected packs"),
            Self::Translation(e) => f.write_str(e),
            Self::Panicked(e) => write!(f, "the shader translator failed: {e}"),
        }
    }
}

/// [`load_pair`]'s program: none when no pack replaces either stage, else the translation, or
/// why the shader is refused. A stage the packs don't replace, and every include, is the stack's
/// (`root`: what the selected packs left of the game's resources), never `defaults` beside it,
/// so a file a pack's filter hid stays hidden and refuses the shader. `defaults` only tells a
/// pack's stage from the game's. Each stage and include is read only up to the translator's
/// budget ([`read_shader_file`]); a file over it refuses the shader by its path. A panic in the
/// translator refuses it too ([`contained`]).
fn translate_pair(
    root: &ResourceSource,
    defaults: &ResourceSource,
    (vertex_name, name): (&str, &str),
    defines: &[&str],
    glue: &Glue,
    provided: &dyn Fn(&str) -> bool,
) -> Option<Result<CoreProgram, Refusal>> {
    // The first file refused while reading, which the shader is refused for.
    let rejected = std::cell::RefCell::new(None);
    let read = |base: &ResourceSource, path: &str| {
        read_shader_file(base, path).unwrap_or_else(|e| {
            rejected.borrow_mut().get_or_insert(e);
            None
        })
    };
    let refused = || rejected.borrow().clone().map(Err);
    let vertex_path = format!("assets/minecraft/shaders/core/{vertex_name}.vsh");
    let fragment_path = format!("assets/minecraft/shaders/core/{name}.fsh");
    let (vertex, fragment) = (read(root, &vertex_path), read(root, &fragment_path));
    let own = |path: &str, text: &Option<String>| {
        text.is_some() && read(defaults, path).as_ref() != text.as_ref()
    };
    let replaced = own(&vertex_path, &vertex) || own(&fragment_path, &fragment);
    if let Some(refused) = refused() {
        return Some(refused);
    }
    if !replaced {
        return None;
    }
    let (Some(vertex), Some(fragment)) = (&vertex, &fragment) else {
        let path = if vertex.is_none() {
            vertex_path
        } else {
            fragment_path
        };
        return Some(Err(Refusal::StageHidden { path }));
    };
    // The first include the stack doesn't have, which the translator then fails on.
    let missing = std::cell::RefCell::new(None);
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        let text = read(root, &format!("assets/{namespace}/{path}"));
        if text.is_none() {
            missing.borrow_mut().get_or_insert_with(|| id.to_owned());
        }
        text
    };
    let translated = contained(|| {
        crate::core_shader::translate_core_with(vertex, fragment, defines, &include, glue, provided)
    });
    if let Some(refused) = refused() {
        return Some(refused);
    }
    Some(translated.map_err(|e| match (e, missing.into_inner()) {
        (Refusal::Translation(_), Some(id)) => Refusal::IncludeMissing { id },
        (e, _) => e,
    }))
}

/// `translate`'s result, or its panic as a refusal: a pack's GLSL is untrusted, and a fault the
/// translator or naga meets in it must not end the game.
fn contained(
    translate: impl FnOnce() -> Result<CoreProgram, String>,
) -> Result<CoreProgram, Refusal> {
    crate::crash::contain(translate)
        .map_err(Refusal::Panicked)
        .and_then(|translated| translated.map_err(Refusal::Translation))
}

/// A shader file (a stage or an include) of `base`, read only up to the translator's budget
/// for a whole shader (`core_shader::MAX_EXPANDED`, 1 MiB): none when `base` hasn't it, the
/// refusal naming its path when it is over the budget or isn't text.
fn read_shader_file(base: &ResourceSource, path: &str) -> Result<Option<String>, Refusal> {
    if !base.contains(path) {
        return Ok(None);
    }
    let max = crate::core_shader::MAX_EXPANDED as u64;
    let path = path.to_owned();
    // `read_limited` refuses a resource over `max` as invalid data.
    let bytes = base.read_limited(&path, max).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidData => Refusal::OverBudget { path: path.clone() },
        _ => Refusal::Unreadable {
            path: path.clone(),
            error: e.to_string(),
        },
    })?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Refusal::NotText { path })
}

/// The packs' `core/lightmap` with `core/screenquad`, when a pack replaces either stage: the
/// pass gives it `LightmapInfo` besides vanilla's blocks.
fn load_lightmap(root: &ResourceSource, defaults: &ResourceSource) -> Option<Arc<CoreProgram>> {
    load_pair(
        root,
        defaults,
        ("screenquad", "lightmap"),
        &[],
        &crate::gfx::core_lightmap::GLUE,
        &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
    )
}

/// The packs' core shaders the atlas carries (text, block outline, sky, stars, lightmap).
pub(super) fn load_shaders(
    atlas: &mut super::HudAtlas,
    root: &ResourceSource,
    defaults: &ResourceSource,
) {
    atlas.text_shaders.gui = load(root, defaults);
    atlas.lines_shader = load_core(
        root,
        defaults,
        "rendertype_lines",
        &[],
        &crate::gfx::core_lines::GLUE,
    );
    let sky = |name| load_core(root, defaults, name, &[], &crate::gfx::core_sky::GLUE);
    atlas.sky_shader = sky("sky");
    atlas.stars_shader = sky("stars");
    atlas.lightmap_shader = load_lightmap(root, defaults);
    let world = |defines: &[&str], glue| load_core(root, defaults, "text", defines, glue);
    atlas.text_shaders.world = world(&[], &crate::gfx::core_text::WORLD_GLUE);
    atlas.text_shaders.see_through = world(
        &["IS_SEE_THROUGH"],
        &crate::gfx::core_text::SEE_THROUGH_GLUE,
    );
}

/// Each uniform block's offset in the one buffer (`core_shader::BLOCKS` order).
const BLOCK_OFFSETS: [u64; 4] = [0, 256, 512, 768];
const BLOCK_SIZES: [u64; 4] = [48, 64, 160, 48];

/// The pipeline and its resources.
pub(super) struct PackText {
    pipeline: Pipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl PackText {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        atlas: &wgpu::TextureView,
        program: &CoreProgram,
        coverage: bool,
    ) -> Option<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Resource pack text (vertex)", &program.vertex);
        let fragment = module("Resource pack text (fragment)", &program.fragment);
        let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> = (0..4)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        for (_, binding) in crate::core_shader::SAMPLERS {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: binding + 1,
                visibility,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack text"),
            entries: &entries,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack text uniforms"),
            size: 1024,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The font textures' nearest filtering; every sampler a pack names reads the atlas.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Resource pack text"),
            ..Default::default()
        });
        let mut resources: Vec<wgpu::BindGroupEntry> = (0..4)
            .map(|i| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: BLOCK_OFFSETS[i],
                    size: wgpu::BufferSize::new(BLOCK_SIZES[i]),
                }),
            })
            .collect();
        for (_, binding) in crate::core_shader::SAMPLERS {
            resources.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(atlas),
            });
            resources.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack text"),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack text"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4];
        let desc = wgpu::RenderPipelineDescriptor {
            label: Some("Resource pack text"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                // BlendFunction.TRANSLUCENT.
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        };
        let source = wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(program.fragment.clone()));
        let pipeline = Pipeline::new(device, &desc, source, coverage);
        if let Some(e) = pollster::block_on(scope.pop()) {
            log::warn!("the resource pack's text shader is not used: {e}");
            return None;
        }
        let Some(pipeline) = pipeline else {
            log::warn!("the resource pack's text shader is not used: it has no colour output");
            return None;
        };
        Some(Self {
            pipeline,
            group,
            uniform,
        })
    }

    /// The frame's blocks, as `GuiRenderer` sets them for the GUI.
    pub(super) fn prepare(
        &self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
        size: [u32; 2],
        scale: f32,
    ) {
        uploads.buffer(encoder, &self.uniform, 0, &uniforms(batch, size, scale));
    }

    /// Draw `range` (whole glyph quads) with the pack's shader; the caller sets its own
    /// pipeline and bind group again after.
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: std::ops::Range<u32>,
        coverage: bool,
    ) {
        pass.set_pipeline(self.pipeline.get(coverage));
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(range.clone(), range.start..range.start + 1);
    }
}

/// The std140 blocks: `Globals` (`GlobalSettingsUniform.update`: the camera at the origin,
/// `GlintAlpha`, `GameTime`, `ScreenSize`), `Projection` (`GuiRenderer`'s orthographic GUI
/// projection, 1000 to 11000 deep, in GL's depth range), `DynamicTransforms` (the GUI's model view
/// moving 11000 back, colour modulator white) and an empty `Fog`.
/// `scale`: target pixels per GUI pixel (`batch.scale`, times `HudRenderer::zoom`).
fn uniforms(batch: &HudBatch, size: [u32; 2], scale: f32) -> Vec<u8> {
    let mut data = vec![0f32; 256];
    let globals = 0;
    data[globals + 3] = batch.glint_alpha;
    data[globals + 7] = batch.game_time;
    data[globals + 8] = size[0] as f32;
    data[globals + 9] = size[1] as f32;
    // Matrix4f.setOrtho(0, width, height, 0, near, far) with the GUI's scale, columns.
    let (w, h) = (size[0] as f32 / scale, size[1] as f32 / scale);
    let (near, far) = (1000.0f32, 11000.0f32);
    let projection = 64;
    data[projection] = 2.0 / w;
    data[projection + 5] = -2.0 / h;
    data[projection + 10] = 2.0 / (near - far);
    data[projection + 12] = -1.0;
    data[projection + 13] = 1.0;
    data[projection + 14] = (far + near) / (near - far);
    data[projection + 15] = 1.0;
    let transforms = 128;
    for (matrix, z) in [(transforms, -11000.0), (transforms + 16, 0.0)] {
        data[matrix] = 1.0;
        data[matrix + 5] = 1.0;
        data[matrix + 10] = 1.0;
        data[matrix + 14] = z;
        data[matrix + 15] = 1.0;
    }
    data[transforms + 32..transforms + 36].fill(1.0);
    bytemuck::cast_slice(&data).to_vec()
}

/// `range` split into runs of GUI text and of the rest, in order.
pub(super) fn text_runs(
    vertices: &[HudVertex],
    range: std::ops::Range<u32>,
) -> Vec<(std::ops::Range<u32>, bool)> {
    let mut runs: Vec<(std::ops::Range<u32>, bool)> = Vec::new();
    for i in range {
        let text = vertices.get(i as usize).is_some_and(|v| v.text != 0);
        match runs.last_mut() {
            Some((run, t)) if *t == text => run.end = i + 1,
            _ => runs.push((i..i + 1, text)),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gui_projection_maps_gui_pixels_to_the_screen() {
        let mut batch = HudBatch::scratch([1920, 1080]);
        batch.scale = 4.0;
        let data: Vec<f32> =
            bytemuck::cast_slice(&uniforms(&batch, [1920, 1080], batch.scale)).to_vec();
        let column = |matrix: usize, c: usize| &data[matrix + 4 * c..matrix + 4 * c + 4];
        let apply = |matrix: usize, v: [f32; 4]| -> [f32; 4] {
            std::array::from_fn(|r| (0..4).map(|c| column(matrix, c)[r] * v[c]).sum())
        };
        // ProjMat * ModelViewMat * (x, y, 0, 1) for the GUI's corners: 480 x 270 GUI pixels.
        for ([x, y], [cx, cy]) in [([0.0, 0.0], [-1.0, 1.0]), ([480.0, 270.0], [1.0, -1.0])] {
            let clip = apply(64, apply(128, [x, y, 0.0, 1.0]));
            assert!(
                (clip[0] - cx).abs() < 1e-5 && (clip[1] - cy).abs() < 1e-5,
                "{clip:?}"
            );
            // At GL's far plane, as vanilla's GUI is.
            assert!((clip[2] - 1.0).abs() < 1e-4, "{clip:?}");
        }
        // A pack finds the GUI scale as ScreenSize.x * ProjMat[0][0] / 2.
        assert!((data[8] * data[64] / 2.0 - 4.0).abs() < 1e-4);
    }

    /// Packs in memory: vanilla's lightmap stages (in shape) as the defaults, and `own` files
    /// of a pack over them.
    fn lightmap_packs(own: &[(&str, &str)]) -> (ResourceSource, ResourceSource) {
        use sunrise_assets::resource_source::Resource;
        let vanilla = [
            (
                "assets/minecraft/shaders/core/screenquad.vsh",
                "#version 330\nlayout(location = 0) out vec2 texCoord;\n\
                 void main() {\n    vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n\
                     gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);\n    texCoord = uv;\n}\n",
            ),
            (
                "assets/minecraft/shaders/core/lightmap.fsh",
                "#version 330\nlayout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                 layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                 void main() {\n    fragColor = vec4(info.SkyFactor);\n}\n",
            ),
        ];
        let source = |files: Vec<(&str, &str)>| {
            ResourceSource::indexed(
                files
                    .into_iter()
                    .map(|(path, text)| {
                        (path.to_owned(), Resource::bytes(text.as_bytes().to_vec()))
                    })
                    .collect(),
            )
        };
        let mut pack = vanilla.to_vec();
        for (path, text) in own {
            pack.retain(|(p, _)| p != path);
            pack.push((path, text));
        }
        (source(pack), source(vanilla.to_vec()))
    }

    fn lightmap(own: &[(&str, &str)]) -> Option<Arc<CoreProgram>> {
        let (root, defaults) = lightmap_packs(own);
        load_lightmap(&root, &defaults)
    }

    /// [`lightmap`]'s translation, with why it is refused.
    fn lightmap_translation(own: &[(&str, &str)]) -> Option<Result<CoreProgram, Refusal>> {
        let (root, defaults) = lightmap_packs(own);
        translate_pair(
            &root,
            &defaults,
            ("screenquad", "lightmap"),
            &[],
            &crate::gfx::core_lightmap::GLUE,
            &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
        )
    }

    /// A pack's `lightmap.fsh` importing `"tint.glsl"`.
    const IMPORTING_FSH: &str = "#version 330\n#moj_import \"tint.glsl\"\n\
        layout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
        layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = tint(info.SkyFactor);\n}\n";
    const TINT_GLSL: &str = "vec4 tint(float f) { return vec4(f, 0.5 * f, 0.0, 1.0); }\n";

    /// A stage the pack doesn't replace, and an include, come from the stack the selected packs
    /// left, never from the game's resources beside it: one a pack's filter hid refuses the
    /// shader, named, instead of coming back.
    #[test]
    fn a_file_the_selected_packs_hid_does_not_come_back() {
        use sunrise_assets::resource_source::Resource;
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let vsh = "assets/minecraft/shaders/core/screenquad.vsh";
        let tint = "assets/minecraft/shaders/core/tint.glsl";
        let (full, defaults) = lightmap_packs(&[(fsh, IMPORTING_FSH), (tint, TINT_GLSL)]);
        // The stack without one file: a filter hid it, though the game's resources have it.
        let hiding = |hidden: &str| {
            let files = full
                .list("assets/")
                .into_iter()
                .filter(|p| p != hidden)
                .map(|p| {
                    let text = full.read(&p).unwrap();
                    (p, Resource::bytes(text))
                })
                .collect();
            ResourceSource::indexed(files)
        };
        let defaults_with_tint = {
            let mut files: std::collections::BTreeMap<String, Resource> = defaults
                .list("assets/")
                .into_iter()
                .map(|p| {
                    let text = defaults.read(&p).unwrap();
                    (p, Resource::bytes(text))
                })
                .collect();
            files.insert(
                tint.to_owned(),
                Resource::bytes(TINT_GLSL.as_bytes().to_vec()),
            );
            ResourceSource::indexed(files)
        };
        let translate = |root: &ResourceSource| {
            translate_pair(
                root,
                &defaults_with_tint,
                ("screenquad", "lightmap"),
                &[],
                &crate::gfx::core_lightmap::GLUE,
                &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
            )
            .expect("the pack replaces a stage")
        };
        assert!(translate(&full).is_ok(), "nothing hidden");
        assert_eq!(
            translate(&hiding(vsh)).expect_err("the screen quad hidden"),
            Refusal::StageHidden { path: vsh.into() }
        );
        assert_eq!(
            translate(&hiding(tint)).expect_err("the include hidden"),
            Refusal::IncludeMissing {
                id: "minecraft:shaders/core/tint.glsl".into()
            }
        );
    }

    /// A stage that isn't UTF-8 text refuses the shader, named.
    #[test]
    fn a_stage_that_is_not_text_is_refused_by_its_path() {
        use sunrise_assets::resource_source::Resource;
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let (full, defaults) = lightmap_packs(&[]);
        let files = full
            .list("assets/")
            .into_iter()
            .map(|p| {
                let bytes = if p == fsh {
                    vec![0xff, 0xfe, 0x00]
                } else {
                    full.read(&p).unwrap()
                };
                (p, Resource::bytes(bytes))
            })
            .collect();
        let root = ResourceSource::indexed(files);
        let refused = translate_pair(
            &root,
            &defaults,
            ("screenquad", "lightmap"),
            &[],
            &crate::gfx::core_lightmap::GLUE,
            &|block| block == crate::gfx::core_lightmap::INFO_BLOCK,
        );
        assert_eq!(
            refused.expect("the pack replaces a stage").err(),
            Some(Refusal::NotText { path: fsh.into() })
        );
    }

    /// A panic in the translator is a refusal, not the game's end.
    #[test]
    fn a_translator_panic_refuses_the_shader() {
        assert!(matches!(
            contained(|| panic!("a fault in the translator")),
            Err(Refusal::Panicked(_))
        ));
    }

    /// A quoted import in a pack's `core/lightmap` is read from the shader's own folder,
    /// `minecraft:shaders/core/`, not from `shaders/include/`.
    #[test]
    fn a_lightmap_shaders_quoted_import_is_read_from_its_own_folder() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let beside = "assets/minecraft/shaders/core/tint.glsl";
        let program = lightmap_translation(&[(fsh, IMPORTING_FSH), (beside, TINT_GLSL)])
            .expect("the pack replaces a stage")
            .expect("the import beside the shader is found");
        assert_eq!(program.other, ["LightmapInfo"]);
        let include = "assets/minecraft/shaders/include/tint.glsl";
        let why = lightmap_translation(&[(fsh, IMPORTING_FSH), (include, TINT_GLSL)])
            .expect("the pack replaces a stage")
            .expect_err("an import only under shaders/include is not beside the shader");
        assert_eq!(
            why,
            Refusal::IncludeMissing {
                id: "minecraft:shaders/core/tint.glsl".into()
            }
        );
    }

    /// A shader file is read only up to the translator's 1 MiB budget: an import or a stage
    /// over it refuses the pack's lightmap, naming the file, and one at the budget is read.
    #[test]
    fn a_lightmap_shader_file_over_the_budget_is_refused_by_its_path() {
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let tint = "assets/minecraft/shaders/core/tint.glsl";
        // A comment pads the include to exactly `len` bytes.
        let padded = |len: usize| {
            let pad = len - TINT_GLSL.len() - 4;
            format!("// {}\n{TINT_GLSL}", "x".repeat(pad))
        };
        let at_budget = padded(1 << 20);
        assert_eq!(at_budget.len(), 1_048_576);
        let over = padded((1 << 20) + 1);
        let refused = |files: &[(&str, &str)]| {
            lightmap_translation(files)
                .expect("the pack replaces a stage")
                .expect_err("refused")
        };
        assert_eq!(
            refused(&[(fsh, IMPORTING_FSH), (tint, &over)]),
            Refusal::OverBudget { path: tint.into() }
        );
        // The same file at the budget gets past the read (and the translator then finds the
        // expanded shader too long, a refusal of its own).
        assert!(matches!(
            refused(&[(fsh, IMPORTING_FSH), (tint, &at_budget)]),
            Refusal::Translation(_)
        ));
        let big_stage = format!("// {}\n{IMPORTING_FSH}", "x".repeat(1 << 20));
        assert_eq!(
            refused(&[(fsh, &big_stage), (tint, TINT_GLSL)]),
            Refusal::OverBudget { path: fsh.into() }
        );
        assert!(lightmap(&[(fsh, &big_stage), (tint, TINT_GLSL)]).is_none());
    }

    /// A pack's `core/lightmap` is loaded with vanilla's `core/screenquad` (a stage of another
    /// name), and the reverse; none when no pack replaces either; refused when it reads a block
    /// the lightmap pass doesn't give.
    #[test]
    fn a_pack_replacing_either_lightmap_stage_is_loaded() {
        let red = "#version 330\nlayout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
                   layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(info.SkyFactor, 0.0, 0.0, 1.0);\n}\n";
        let fsh = "assets/minecraft/shaders/core/lightmap.fsh";
        let vsh = "assets/minecraft/shaders/core/screenquad.vsh";
        let program = lightmap(&[(fsh, red)]).expect("the pack's lightmap");
        assert_eq!(program.other, ["LightmapInfo"]);
        let quad = "#version 330\nlayout(location = 0) out vec2 texCoord;\n\
                    void main() {\n    texCoord = vec2(0.5);\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n";
        assert!(lightmap(&[(vsh, quad)]).is_some());
        assert!(lightmap(&[]).is_none(), "no pack replaces either stage");
        let other = red.replace("LightmapInfo", "Elsewhere");
        assert!(lightmap(&[(fsh, &other)]).is_none());
    }

    #[test]
    fn text_splits_from_the_rest() {
        let vertex = |text| HudVertex {
            pos: [0.0; 2],
            uv: [0.0; 2],
            color: 0,
            depth: 0.0,
            glint_uv: [0.0; 2],
            text,
        };
        let vertices: Vec<HudVertex> = [0, 0, 1, 1, 1, 0].into_iter().map(vertex).collect();
        assert_eq!(
            text_runs(&vertices, 1..6),
            [(1..2, false), (2..5, true), (5..6, false)]
        );
    }
}
