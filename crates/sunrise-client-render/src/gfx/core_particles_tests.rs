use super::*;
use crate::particles::polytone_gpu::GLUE;
use glam::Mat4;
use sunrise_polytone::gpu_particle::{GPU_PARTICLE_GLSL, GPU_PARTICLE_INCLUDE};

const INCLUDES: [(&str, &str); 2] = [
    (
        "minecraft:shaders/include/dynamictransforms.glsl",
        "layout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n",
    ),
    (
        "minecraft:shaders/include/projection.glsl",
        "layout(std140) uniform Projection {\n    mat4 ProjMat;\n};\n",
    ),
];

fn include(id: &str) -> Option<String> {
    if id == GPU_PARTICLE_INCLUDE {
        return Some(GPU_PARTICLE_GLSL.to_owned());
    }
    INCLUDES
        .iter()
        .find(|(path, _)| *path == id)
        .map(|(_, text)| (*text).to_owned())
}

/// A custom shader in the wiki's 1.21.11 form: the record's quad around `Position + Origin`,
/// `Params.x` its half-width, the corner from `gl_VertexIndex & 3`.
const VSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
    #include <minecraft:dynamictransforms.glsl>\n#include <minecraft:projection.glsl>\n\
    #include <polytone:gpu_particle.glsl>\n\
    layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec3 Velocity;\n\
    layout(location = 2) in vec2 SpawnLife;\nlayout(location = 3) in vec4 Params;\n\
    layout(location = 4) in vec4 Color;\nlayout(location = 5) in ivec2 UV2;\n\
    layout(location = 0) out vec4 vertexColor;\n\
    void main() {\n    int corner = gl_VertexIndex & 3;\n\
        vec2 side = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n\
        vec3 at = Position + Origin + vec3(side * Params.x, 0.0);\n\
        gl_Position = ProjMat * ModelViewMat * vec4(at, 1.0);\n\
        vertexColor = Color;\n}\n";

/// The record's colour times the pack uniform `Brightness`, a block of its own.
const FSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
    layout(std140) uniform Brightness {\n    float brightnessValue;\n};\n\
    layout(location = 0) in vec4 vertexColor;\nlayout(location = 0) out vec4 fragColor;\n\
    void main() {\n    fragColor = vec4(vertexColor.rgb * brightnessValue, 1.0);\n}\n";

/// The test shader, translated as if `Brightness` were given, with the pack uniforms named
/// `uniforms` (each 0.5).
fn shader(uniforms: &[&str]) -> Arc<GpuShader> {
    custom(VSH, FSH, &include, None, RenderType::Opaque, uniforms)
}

/// `vsh` and `fsh` with `include`'s files, as a GPU type's shader drawn as `render_type`.
fn custom(
    vsh: &str,
    fsh: &str,
    include: &dyn Fn(&str) -> Option<String>,
    texture: Option<(u32, u32, Arc<[u8]>)>,
    render_type: RenderType,
    uniforms: &[&str],
) -> Arc<GpuShader> {
    let provided = |name: &str| matches!(name, GPU_BLOCK | "Brightness");
    let program = crate::core_shader::translate_core_with(vsh, fsh, &[], include, &GLUE, &provided)
        .expect("the shaders translate");
    Arc::new(GpuShader::new(
        "test:particle".to_owned(),
        Arc::new(program),
        texture,
        render_type,
        uniforms
            .iter()
            .map(|name| {
                let program = sunrise_polytone::expr::Program::compile("0.5").unwrap();
                ((*name).to_owned(), program)
            })
            .collect(),
    ))
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let device = crate::gfx::test_gpu::optional_device();
    if device.is_none() {
        eprintln!("skipped: no adapter");
    }
    device.map(|(_, device, queue)| (device, queue))
}

