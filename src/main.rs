#![allow(dead_code, unused_imports)]
mod ui;
use egui::{Color32, Ui, Vec2};
use egui_wgpu::{
    wgpu::{
        util::DeviceExt, vertex_attr_array, CommandEncoderDescriptor, CompositeAlphaMode,
        DeviceDescriptor, FragmentState, Instance, InstanceDescriptor, LoadOp, MultisampleState,
        Operations, PipelineCompilationOptions, PresentMode, PrimitiveState,
        RenderPassColorAttachment, RenderPassDescriptor, RequestAdapterOptions,
        ShaderModuleDescriptor, StoreOp, SurfaceConfiguration, TextureFormat, TextureUsages,
        TextureViewDescriptor, VertexBufferLayout, VertexState,
    },
    ScreenDescriptor,
};
use glyphon::{
    cosmic_text::ttf_parser::name::Name, Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::{
    borrow::{Borrow, BorrowMut},
    collections::HashSet,
    fmt::{self, Error},
    sync::Arc,
    time::{Duration, Instant},
};
use ui::EguiRenderer;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{self, ControlFlow, EventLoop},
    keyboard::{Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey, SmolStr},
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

#[derive(Clone, Debug)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, Debug)]
struct Rectangle {
    first: [f32; 2],
    last: [f32; 2],
    color: [f32; 4],
}

impl Rectangle {
    fn to_vertices(self) -> Vec<Vertex> {
        let (x1, y1) = (self.first[0], self.first[1]);
        let (x2, y2) = (self.last[0], self.last[1]);

        vec![
            Vertex {
                position: [x1, y2],
                color: self.color,
            },
            Vertex {
                position: [x2, y2],
                color: self.color,
            },
            Vertex {
                position: [x2, y2],
                color: self.color,
            },
            Vertex {
                position: [x2, y1],
                color: self.color,
            },
            Vertex {
                position: [x2, y1],
                color: self.color,
            },
            Vertex {
                position: [x1, y1],
                color: self.color,
            },
            Vertex {
                position: [x1, y1],
                color: self.color,
            },
            Vertex {
                position: [x1, y2],
                color: self.color,
            },
        ]
    }
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
    fn null(color: [u8; 4]) -> Self {
        TextEntries {
            position: [0.0, 0.0],
            color,
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
    Shapes(Rectangle),
}

struct WindowState {
    device: egui_wgpu::wgpu::Device,
    pressed_keys: HashSet<Key>,
    egui_rendererd: bool,
    queue: egui_wgpu::wgpu::Queue,
    surface: egui_wgpu::wgpu::Surface<'static>,
    surface_config: SurfaceConfiguration,
    last_cursor_position: PhysicalPosition<f64>,
    actions: Vec<Action>,
    modifiers: ModifiersState,
    scale_factor: f64,
    egui_renderer: EguiRenderer,
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

    render_pipeline: egui_wgpu::wgpu::RenderPipeline,
    rectangle_shader: Option<egui_wgpu::wgpu::RenderPipeline>,
    vertex_buffer: egui_wgpu::wgpu::Buffer,
    start_typing: bool,
    shape_positions: Vec<Vertex>,
    shapes: Vec<Rectangle>,
    create_rect: bool,
    cursor_visible: bool,
    cursor_timer: Instant,
    last_click_time: Option<Instant>,
    last_click_position: Option<PhysicalPosition<f64>>,
    editing_text_index: Option<usize>,
    selection_vertex_buffer: Option<egui_wgpu::wgpu::Buffer>,
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
                    if self.create_rect {
                        if self.shape_positions.is_empty() {
                            self.shape_positions.push(Vertex {
                                position: [x, y],
                                color: self.current_color,
                            });
                        } else {
                            if self.shape_positions.len() > 1 {
                                self.shape_positions.pop();
                            }
                            self.shape_positions.push(Vertex {
                                position: [x, y],
                                color: self.current_color,
                            });
                        }
                    } else {
                        self.current_stroke.push(Vertex {
                            position: [x, y],
                            color: self.current_color,
                        });
                    }

                    window.request_redraw();
                }
                true
            }
            WindowEvent::MouseInput {
                device_id: _,
                state,
                button,
            } => {
                if *button == MouseButton::Right && *state == ElementState::Pressed {
                    let now = Instant::now();
                    let position = self.last_cursor_position;

                    let mut double_click_detected = false;

                    if let Some(last_click_time) = self.last_click_time {
                        if now.duration_since(last_click_time) <= DOUBLE_CLICK_THRESHOLD {
                            if let Some(last_click_position) = self.last_click_position {
                                let dx = position.x - last_click_position.x;
                                let dy = position.y - last_click_position.y;
                                let distance_squared = dx * dx + dy * dy;
                                if distance_squared <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
                                {
                                    double_click_detected = true;
                                }
                            }
                        }
                    }

                    if double_click_detected {
                        for (i, text_entry) in self.texts.iter_mut().enumerate() {
                            let bounds = &text_entry.bounds;
                            if position.x >= bounds.x as f64
                                && position.x <= (bounds.x + bounds.width) as f64
                                && position.y >= bounds.y as f64
                                && position.y <= (bounds.y + bounds.height) as f64
                            {
                                self.editing_text_index = Some(i);
                                self.start_typing = true;
                                text_entry.pending = true;
                                window.request_redraw();

                                break;
                            }
                        }
                    }

                    self.last_click_time = Some(now);
                    self.last_click_position = Some(position);

                    if self.start_typing && self.editing_text_index.is_none() {
                        self.start_typing = false;
                        if let Some(text) = self.texts.last_mut() {
                            text.pending = false;
                            self.actions.push(Action::Text(text.clone()));
                        }
                    } else {
                        self.start_typing = true;
                        self.texts
                            .push(TextEntries::null(normalized_to_rgba(self.current_color)));
                        let position = self.last_cursor_position;
                        let x = position.x as f32;
                        let y = position.y as f32;
                        if let Some(text) = self.texts.last_mut() {
                            text.position = [x, y];
                        }
                    }
                }
                if *button == MouseButton::Left {
                    if *state == ElementState::Pressed {
                        self.mouse_pressed = true;
                        self.current_stroke = Vec::new();

                        if self
                            .pressed_keys
                            .contains(&Key::Character(SmolStr::new("s")))
                        {
                            self.create_rect = true;
                        }
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

                        if self.start_typing || self.editing_text_index.is_some() {
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
                                        self.editing_text_index = None;
                                        if let Some(text) = self.texts.last_mut() {
                                            text.pending = false;
                                            self.actions.push(Action::Text(text.clone()));
                                        }
                                        window.request_redraw();
                                    }
                                    NamedKey::Delete => {
                                        let text_entry =
                                            if let Some(index) = self.editing_text_index {
                                                self.texts.get_mut(index)
                                            } else {
                                                self.texts.last_mut()
                                            };
                                        if let Some(entry) = text_entry {
                                            entry.text.pop();
                                            window.request_redraw();
                                        }
                                    }
                                    NamedKey::GoBack => {
                                        self.start_typing = false;
                                        self.editing_text_index = None;
                                        if let Some(text) = self.texts.last_mut() {
                                            text.pending = false;
                                            self.actions.push(Action::Text(text.clone()));
                                        }
                                        window.request_redraw();
                                    }
                                    NamedKey::Backspace => {
                                        if self.editing_text_index.is_some() {
                                            let editing_text = self.texts
                                                [self.editing_text_index.unwrap()]
                                            .borrow_mut();
                                            if editing_text.pending
                                                && editing_text.text.chars().count() > 1
                                            {
                                                editing_text.text = editing_text
                                                    .text
                                                    .chars()
                                                    .take(editing_text.text.chars().count() - 2)
                                                    .collect();
                                                window.request_redraw();
                                            }
                                        } else if let Some(text) = self.texts.last_mut() {
                                            if text.pending && text.text.chars().count() > 1 {
                                                text.text = text
                                                    .text
                                                    .chars()
                                                    .take(text.text.chars().count() - 2)
                                                    .collect();
                                                window.request_redraw();
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
                                        Action::Shapes(_) => {
                                            self.shapes.pop();
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
                        self.create_rect = false;

                        if let (Some(first), Some(last)) =
                            (self.shape_positions.first(), self.shape_positions.last())
                        {
                            let rectangle = Rectangle {
                                first: first.position,
                                last: last.position,
                                color: self.current_color,
                            };

                            self.actions.push(Action::Shapes(rectangle));
                            self.shapes.push(rectangle);
                        }

                        self.shape_positions.clear();
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
            height: (physical_size.height as f32 * 0.8) as u32,
            present_mode: PresentMode::Fifo,
            alpha_mode: CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        let egui_renderer = EguiRenderer::new(&window, &device, surface_config.format, None, 1);
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

        let shader = device.create_shader_module(egui_wgpu::wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: egui_wgpu::wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout =
            device.create_pipeline_layout(&egui_wgpu::wgpu::PipelineLayoutDescriptor {
                label: Some("Pipeline Layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

        let shader_shape = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("rect shader"),
            source: egui_wgpu::wgpu::ShaderSource::Wgsl(include_str!("shaders/shape.wgsl").into()),
        });
        let rectangle_shader =
            device.create_render_pipeline(&egui_wgpu::wgpu::RenderPipelineDescriptor {
                label: Some("rect pipline"),
                layout: Some(&pipeline_layout),
                vertex: egui_wgpu::wgpu::VertexState {
                    module: &shader_shape,
                    entry_point: "rectangle_vs",
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as egui_wgpu::wgpu::BufferAddress,
                        step_mode: egui_wgpu::wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            egui_wgpu::wgpu::VertexAttribute {
                                format: egui_wgpu::wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0,
                            },
                            egui_wgpu::wgpu::VertexAttribute {
                                format: egui_wgpu::wgpu::VertexFormat::Float32x4,
                                offset: std::mem::size_of::<[f32; 2]>()
                                    as egui_wgpu::wgpu::BufferAddress,
                                shader_location: 1,
                            },
                        ],
                    }],
                },
                primitive: PrimitiveState {
                    topology: egui_wgpu::wgpu::PrimitiveTopology::LineList,
                    strip_index_format: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &shader_shape,
                    entry_point: "fs_main",
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(egui_wgpu::wgpu::ColorTargetState {
                        format: surface_config.format,
                        blend: Some(egui_wgpu::wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: egui_wgpu::wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });

        let render_pipeline =
            device.create_render_pipeline(&egui_wgpu::wgpu::RenderPipelineDescriptor {
                label: Some("Render Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: egui_wgpu::wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[egui_wgpu::wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>()
                            as egui_wgpu::wgpu::BufferAddress,
                        step_mode: egui_wgpu::wgpu::VertexStepMode::Vertex,
                        attributes: &vertex_attr_array![
                            0 => Float32x2,
                            1 => Float32x4
                        ],
                    }],
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(egui_wgpu::wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(egui_wgpu::wgpu::ColorTargetState {
                        format: surface_config.format,
                        blend: Some(egui_wgpu::wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: egui_wgpu::wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: PipelineCompilationOptions::default(),
                }),
                primitive: egui_wgpu::wgpu::PrimitiveState {
                    topology: egui_wgpu::wgpu::PrimitiveTopology::LineList,
                    strip_index_format: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: egui_wgpu::wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let vertex_buffer =
            device.create_buffer_init(&egui_wgpu::wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer"),
                contents: &[],
                usage: egui_wgpu::wgpu::BufferUsages::VERTEX
                    | egui_wgpu::wgpu::BufferUsages::COPY_DST,
            });

        let mut render_self = Self {
            device,
            shapes: Vec::new(),
            last_cursor_position: PhysicalPosition::new(0.0, 0.0),
            queue,
            egui_rendererd: false,
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
            texts: Vec::new(),
            create_rect: false,
            window,
            size: physical_size,
            mouse_pressed: false,
            render_pipeline,
            vertex_buffer,
            strokes: Vec::new(),
            current_stroke: Vec::new(),
            current_color: [0.0, 0.0, 0.0, 1.0],
            start_typing: false,
            cursor_visible: false,
            cursor_timer: Instant::now(),
            last_click_time: None,
            last_click_position: None,
            editing_text_index: None,
            selection_vertex_buffer: None,
            rectangle_shader: Some(rectangle_shader),
            shape_positions: Vec::new(),
            egui_renderer,
        };

        let _ = Self::render(&mut render_self);
        render_self
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.surface_config.width = self.size.width;
            self.surface_config.height = self.size.height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn update(&mut self) -> Result<(), egui_wgpu::wgpu::SurfaceError> {
        let buffers: Vec<glyphon::Buffer> = Vec::new();
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut all_vertices = Vec::new();

        self.actions.iter().for_each(|x| {
            if let Action::Stroke(stroke) = x {
                if stroke.len() >= 2 {
                    for i in 0..(stroke.len() - 1) {
                        all_vertices.push(stroke[i]);
                        all_vertices.push(stroke[i + 1]);
                    }
                }
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

            let normalized_color = normalized_to_rgba(self.current_color);
            let default_color = if Some(index) == self.editing_text_index {
                Color::rgb(0, 0, 255)
            } else {
                Color::rgba(
                    normalized_color[0],
                    normalized_color[1],
                    normalized_color[2],
                    normalized_color[3],
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
                    .create_buffer_init(&egui_wgpu::wgpu::util::BufferInitDescriptor {
                        label: Some("Vertex Buffer"),
                        contents: vertex_data,
                        usage: egui_wgpu::wgpu::BufferUsages::VERTEX
                            | egui_wgpu::wgpu::BufferUsages::COPY_DST,
                    });
        }

        let mut buffers: Vec<glyphon::Buffer> = Vec::new();

        for text_entry in self.texts.iter() {
            let mut buffer = self.text_buffer.clone();
            let mut text = text_entry.text.clone();
            if text_entry.pending && self.cursor_visible {
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

            let default_color = Color::rgba(
                text_entry.color[0],
                text_entry.color[1],
                text_entry.color[2],
                text_entry.color[3],
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

        self.egui_renderer.begin_pass(&self.window);

        Ok(())
    }

    fn render(&mut self) -> Result<(), egui_wgpu::wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&egui_wgpu::wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.device
                .create_command_encoder(&egui_wgpu::wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

        {
            let encoder = encoder.borrow_mut();
            let mut render_pass =
                encoder
                    .borrow_mut()
                    .begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                        label: Some("Strokes Render Pass"),
                        color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: egui_wgpu::wgpu::Operations {
                                load: egui_wgpu::wgpu::LoadOp::Clear(egui_wgpu::wgpu::Color::WHITE),
                                store: egui_wgpu::wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

            if let Some(rectangle_shader) = &self.rectangle_shader {
                let mut temp_shapes = self.shapes.clone();

                if self.create_rect {
                    if let (Some(first), Some(last)) =
                        (&self.shape_positions.first(), &self.shape_positions.last())
                    {
                        let rectangle = Rectangle {
                            first: first.position,
                            last: last.position,
                            color: self.current_color,
                        };

                        temp_shapes.push(rectangle);
                    }
                }

                let flattened_shapes: Vec<_> = temp_shapes
                    .iter()
                    .flat_map(|rect| rect.to_vertices())
                    .collect();

                if !flattened_shapes.is_empty() {
                    let rectangle_vertex_buffer = self.device.create_buffer_init(
                        &egui_wgpu::wgpu::util::BufferInitDescriptor {
                            label: Some("Rectangle Vertex Buffer"),
                            contents: bytemuck::cast_slice(&flattened_shapes),
                            usage: egui_wgpu::wgpu::BufferUsages::VERTEX,
                        },
                    );

                    render_pass.set_pipeline(rectangle_shader);
                    render_pass.set_vertex_buffer(0, rectangle_vertex_buffer.slice(..));
                    render_pass.draw(0..flattened_shapes.len() as u32, 0..1);
                }
            }

            if self.vertex_buffer.size() > 0 {
                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(
                    0..(self.vertex_buffer.size() as u32 / std::mem::size_of::<Vertex>() as u32),
                    0..1,
                );
            }

            let screen_descriptor = ScreenDescriptor {
                size_in_pixels: [
                    self.surface_config.width,
                    (self.surface_config.height as f32 * 0.2) as u32,
                ],
                pixels_per_point: (self.window.as_ref().scale_factor() * self.scale_factor) as f32,
            };

            self.egui_renderer.begin_pass(&self.window);

            let header_height = self.surface_config.height as f32 * 0.2;
            let header_width = self.surface_config.width as f32;

            let menu_color = egui::Color32::from_hex("#5C5C5C").expect("unable to get color");
            egui::Area::new("Header".into())
                .fixed_pos([0.0, 0.0])
                .movable(false)
                .default_size([header_width, header_height])
                .show(self.egui_renderer.context(), |ui| {
                    let custom_frame = egui::Frame::none()
                        .fill(menu_color)
                        .stroke(egui::Stroke::new(1.0, menu_color));
                    custom_frame.show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.add_space(5.0);
                            ui.horizontal(|ui| {
                                ui.set_width(ui.available_width());

                                ui.add_space(ui.available_width() * 0.18);
                                for _ in 0..5 {
                                    if ui.button("Settings").clicked() {
                                        println!("Settings clicked");
                                    }
                                }
                            });

                            ui.add_space(5.0);
                        });
                    });
                });

            let mut encoder_egui =
                self.device
                    .create_command_encoder(&egui_wgpu::wgpu::CommandEncoderDescriptor {
                        label: Some("Render Encoder"),
                    });

            self.egui_renderer.end_frame_and_draw(
                &self.device,
                &self.queue,
                render_pass,
                &mut encoder_egui,
                &self.window,
                &view,
                screen_descriptor,
            );
        }

        {
            let mut render_pass =
                encoder.begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
                    label: Some("Text Render Pass"),
                    color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: egui_wgpu::wgpu::Operations {
                            load: egui_wgpu::wgpu::LoadOp::Load,
                            store: egui_wgpu::wgpu::StoreOp::Store,
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

        if state.start_typing && state.cursor_timer.elapsed().as_secs_f32() >= CURSOR_BLINK_INTERVAL
        {
            state.cursor_visible = !state.cursor_visible;
            state.cursor_timer = Instant::now();
            state.window.request_redraw();
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
        // event_loop.set_control_flow(ControlFlow::Poll);
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
        state
            .egui_renderer
            .handle_input(state.window.as_ref(), &event);
        match event {
            WindowEvent::RedrawRequested => {
                state.viewport.update(
                    &state.queue,
                    Resolution {
                        width: state.surface_config.width,
                        height: (state.surface_config.height as f32 * 0.8) as u32,
                    },
                );
                let _ = state.update();
                match state.render() {
                    Ok(_) => {}
                    Err(egui_wgpu::wgpu::SurfaceError::Lost) => state.resize(state.size),
                    Err(egui_wgpu::wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                    Err(e) => eprintln!("{:?}", e),
                }
            }
            WindowEvent::Focused(_) => {
                let Some(state) = &mut self.window_state else {
                    return;
                };

                const CURSOR_BLINK_INTERVAL: f32 = 0.5;

                if state.start_typing
                    && state.cursor_timer.elapsed().as_secs_f32() >= CURSOR_BLINK_INTERVAL
                {
                    state.cursor_visible = !state.cursor_visible;
                    state.cursor_timer = Instant::now();
                    state.window.request_redraw();
                }
            }
            _ => (),
        }
    }
}

fn normalized_to_rgba(normalized: [f32; 4]) -> [u8; 4] {
    let red = (normalized[0] * 255.0) as u8;
    let green = (normalized[1] * 255.0) as u8;
    let blue = (normalized[2] * 255.0) as u8;
    let alpha = (normalized[3] * 255.0) as u8;
    [red, green, blue, alpha]
}

fn color_picker_popup(ui: &mut Ui, current_color: &mut Color32) {
    let colors = [
        ("Red", Color32::from_rgb(255, 0, 0)),
        ("Green", Color32::from_rgb(0, 255, 0)),
        ("Blue", Color32::from_rgb(0, 0, 255)),
        ("Yellow", Color32::from_rgb(255, 255, 0)),
        ("Black", Color32::from_rgb(0, 0, 0)),
        ("White", Color32::from_rgb(255, 255, 255)),
    ];

    ui.label("Pick a color:");
    for (name, color) in colors.iter() {
        if ui
            .selectable_label(*current_color == *color, *name)
            .clicked()
        {
            *current_color = *color;
            ui.close_menu(); // Close the popup when a color is selected
        }
    }
}

fn modal_color_picker(ctx: &egui::Context, current_color: &mut egui::Color32) {
    // Unique ID for the popup
    let popup_id = egui::Id::new("color_picker_popup");

    egui::CentralPanel::default().show(ctx, |ui| {
        if ui
            .button(format!("Select Color (Currently: {:?})", "red"))
            .clicked()
        {}

        egui::popup::show_tooltip(ctx, ui.layer_id(), popup_id, |ui| {
            ui.set_min_width(150.0);
            color_picker_popup(ui, current_color);
        });

        // Show the currently selected color
        ui.horizontal(|ui| {
            ui.label("Selected Color:");
            ui.colored_label(*current_color, format!("{:?}", current_color));
        });
    });
}
