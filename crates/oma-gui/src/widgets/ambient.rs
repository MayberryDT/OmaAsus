//! The pixel field behind every surface (see `shaders::FIELD`).

use iced::wgpu;
use iced::widget::shader;
use iced::{Color, Rectangle, mouse};
use std::sync::atomic::{AtomicU32, Ordering};

pub static CURSOR_X: AtomicU32 = AtomicU32::new(0);
pub static CURSOR_Y: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy)]
pub struct Ambient {
    pub p: crate::theme::Palette,
    pub time: f32,
    pub heat: f32,
    pub load: f32,
    pub cell: f32,
    pub alpha: f32,
    pub intensity: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    cursor: [f32; 2],
    time: f32,
    cell: f32,
    load: f32,
    heat: f32,
    bg: [f32; 4],
    dim: [f32; 4],
    mid: [f32; 4],
    lit: [f32; 4],
    hover: [f32; 4],
    crest: [f32; 4],
    params: [f32; 4],
}

fn c4(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, 1.0]
}

#[derive(Debug)]
pub struct Primitive {
    uniforms: Uniforms,
}

impl<M> shader::Program<M> for Ambient {
    type State = ();
    type Primitive = Primitive;

    fn update(&self, _state: &mut (), event: &iced::Event, bounds: Rectangle, _cursor: mouse::Cursor) -> Option<shader::Action<M>> {
        if let iced::Event::Mouse(mouse::Event::CursorMoved { position }) = event {
            CURSOR_X.store((position.x - bounds.x).to_bits(), Ordering::Relaxed);
            CURSOR_Y.store((position.y - bounds.y).to_bits(), Ordering::Relaxed);
        }
        None
    }

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, bounds: Rectangle) -> Primitive {
        let p = self.p;
        Primitive {
            uniforms: Uniforms {
                resolution: [bounds.width, bounds.height],
                cursor: [f32::from_bits(CURSOR_X.load(Ordering::Relaxed)), f32::from_bits(CURSOR_Y.load(Ordering::Relaxed))],
                time: self.time,
                cell: self.cell,
                load: self.load,
                heat: self.heat,
                bg: c4(p.field_bg),
                dim: c4(p.field_dim),
                mid: c4(p.field_mid),
                lit: c4(p.field_lit),
                hover: c4(p.field_hover),
                crest: c4(p.field_crest),
                params: [self.alpha, 0.0, self.intensity, 0.0],
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("oma field"), source: wgpu::ShaderSource::Wgsl(super::shaders::FIELD.into()) });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("oma field uniforms"), size: std::mem::size_of::<Uniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oma field bgl"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("oma field bg"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }] });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oma field pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_main"), targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
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
