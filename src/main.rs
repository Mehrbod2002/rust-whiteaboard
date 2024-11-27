#![allow(dead_code, unused_imports)]
use glyphon::{
    cosmic_text::ttf_parser::name::Name, Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::{
    borrow::Borrow,
    collections::HashSet,
    fmt::{self, Error},
    sync::Arc,
    time::{Duration, Instant},
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
    event_loop::{self, ControlFlow, EventLoop},
    keyboard::{Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey},
    window::Window,
};

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop
        .run_app(&mut Application { window_state: None })
        .unwrap();
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, Debug)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

#[derive(Debug, Clone)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
struct TextEntries {
    position: [f32; 2],
    color: [u8; 4],
    text: String,
    pending: bool,
    bounds: Rect,
}

impl TextEntries {
    fn null() -> Self {
        TextEntries {
            position: [0.0, 0.0],
            color: [0, 0, 0, 0],
            text: String::new(),
            pending: true,
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
        }
    }
}

#[derive(Clone, Debug)]
enum Action {
    Stroke(Vec<Vertex>),
    Text(TextEntries),
}

struct WindowState {
    device: wgpu::Device,
    pressed_keys: HashSet<Key>,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: SurfaceConfiguration,
    last_cursor_position: PhysicalPosition<f64>,
    actions: Vec<Action>,
    modifiers: ModifiersState,
    scale_factor: f64,
    size: PhysicalSize<u32>,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: glyphon::Viewport,
    texts: Vec<TextEntries>,
    atlas: glyphon::TextAtlas,
    text_renderer: glyphon::TextRenderer,
    text_buffer: glyphon::Buffer,

    window: Arc<Window>,

    mouse_pressed: bool,
    strokes: Vec<Vec<Vertex>>,
    current_stroke: Vec<Vertex>,
    current_color: [f32; 4],

    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    start_typing: bool,

    cursor_visible: bool,
    cursor_timer: Instant,
    last_click_time: Option<Instant>,
    last_click_position: Option<PhysicalPosition<f64>>,
    editing_text_index: Option<usize>,
    selection_vertex_buffer: Option<wgpu::Buffer>,
}