const SIZE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One red record at the batch's `Origin`, 2 blocks ahead of the camera, `half` blocks
/// across each way, with `Brightness` 0.5, `copies` times one after the other.
fn batch(shader: Arc<GpuShader>, half: f32, copies: usize) -> ShaderBatch {
    let mut block = [0u8; GPU_BLOCK_SIZE];
    for (i, v) in [0.0f32, 0.0, -2.0].into_iter().enumerate() {
        block[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut vertex: ShaderVertex = bytemuck::Zeroable::zeroed();
    vertex.spawn_life = [0.0, 100.0];
    vertex.params = [half, 0.0, 0.0, 0.0];
    vertex.color = [255, 0, 0, 255];
    ShaderBatch {
        shader,
        vertices: vec![vertex; 4 * copies],
        block,
        uniforms: vec![("Brightness".to_owned(), 0.5)],
    }
}

/// A 1 by 1 lightmap of `value`.
fn lightmap(device: &wgpu::Device, queue: &wgpu::Queue, value: u8) -> wgpu::TextureView {
    texture(device, queue, 1, 1, &[value, value, value, 255]).create_view(&Default::default())
}

/// One frame of `particles`: the renderer's verdict on `shaders` for a frame of `linear`
/// light, then `batches` prepared.
fn frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    particles: &mut CoreParticles,
    linear: bool,
    shaders: &[Arc<GpuShader>],
    batches: Vec<ShaderBatch>,
) {
    let lightmap = lightmap(device, queue, 200);
    let sampler = device.create_sampler(&Default::default());
    particles.decide(
        device,
        queue,
        (FORMAT, linear),
        (&lightmap, &sampler),
        shaders,
    );
    let view = super::super::View {
        view: Mat4::IDENTITY,
        projection: super::super::world_projection(70.0, 1.0, 64.0),
        aspect: 1.0,
        tan_half_fov: 35f32.to_radians().tan(),
    };
    particles.batches = batches;
    particles.prepare_frame(device, queue, false, || {
        super::super::core_lines::uniforms(
            [0; 48],
            &view,
            Mat4::IDENTITY,
            Mat4::IDENTITY,
            1.0,
            [0.0; 4],
            [0.0; 6],
        )
    });
}

/// Draws what `particles` prepared on a 64 by 64 target cleared to transparent black, one quad
/// of indices bound: the RGBA rows read back.
fn draw(device: &wgpu::Device, queue: &wgpu::Queue, particles: &CoreParticles) -> Vec<u8> {
    let target = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
            view_formats: &[],
        })
    };
    let color = target(FORMAT, wgpu::TextureUsages::COPY_SRC);
    let depth = target(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::empty(),
    );
    let indices = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        },
    );
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        particles.draw(&mut pass, &indices, 1);
    }
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
        },
        color.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    read.slice(..).get_mapped_range().unwrap().to_vec()
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

/// The record reaches the shader as its vertex format (position, size, colour), the knobs block
/// carries `Origin` and the pack's uniform is its own block: a half-red quad in the middle,
/// nothing at the edge, the frame's alpha (HDR output's marks) left as it was. The pipeline
/// is built when the renderer decides, before the frame's quads, and draws in that frame.
#[test]
#[ignore = "daily"]
fn a_custom_shader_draws_its_records_with_its_uniforms() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let mut particles = CoreParticles::new(&device, &queue, (FORMAT, false));
    let shader = shader(&["Brightness"]);
    let batches = vec![batch(shader.clone(), 0.5, 1)];
    frame(
        &device,
        &queue,
        &mut particles,
        false,
        std::slice::from_ref(&shader),
        batches,
    );
    assert!(shader.drawn() && particles.drawing());
    let pixels = draw(&device, &queue, &particles);
    let middle = at(&pixels, 32, 32);
    assert!(
        middle[0].abs_diff(128) <= 1 && middle[1..] == [0, 0, 0],
        "{middle:?}"
    );
    assert_eq!(at(&pixels, 2, 2), [0, 0, 0, 0]);
}

/// A shader reading a block that is neither the knobs nor one of its type's `uniforms` is not
/// used: nothing draws, rather than failing at the draw, and its type keeps its quads.
#[test]
#[ignore = "daily"]
fn a_shader_reading_a_block_it_is_not_given_is_refused() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let mut particles = CoreParticles::new(&device, &queue, (FORMAT, false));
    let shader = shader(&[]);
    let batches = vec![batch(shader.clone(), 0.5, 1)];
    frame(
        &device,
        &queue,
        &mut particles,
        false,
        std::slice::from_ref(&shader),
        batches,
    );
    assert!(shader.refused() && !particles.drawing());
    assert_eq!(at(&draw(&device, &queue, &particles), 32, 32), [0, 0, 0, 0]);
}

/// The backend refusing a shader that translated (its own `DynamicTransforms`, bigger than the
/// block the pass binds) when the renderer first decides on it: the shader is refused in that
/// same frame, before its type's quads are built, so they still draw.
#[test]
#[ignore = "daily"]
fn a_shader_the_backend_refuses_at_its_first_frame_keeps_the_types_quads() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let bigger = |id: &str| {
        include(id)
            .map(|text| text.replace("vec3 ModelOffset;", "vec3 ModelOffset;\n    vec4 More;"))
    };
    let shader = custom(VSH, FSH, &bigger, None, RenderType::Opaque, &["Brightness"]);
    let mut particles = CoreParticles::new(&device, &queue, (FORMAT, false));
    let batches = vec![batch(shader.clone(), 0.5, 1)];
    frame(
        &device,
        &queue,
        &mut particles,
        false,
        std::slice::from_ref(&shader),
        batches,
    );
    assert!(shader.refused() && !shader.drawn());
    assert!(!particles.drawing());
}

