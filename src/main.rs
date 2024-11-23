#![allow(dead_code, unused_imports)]
use glyphon::{
    cosmic_text::LineEnding, Attrs, AttrsList, Buffer, BufferLine, Cache, Color, Family,
    FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds,
    TextRenderer, Viewport,
};
use std::{
    fmt::{self, Error},
    sync::Arc,
};
use wgpu::{
    util::DeviceExt, vertex_attr_array, CommandEncoderDescriptor, CompositeAlphaMode,
    DeviceDescriptor, Instance, InstanceDescriptor, LoadOp, MultisampleState, Operations,
    PipelineCompilationOptions, PresentMode, RenderPassColorAttachment, RenderPassDescriptor,
    RequestAdapterOptions, StoreOp, SurfaceConfiguration, TextureFormat, TextureUsages,
    TextureViewDescriptor,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::EventLoop,
    keyboard::{Key, KeyCode, KeyLocation, NamedKey, PhysicalKey},
    window::{CursorGrabMode, Window},
};

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop
        .run_app(&mut Application { window_state: None })
        .unwrap();
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

struct WindowState {
    device: wgpu::Device,
    swash_cache: SwashCache,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: SurfaceConfiguration,
    size: PhysicalSize<u32>,
    scale_factor: f64,
    window: Arc<Window>,

    mouse_pressed: bool,
    strokes: Vec<Vec<Vertex>>,
    current_stroke: Vec<Vertex>,
    current_color: [f32; 4],

    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    stroke_vertex_ranges: Vec<std::ops::Range<u32>>,

    text_input_mode: bool,
    text_position: Option<[f32; 2]>,
    current_text: String,
    text_entries: Vec<(String, [f32; 2])>,
    atlas: glyphon::TextAtlas,
    viewport: glyphon::Viewport,
    font_system: FontSystem,
    text_renderer: TextRenderer,

    cursor_position: Option<PhysicalPosition<f64>>,
}