impl WindowState {
    fn input(&mut self, window: Arc<Window>, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CursorMoved {
                device_id: _,
                position,
            } => {
                self.last_cursor_position = *position;
                if self.mouse_pressed {
                    let x = position.x as f32 / self.size.width as f32 * 2.0 - 1.0;
                    let y = -(position.y as f32 / self.size.height as f32 * 2.0 - 1.0);
                    self.current_stroke.push(Vertex {
                        position: [x, y],
                        color: self.current_color,
                    });

                    window.request_redraw();
                }
                true
            }
            WindowEvent::MouseInput {
                device_id: _,
                state,
                button,
            } => {
                if *button == MouseButton::Right {
                    if *state == ElementState::Pressed {
                        let now = Instant::now();
                        let position = self.last_cursor_position;

                        let mut double_click_detected = false;

                        if let Some(last_click_time) = self.last_click_time {
                            if now.duration_since(last_click_time) <= DOUBLE_CLICK_THRESHOLD {
                                if let Some(last_click_position) = self.last_click_position {
                                    let dx = position.x - last_click_position.x;
                                    let dy = position.y - last_click_position.y;
                                    let distance_squared = dx * dx + dy * dy;
                                    if distance_squared
                                        <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
                                    {
                                        double_click_detected = true;
                                    }
                                }
                            }
                        }

                        if double_click_detected {
                            let mut text_found = false;
                            for (i, text_entry) in self.texts.iter().enumerate() {
                                let bounds = &text_entry.bounds;
                                if position.x >= bounds.x as f64
                                    && position.x <= (bounds.x + bounds.width) as f64
                                    && position.y >= bounds.y as f64
                                    && position.y <= (bounds.y + bounds.height) as f64
                                {
                                    self.editing_text_index = Some(i);
                                    self.start_typing = true;
                                    window.request_redraw();
                                    text_found = true;
                                    break;
                                }
                            }
                            if !text_found {
                                let mut new_text_entry = TextEntries::null();
                                let x = position.x as f32;
                                let y = position.y as f32;
                                new_text_entry.position = [x, y];
                                new_text_entry.color = [0, 0, 0, 255];
                                self.texts.push(new_text_entry);
                                self.editing_text_index = Some(self.texts.len() - 1);
                                self.start_typing = true;
                                window.request_redraw();
                            }
                        }

                        self.last_click_time = Some(now);
                        self.last_click_position = Some(position);

                        if self.start_typing {
                            self.start_typing = false;
                            if let Some(text) = self.texts.last_mut() {
                                text.pending = false;
                                self.actions.push(Action::Text(text.clone()));
                            }
                        } else {
                            self.start_typing = true;
                            self.texts.push(TextEntries::null());
                            let position = self.last_cursor_position;
                            let x = position.x as f32;
                            let y = position.y as f32;
                            if let Some(text) = self.texts.last_mut() {
                                text.position = [x, y];
                            }
                        }
                    }
                }
                if *button == MouseButton::Left {
                    if *state == ElementState::Pressed {
                        self.mouse_pressed = true;
                        self.current_stroke = Vec::new();
                    } else {
                        self.mouse_pressed = false;
                        if !self.current_stroke.is_empty() {
                            self.strokes.push(self.current_stroke.clone());
                            self.actions
                                .push(Action::Stroke(self.current_stroke.clone()));
                            self.current_stroke.clear();
                        }
                        window.request_redraw();
                    }
                }
                true
            }
            WindowEvent::KeyboardInput { event, .. } => {
                match event.state {
                    ElementState::Pressed => {
                        self.pressed_keys.insert(event.logical_key.clone());

                        if self.start_typing {
                            if let Some(text_input) = &event.text {
                                if let Some(text) = self.texts.last_mut() {
                                    if text.pending {
                                        text.text.push_str(text_input);
                                        window.request_redraw();
                                    }
                                }
                            }
                            if let Key::Named(key) = event.logical_key {
                                match key {
                                    NamedKey::Enter => {
                                        self.start_typing = false;
                                        if let Some(text) = self.texts.last_mut() {
                                            text.pending = false;
                                            self.actions.push(Action::Text(text.clone()));
                                        }
                                        window.request_redraw();
                                    }
                                    NamedKey::GoBack => {
                                        self.start_typing = false;
                                        if let Some(text) = self.texts.last_mut() {
                                            text.pending = false;
                                            self.actions.push(Action::Text(text.clone()));
                                        }
                                        window.request_redraw();
                                    }
                                    NamedKey::Backspace => {
                                        if let Some(text) = self.texts.last_mut() {
                                            if text.pending {
                                                if text.text.chars().count() > 1 {
                                                    text.text = text
                                                        .text
                                                        .chars()
                                                        .take(text.text.chars().count() - 2)
                                                        .collect();
                                                    window.request_redraw();
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        } else {
                            if self.pressed_keys.contains(&Key::Named(NamedKey::Control))
                                && self
                                    .pressed_keys
                                    .contains(&Key::Character("z".to_string().into()))
                            {
                                if let Some(action) = self.actions.pop() {
                                    match action {
                                        Action::Stroke(_) => {
                                            self.strokes.pop();
                                        }
                                        Action::Text(_) => {
                                            self.texts.pop();
                                        }
                                    }
                                }
                                window.request_redraw();
                                return true;
                            }
                            if let Some(ref text) = event.text {
                                match text.as_str() {
                                    "1" => self.current_color = [1.0, 0.0, 0.0, 1.0], // Red
                                    "2" => self.current_color = [0.0, 1.0, 0.0, 1.0], // Green
                                    "3" => self.current_color = [0.0, 0.0, 1.0, 1.0], // Blue
                                    "4" => self.current_color = [1.0, 1.0, 0.0, 1.0], // Yellow
                                    "5" => self.current_color = [1.0, 0.0, 1.0, 1.0], // Magenta
                                    "6" => self.current_color = [0.0, 1.0, 1.0, 1.0], // Cyan
                                    "7" => self.current_color = [0.0, 0.0, 0.0, 1.0], // Black
                                    "8" => self.current_color = [1.0, 1.0, 1.0, 1.0], // White
                                    _ => (),
                                }
                            }
                        }
                    }
                    ElementState::Released => {
                        self.pressed_keys.remove(&event.logical_key);
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

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, swapchain_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

        let physical_width = (physical_size.width as f64 * scale_factor) as f32;
        let physical_height = (physical_size.height as f64 * scale_factor) as f32;

        text_buffer.set_size(
            &mut font_system,
            Some(physical_width),
            Some(physical_height),
        );
        text_buffer.shape_until_scroll(&mut font_system, false);

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
                topology: wgpu::PrimitiveTopology::LineList,
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

        Self {
            device,
            last_cursor_position: PhysicalPosition::new(0.0, 0.0),
            queue,
            scale_factor,
            surface,
            actions: Vec::new(),
            modifiers: ModifiersState::default(),
            pressed_keys: HashSet::new(),
            surface_config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,
            window,
            size: physical_size,
            mouse_pressed: false,
            render_pipeline,
            vertex_buffer,
            strokes: Vec::new(),
            current_stroke: Vec::new(),
            current_color: [0.0, 0.0, 0.0, 1.0],
            start_typing: false,
            texts: Vec::new(),
            cursor_visible: false,
            cursor_timer: Instant::now(),
            last_click_time: None,
            last_click_position: None,
            editing_text_index: None,
            selection_vertex_buffer: None,
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
        let mut buffers: Vec<glyphon::Buffer> = Vec::new();
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut all_vertices = Vec::new();

        self.actions.iter().for_each(|x| match x {
            Action::Stroke(stroke) => {
                if stroke.len() >= 2 {
                    for i in 0..(stroke.len() - 1) {
                        all_vertices.push(stroke[i]);
                        all_vertices.push(stroke[i + 1]);
                    }
                }
            }
            Action::Text(text) => {
                let mut buffer = self.text_buffer.clone();
                buffer.set_text(
                    &mut self.font_system,
                    &text.text,
                    Attrs::new().family(Family::SansSerif),
                    Shaping::Advanced,
                );

                buffer.shape_until_scroll(&mut self.font_system, false);
                buffers.push(buffer);
            }
        });

        const CURSOR_BLINK_INTERVAL: f32 = 0.5;

        if self.start_typing {
            let elapsed = self.cursor_timer.elapsed().as_secs_f32();
            if elapsed >= CURSOR_BLINK_INTERVAL {
                self.cursor_visible = !self.cursor_visible;
                self.cursor_timer = Instant::now();
                self.window.request_redraw();
            }
        }

        for (index, (text_entry, buffer)) in self.texts.iter_mut().zip(buffers.iter()).enumerate() {
            let x = text_entry.position[0];
            let y = text_entry.position[1];

            let mut min_x = f32::MAX;
            let mut min_y = f32::MAX;
            let mut max_x = f32::MIN;
            let mut max_y = f32::MIN;

            for layout_run in buffer.layout_runs() {
                for glyph in layout_run.glyphs {
                    let glyph_x = glyph.x;
                    let glyph_y = glyph.y;
                    let glyph_w = glyph.w;
                    let glyph_h = glyph.x;

                    min_x = min_x.min(glyph_x);
                    min_y = min_y.min(glyph_y);
                    max_x = max_x.max(glyph_x + glyph_w);
                    max_y = max_y.max(glyph_y + glyph_h);
                }
            }

            let width = max_x - min_x;
            let height = max_y - min_y;

            text_entry.bounds = Rect {
                x,
                y,
                width,
                height,
            };

            let text_bounds = TextBounds {
                left: 0,
                top: 0,
                right: self.size.width as i32,
                bottom: self.size.height as i32,
            };

            let default_color = if Some(index) == self.editing_text_index {
                Color::rgb(0, 0, 255)
            } else {
                Color::rgb(
                    text_entry.color[0],
                    text_entry.color[1],
                    text_entry.color[2],
                )
            };

            text_areas.push(TextArea {
                buffer,
                left: x,
                top: y,
                scale: 1.0,
                bounds: text_bounds,
                default_color,
                custom_glyphs: &[],
            });
        }

        let _ = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        );

        if self.current_stroke.len() >= 2 {
            for i in 0..(self.current_stroke.len() - 1) {
                all_vertices.push(self.current_stroke[i]);
                all_vertices.push(self.current_stroke[i + 1]);
            }
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

        let mut buffers: Vec<glyphon::Buffer> = Vec::new();

        for text_entry in &self.texts {
            let mut buffer = self.text_buffer.clone();
            let mut text = text_entry.text.clone();
            if text_entry.pending && self.start_typing && self.cursor_visible {
                text.push('|');
            }

            buffer.set_text(
                &mut self.font_system,
                &text,
                Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
            );

            buffers.push(buffer);
        }

        let mut text_areas: Vec<TextArea> = Vec::new();

        for (text_entry, buffer) in self.texts.iter().zip(buffers.iter()) {
            let x = text_entry.position[0];
            let y = text_entry.position[1];

            let text_bounds = TextBounds {
                left: 0,
                top: 0,
                right: self.size.width as i32,
                bottom: self.size.height as i32,
            };

            let default_color = Color::rgb(
                text_entry.color[0],
                text_entry.color[1],
                text_entry.color[2],
            );

            text_areas.push(TextArea {
                buffer,
                left: x,
                top: y,
                scale: 1.0,
                bounds: text_bounds,
                default_color,
                custom_glyphs: &[],
            });
        }

        let _ = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        );

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
                label: Some("Strokes Render Pass"),
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

            if self.vertex_buffer.size() > 0 {
                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(
                    0..(self.vertex_buffer.size() as u32 / std::mem::size_of::<Vertex>() as u32),
                    0..1,
                );
            }
        }

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Text Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut render_pass)
                .unwrap();
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        self.atlas.trim();

        Ok(())
    }
}

struct Application {
    window_state: Option<WindowState>,
}

const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f64 = 5.0;

impl ApplicationHandler for Application {
    fn about_to_wait(&mut self, _: &event_loop::ActiveEventLoop) {
        let Some(state) = &mut self.window_state else {
            return;
        };

        const CURSOR_BLINK_INTERVAL: f32 = 0.5;

        if state.start_typing {
            if state.cursor_timer.elapsed().as_secs_f32() >= CURSOR_BLINK_INTERVAL {
                state.cursor_visible = !state.cursor_visible;
                state.cursor_timer = Instant::now();
                state.window.request_redraw();
            }
        }
    }

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
        event_loop.set_control_flow(ControlFlow::Poll);
        let Some(state) = &mut self.window_state else {
            return;
        };

        let window = &state.window;
        if !state.input(window.clone(), &event) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => state.resize(size),
                _ => {}
            }
        }
        match event {
            WindowEvent::RedrawRequested => {
                state.viewport.update(
                    &state.queue,
                    Resolution {
                        width: state.surface_config.width,
                        height: state.surface_config.height,
                    },
                );
                let _ = state.update();
                match state.render() {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
                    Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                    Err(e) => eprintln!("{:?}", e),
                }
            }
            WindowEvent::Focused(_) => {
                let Some(state) = &mut self.window_state else {
                    return;
                };

                const CURSOR_BLINK_INTERVAL: f32 = 0.5;

                if state.start_typing {
                    if state.cursor_timer.elapsed().as_secs_f32() >= CURSOR_BLINK_INTERVAL {
                        state.cursor_visible = !state.cursor_visible;
                        state.cursor_timer = Instant::now();
                        state.window.request_redraw();
                    }
                }
            }
            _ => (),
        }
    }
}
