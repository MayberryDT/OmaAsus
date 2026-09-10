//! Ambient GPU background: a slow domain-warped aurora field tinted by the
//! profile accent, warmed by thermal load, with vignette and film grain.
//! Rendered by a custom wgpu pipeline through `iced::widget::shader`.

use iced::widget::shader;
use iced::wgpu;
use iced::{Color, Rectangle, mouse};

#[derive(Debug, Clone, Copy)]
pub struct Ambient {
    pub accent: Color,
    pub accent2: Color,
    /// 0 = cold, 1 = hot.
    pub heat: f32,
    /// 0 = idle, 1 = full load.
    pub load: f32,
    pub time: f32,
    /// Overall intensity (overlay wants it quieter).
    pub intensity: f32,
    /// Corner radius in px (0 = square) and alpha of the field.
    pub radius: f32,
    pub alpha: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    heat: f32,
    accent: [f32; 4],
    accent2: [f32; 4],
    load: f32,
    intensity: f32,
    _pad: [f32; 2],
}

#[derive(Debug)]
pub struct Primitive {
    uniforms: Uniforms,
}

impl<M> shader::Program<M> for Ambient {
    type State = ();
    type Primitive = Primitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, bounds: Rectangle) -> Primitive {
        Primitive {
            uniforms: Uniforms {
                resolution: [bounds.width, bounds.height],
                time: self.time,
                heat: self.heat,
                accent: [self.accent.r, self.accent.g, self.accent.b, 1.0],
                accent2: [self.accent2.r, self.accent2.g, self.accent2.b, 1.0],
                load: self.load,
                intensity: self.intensity,
                _pad: [self.radius, self.alpha],
            },
        }
    }
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oma ambient"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER)),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oma ambient uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oma ambient bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oma ambient bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oma ambient pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, uniform, bind_group }
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(&self, pipeline: &mut Pipeline, _device: &wgpu::Device, queue: &wgpu::Queue, _bounds: &Rectangle, _viewport: &shader::Viewport) {
        queue.write_buffer(&pipeline.uniform, 0, bytemuck::bytes_of(&self.uniforms));
    }

    fn draw(&self, pipeline: &Pipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.bind_group, &[]);
        pass.draw(0..3, 0..1);
        true
    }
}

const SHADER: &str = r#"
struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    heat: f32,
    accent: vec4<f32>,
    accent2: vec4<f32>,
    load: f32,
    intensity: f32,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var o: VsOut;
    o.pos = vec4(p[i], 0.0, 1.0);
    o.uv = (p[i] + vec2(1.0, 1.0)) * 0.5;
    return o;
}

fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u2 = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2(1.0, 0.0));
    let c = hash(i + vec2(0.0, 1.0));
    let d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u2.x), mix(c, d, u2.x), u2.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var v = 0.0;
    var a = 0.5;
    let rot = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * noise(p);
        p = rot * p * 2.03 + vec2(1.7, 9.2);
        a = a * 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    var uv = in.uv;
    uv.y = 1.0 - uv.y;
    let p = vec2(uv.x * aspect, uv.y);
    let t = u.time * 0.03;

    // Slow, low-frequency domain warp: broad ribbons, not texture.
    let q = vec2(fbm(p * 0.9 + vec2(0.0, t)), fbm(p * 0.9 + vec2(5.2, 1.3) - t * 0.6));
    let r = vec2(fbm(p * 0.9 + 2.2 * q + vec2(1.7, 9.2) + t * 0.4), fbm(p * 0.9 + 2.2 * q + vec2(8.3, 2.8) - t * 0.25));
    let f = fbm(p * 0.9 + 2.0 * r);

    let ink = vec3(0.016, 0.018, 0.030);
    let deep = vec3(0.030, 0.034, 0.056);
    let warm = mix(u.accent.rgb, vec3(1.0, 0.42, 0.18), u.heat * 0.7);
    let cool = mix(u.accent2.rgb, vec3(0.12, 0.22, 0.55), 0.5);

    var col = mix(deep, ink, clamp(uv.y * 0.9 + uv.x * 0.3, 0.0, 1.0));
    let ribbon = smoothstep(0.42, 0.85, f);
    let ribbon2 = smoothstep(0.58, 0.98, fbm(p * 1.3 - 1.2 * r + t * 0.7));
    col = col + warm * ribbon * (0.14 + 0.14 * u.load) * u.intensity;
    col = col + cool * ribbon2 * 0.10 * u.intensity;

    // A single light behind the brand, top-left, and a faint counter-light.
    let d1 = distance(p, vec2(0.12 * aspect, 0.08));
    col = col + warm * exp(-d1 * d1 * 2.2) * 0.10 * u.intensity;
    let d2 = distance(p, vec2(0.95 * aspect, 1.0));
    col = col + cool * exp(-d2 * d2 * 1.8) * 0.08 * u.intensity;

    // Vignette + fine grain.
    let vg = smoothstep(1.4, 0.3, distance(uv, vec2(0.5, 0.5)) * 1.1);
    col = col * (0.70 + 0.30 * vg);
    let grain = (hash(uv * u.resolution + fract(u.time) * 17.0) - 0.5) * 0.02;
    col = col + vec3(grain);

    // Rounded-rectangle mask (signed distance in pixels).
    let px = uv * u.resolution;
    let rad = u._pad.x;
    let half = u.resolution * 0.5;
    let dd = abs(px - half) - (half - vec2(rad, rad));
    let sd = length(max(dd, vec2(0.0, 0.0))) + min(max(dd.x, dd.y), 0.0) - rad;
    let mask = 1.0 - smoothstep(-1.0, 1.0, sd);
    let a = u._pad.y * mask;
    return vec4(col * a, a);
}
"#;