/// A pack's GLSL writes vanilla's gamma: on a frame of linear light the shader isn't drawn and
/// its type keeps its quads. A renderer made for another frame decides again from scratch, so
/// a shader drawn before is refused in the first frame of linear light, before the quads.
#[test]
#[ignore = "daily"]
fn a_frame_of_linear_light_keeps_the_types_quads_from_its_first_frame() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let shader = shader(&["Brightness"]);
    let mut gamma = CoreParticles::new(&device, &queue, (FORMAT, false));
    frame(
        &device,
        &queue,
        &mut gamma,
        false,
        std::slice::from_ref(&shader),
        Vec::new(),
    );
    assert!(shader.drawn());
    let mut linear = CoreParticles::new(&device, &queue, (FORMAT, true));
    frame(
        &device,
        &queue,
        &mut linear,
        true,
        std::slice::from_ref(&shader),
        Vec::new(),
    );
    assert!(
        shader.refused(),
        "refused before the first linear frame's quads"
    );
    // A renderer whose frame changes in place decides again for the new frame.
    frame(
        &device,
        &queue,
        &mut gamma,
        true,
        std::slice::from_ref(&shader),
        Vec::new(),
    );
    assert!(shader.refused(), "the same renderer, now on linear light");
    // And drawn again by a renderer for a frame of gamma.
    frame(
        &device,
        &queue,
        &mut gamma,
        false,
        std::slice::from_ref(&shader),
        Vec::new(),
    );
    assert!(shader.drawn());
}

/// What the shader samples: `Sampler0` the type's texture, `Sampler1` the heightmap (r its
/// heights' low byte) and `Sampler2` the lightmap.
#[test]
#[ignore = "daily"]
fn a_custom_shader_samples_its_texture_the_heightmap_and_the_lightmap() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let fsh = "#version 330\nuniform sampler2D Sampler0;\nuniform sampler2D Sampler1;\n\
        uniform sampler2D Sampler2;\nlayout(location = 0) in vec4 vertexColor;\n\
        layout(location = 0) out vec4 fragColor;\nvoid main() {\n\
            fragColor = vec4(texture(Sampler0, vec2(0.5)).r, texture(Sampler1, vec2(0.5)).r, \
            texture(Sampler2, vec2(0.5)).r, 1.0);\n}\n";
    let texture = Some((1, 1, Arc::from(&[64u8, 0, 0, 255][..])));
    let shader = custom(VSH, fsh, &include, texture, RenderType::Opaque, &[]);
    assert!(shader.samples_heightmap);
    let mut particles = CoreParticles::new(&device, &queue, (FORMAT, false));
    let size = HEIGHTMAP_SIZE as usize;
    particles.set_heightmap(&queue, &vec![0x0164; size * size]);
    let batches = vec![batch(shader.clone(), 0.5, 1)];
    frame(
        &device,
        &queue,
        &mut particles,
        false,
        std::slice::from_ref(&shader),
        batches,
    );
    let middle = at(&draw(&device, &queue, &particles), 32, 32);
    assert_eq!(middle[..3], [64, 0x64, 200], "{middle:?}");
}

/// A shader spreads an `area`'s copies by the quad they are (`gl_VertexIndex >> 2`): the
/// copies of one record come one quad after the other, so three of them land in three places.
#[test]
#[ignore = "daily"]
fn a_shader_spreads_a_records_copies() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let spread = VSH.replace(
        "vec3 at = Position + Origin + vec3(side * Params.x, 0.0);",
        "float copy = float((gl_VertexIndex >> 2) % 3) - 1.0;\n\
         vec3 at = Position + Origin + vec3(side * Params.x + vec2(copy, 0.0), 0.0);",
    );
    let shader = custom(
        &spread,
        FSH,
        &include,
        None,
        RenderType::Opaque,
        &["Brightness"],
    );
    let mut particles = CoreParticles::new(&device, &queue, (FORMAT, false));
    let batches = vec![batch(shader.clone(), 0.25, 3)];
    frame(
        &device,
        &queue,
        &mut particles,
        false,
        std::slice::from_ref(&shader),
        batches,
    );
    let pixels = draw(&device, &queue, &particles);
    for x in [9, 32, 55] {
        assert!(at(&pixels, x, 32)[0] > 100, "a copy at {x}");
    }
    for x in [20, 44] {
        assert_eq!(
            at(&pixels, x, 32),
            [0, 0, 0, 0],
            "between the copies at {x}"
        );
    }
}