impl WindowState {
    fn input(&mut self, window: Arc<Window>, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = Some(*position);
                if self.mouse_pressed {
                    let x = position.x as f32 / self.size.width as f32 * 2.0 - 1.0;
                    let y = -(position.y as f32 / self.size.height as f32 * 2.0 - 1.0);

                    if let Some(last_vertex) = self.current_stroke.clone().last() {
                        let dx = x - last_vertex.position[0];
                        let dy = y - last_vertex.position[1];
                        let distance_squared = dx * dx + dy * dy;

                        if distance_squared > 0.01 {
                            let steps = (distance_squared.sqrt() * 10.0).ceil() as usize;
                            for i in 1..steps {
                                let t = i as f32 / steps as f32;
                                self.current_stroke.push(Vertex {
                                    position: [
                                        last_vertex.position[0] + dx * t,
                                        last_vertex.position[1] + dy * t,
                                    ],
                                    color: self.current_color,
                                });
                            }
                        }
                    }

                    self.current_stroke.push(Vertex {
                        position: [x, y],
                        color: self.current_color,
                    });

                    window.request_redraw();
                }
                true
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if *button == MouseButton::Right && *state == ElementState::Pressed {
                    if let Some(position) = self.cursor_position {
                        let x = position.x as f32 / self.size.width as f32 * 2.0 - 1.0;
                        let y = -(position.y as f32 / self.size.height as f32 * 2.0 - 1.0);

                        let mut found_entry = None;
                        for (i, (_, entry_pos)) in self.text_entries.iter().enumerate() {
                            let dx = x - entry_pos[0];
                            let dy = y - entry_pos[1];
                            let distance_squared = dx * dx + dy * dy;

                            if distance_squared < 0.01 {
                                found_entry = Some(i);
                                break;
                            }
                        }

                        if let Some(index) = found_entry {
                            self.text_input_mode = true;
                            self.text_position = Some(self.text_entries[index].1);
                            self.current_text = self.text_entries[index].0.clone();
                        } else {
                            self.text_input_mode = true;
                            self.text_position = Some([x, y]);
                            self.current_text.clear();
                        }
                    }
                }
                true
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.text_input_mode && event.state == ElementState::Pressed {
                    match event.logical_key {
                        Key::Named(NamedKey::Enter) => {
                            if let Some(position) = self.text_position {
                                // Save the current text at the selected position
                                let mut found_entry = false;

                                for (entry_text, entry_pos) in &mut self.text_entries {
                                    if *entry_pos == position {
                                        *entry_text = self.current_text.clone();
                                        found_entry = true;
                                        break;
                                    }
                                }

                                if !found_entry {
                                    // Add a new text entry if it doesn't exist
                                    self.text_entries
                                        .push((self.current_text.clone(), position));
                                }
                            }

                            self.text_input_mode = false;
                            self.text_position = None;
                            self.current_text.clear();
                            window.request_redraw();
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.current_text.pop();
                            window.request_redraw();
                        }
                        _ => {
                            if let Some(ref text) = event.text {
                                self.current_text.push_str(text);
                                window.request_redraw();
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    async fn new(window: Arc<Window>) -> Self {
        let physical_size = window.inner_size();
        let scale_factor = window.scale_factor();
        let instance = Instance::new(InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .expect("Create surface");

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default(), None)
            .await
            .unwrap();

        let swapchain_format = TextureFormat::Bgra8UnormSrgb;
        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: swapchain_format,
            width: physical_size.width,
            height: physical_size.height,
            present_mode: PresentMode::Fifo,
            alpha_mode: CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x4
                    ],
                }],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineStrip,
                strip_index_format: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: &[],
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let mut font_system = FontSystem::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, swapchain_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let swash_cache = SwashCache::new();

        Self {
            device,
            queue,
            surface,
            swash_cache,
            surface_config,
            scale_factor,
            window,
            cursor_position: None,
            viewport,
            size: physical_size,
            mouse_pressed: false,
            render_pipeline,
            vertex_buffer,
            strokes: Vec::new(),
            current_stroke: Vec::new(),
            current_color: [0.0, 0.0, 0.0, 1.0],
            stroke_vertex_ranges: Vec::new(),
            text_input_mode: false,
            text_position: None,
            current_text: String::new(),
            text_entries: Vec::new(),
            font_system,
            text_renderer,
            atlas,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.surface_config.width = self.size.width;
            self.surface_config.height = self.size.height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn update(&mut self) -> Result<(), wgpu::SurfaceError> {
        let mut all_vertices = Vec::new();
        self.stroke_vertex_ranges.clear();
        let mut vertex_count = 0u32;

        for stroke in &self.strokes {
            if !stroke.is_empty() {
                let start = vertex_count;
                all_vertices.extend_from_slice(stroke);
                vertex_count += stroke.len() as u32;
                let end = vertex_count;
                self.stroke_vertex_ranges.push(start..end);
            }
        }

        if !self.current_stroke.is_empty() {
            let start = vertex_count;
            all_vertices.extend_from_slice(&self.current_stroke);
            vertex_count += self.current_stroke.len() as u32;
            let end = vertex_count;
            self.stroke_vertex_ranges.push(start..end);
        }

        if !all_vertices.is_empty() {
            let vertex_data = bytemuck::cast_slice(&all_vertices);
            self.vertex_buffer =
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Vertex Buffer"),
                        contents: vertex_data,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    });
        }

        let mut text_areas: Vec<TextArea> = Vec::new();

        if self.text_input_mode && self.text_position.is_some() {
            if let Some(position) = self.text_position {
                self.text_entries
                    .push((self.current_text.clone(), position));
            }
        }

        let mut full_text = String::new();
        for (text, _) in &self.text_entries {
            full_text.push_str(text);
            full_text.push('\n');
        }

        let attrs = Attrs::new().color(Color::rgb(0, 0, 0));

        let mut text_buffer = Buffer::new(&mut self.font_system, Metrics::new(24.0, 14.0));
        let physical_width = (self.size.width as f64 * self.scale_factor) as f32;
        let physical_height = (self.size.height as f64 * self.scale_factor) as f32;

        text_buffer
            .set_text(&mut self.font_system, &full_text, attrs, Shaping::Advanced);

            text_buffer.set_size(
                &mut self.font_system,
                Some(physical_width),
                Some(physical_height),
            );
            text_buffer.shape_until_scroll(&mut self.font_system, false);

        for (_, position) in &self.text_entries {
            let x = (position[0] + 1.0) / 2.0 * self.size.width as f32;
            let y = (-position[1] + 1.0) / 2.0 * self.size.height as f32;

            let text_area = TextArea {
                buffer: &text_buffer,
                left: x,
                top: y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: self.size.width as i32,
                    bottom: self.size.height as i32,
                },
                default_color: Color::rgb(0, 0, 0),
                custom_glyphs: &[],
            };

            text_areas.push(text_area);
        }

        if let Err(e) = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        ) {
            eprintln!("Glyphon prepare error: {:?}", e);
        }

        Ok(())
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if !self.stroke_vertex_ranges.is_empty() {
                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                for range in &self.stroke_vertex_ranges {
                    render_pass.draw(range.clone(), 0..1);
                }
            }

            if let Err(e) = self
                .text_renderer
                .render(&self.atlas, &self.viewport, &mut render_pass)
            {
                eprintln!("Glyphon render error: {:?}", e);
            }
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();

        Ok(())
    }
}

struct Application {
    window_state: Option<WindowState>,
}

impl ApplicationHandler for Application {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window_state.is_some() {
            return;
        }

        let (width, height) = (800, 600);
        let window_attributes = Window::default_attributes()
            .with_inner_size(LogicalSize::new(width as f64, height as f64))
            .with_title("glyphon hello world");
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        self.window_state = Some(pollster::block_on(WindowState::new(window)));
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(state) = &mut self.window_state {
            let window = &state.window;
            if !state.input(window.clone(), &event) {
                match event {
                    WindowEvent::CloseRequested => event_loop.exit(),
                    WindowEvent::Resized(size) => state.resize(size),
                    _ => {}
                }
            }

            if let WindowEvent::RedrawRequested = event {
                state.viewport.update(
                    &state.queue,
                    Resolution {
                        width: state.surface_config.width,
                        height: state.surface_config.height,
                    },
                );
                let _ = state.update();
                if let Err(e) = state.render() {
                    match e {
                        wgpu::SurfaceError::Lost => state.resize(state.size),
                        wgpu::SurfaceError::OutOfMemory => event_loop.exit(),
                        _ => eprintln!("{:?}", e),
                    }
                }
            }
        }
    }
}
