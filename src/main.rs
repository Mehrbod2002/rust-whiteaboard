#![allow(dead_code)]

use egui::{
    include_image, Align2, Color32, Context, Event as EventEgui, Image, ImageButton, ImageSource,
    Key as KeyEgui, RawInput,
};
use egui_wgpu::{
    wgpu::{
        self, util::DeviceExt, vertex_attr_array, CompositeAlphaMode, DeviceDescriptor,
        FragmentState, Instance, InstanceDescriptor, MultisampleState, PipelineCompilationOptions,
        PresentMode, PrimitiveState, RequestAdapterOptions, ShaderModuleDescriptor, StoreOp,
        SurfaceConfiguration, TextureFormat, TextureUsages, VertexBufferLayout,
    },
    Renderer, ScreenDescriptor,
};
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::{
    borrow::BorrowMut,
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, Event, MouseButton, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::Key,
    rwh_06::HasWindowHandle,
    window::{Window, WindowId},
};

fn main() {
    let event_loop = EventLoop::new();

    let window = Window::new(&event_loop).unwrap_or_else(|err| {
        panic!("Error occurred: {:?}", err);
    });

    window.set_title("وایت برد");
    let window = Arc::new(window);

    let mut app = Application {
        window_state: Some(pollster::block_on(WindowState::new(window))),
    };

    event_loop.run(move |event, _, control_flow| {
        let Some(state) = &mut app.window_state else {
            return;
        };
        match event {
            Event::MainEventsCleared => {
                app.about_to_wait();
            }
            Event::WindowEvent {
                window_id, event, ..
            } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                _ => {
                    app.window_event(window_id, event);
                }
            },
            // Event::Resumed => {
            //     if app.window_state.is_some() {
            //         return;
            //     }

            //     let window = Window::new(&event_loop).unwrap_or_else(|err| {
            //         eprintln!("error occurs {:?}", err);
            //         panic!("error occures");
            //     });

            //     let window = Arc::new(window);
            //     app.window_state = Some(pollster::block_on(WindowState::new(window)));
            // }
            // Event::MainEventsCleared => *control_flow = ControlFlow::Exit,
            Event::RedrawRequested(_window_id) => {
                state.viewport.update(
                    &state.queue,
                    Resolution {
                        width: state.size.width,
                        height: state.size.height,
                    },
                );
                let _ = state.update();
                match state.render() {
                    Ok(_) => {}
                    Err(egui_wgpu::wgpu::SurfaceError::Lost) => state.resize(state.size),
                    Err(egui_wgpu::wgpu::SurfaceError::OutOfMemory) => {
                        *control_flow = ControlFlow::Exit
                    }
                    Err(e) => eprintln!("{:?}", e),
                }
            }
            Event::LoopDestroyed => *control_flow = ControlFlow::Exit,
            _ => (),
        }
    });
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
    font_size: i32,
}

impl TextEntries {
    fn null(color: [u8; 4], font_size: i32) -> Self {
        TextEntries {
            font_size,
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

struct WindowState<'a> {
    device: egui_wgpu::wgpu::Device,
    pressed_keys: HashSet<Key<'a>>,
    queue: egui_wgpu::wgpu::Queue,
    show_modal_fonts: bool,
    font_size: i32,
    show_modal_colors: bool,
    surface: egui_wgpu::wgpu::Surface<'static>,
    surface_config: SurfaceConfiguration,
    last_cursor_position: PhysicalPosition<f64>,
    actions: Vec<Action>,
    scale_factor: f64,
    egui_renderer: Renderer,
    raw_input: RawInput,
    egui_context: Context,
    size: PhysicalSize<u32>,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: glyphon::Viewport,
    texts: Vec<TextEntries>,
    atlas: glyphon::TextAtlas,
    text_renderer: glyphon::TextRenderer,
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

    color: ImageSource<'static>,
    rect: ImageSource<'static>,
    prev: ImageSource<'static>,
    font: ImageSource<'static>,
}

impl<'a> WindowState<'a> {
    fn input(&mut self, window: Arc<Window>, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::Focused(focused) => {
                self.raw_input
                    .events
                    .push(egui::Event::WindowFocused(*focused));
                const CURSOR_BLINK_INTERVAL: f32 = 0.5;

                if self.start_typing
                    && self.cursor_timer.elapsed().as_secs_f32() >= CURSOR_BLINK_INTERVAL
                {
                    self.cursor_visible = !self.cursor_visible;
                    self.cursor_timer = Instant::now();
                    self.window.request_redraw();
                }
                true
            }
            WindowEvent::ModifiersChanged(modifiers_state) => {
                self.raw_input.modifiers = egui::Modifiers {
                    alt: modifiers_state.alt_key(),
                    ctrl: modifiers_state.control_key(),
                    shift: modifiers_state.shift_key(),
                    mac_cmd: cfg!(target_os = "macos") && modifiers_state.super_key(),
                    command: if cfg!(target_os = "macos") {
                        modifiers_state.super_key()
                    } else {
                        modifiers_state.control_key()
                    },
                };
                true
            }
            WindowEvent::CursorMoved {
                device_id: _,
                position,
                ..
            } => {
                self.last_cursor_position = *position;

                if let tao::event::WindowEvent::CursorMoved { position, .. } = event {
                    self.raw_input
                        .events
                        .push(egui::Event::PointerMoved(egui::pos2(
                            position.x as f32,
                            position.y as f32,
                        )));
                }

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
                ..
            } => {
                let pressed = *state == tao::event::ElementState::Pressed;

                let button_egui = match button {
                    MouseButton::Left => egui::PointerButton::Primary,
                    MouseButton::Right => egui::PointerButton::Secondary,
                    MouseButton::Middle => egui::PointerButton::Middle,
                    _ => return false,
                };

                self.raw_input.events.push(egui::Event::PointerButton {
                    pos: egui::pos2(
                        self.last_cursor_position.x as f32,
                        self.last_cursor_position.y as f32,
                    ),
                    button: button_egui,
                    pressed,
                    modifiers: self.raw_input.modifiers,
                });

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
                        self.texts.push(TextEntries::null(
                            normalized_to_rgba(self.current_color),
                            self.font_size,
                        ));
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

                        if self.pressed_keys.contains(&Key::Character("s")) {
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

                        window.request_redraw();
                    }
                }
                true
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(key) = egui_key(event.logical_key.clone()) {
                    self.raw_input.events.push(EventEgui::Key {
                        key,
                        physical_key: KeyEgui::from_name(&event.physical_key.to_string()),
                        pressed: true,
                        repeat: false,
                        modifiers: self.raw_input.modifiers,
                    });
                }
                match event.state {
                    ElementState::Pressed => {
                        self.pressed_keys.insert(event.logical_key.clone());

                        if self.start_typing || self.editing_text_index.is_some() {
                            if let Key::Character(char) = &event.logical_key {
                                if let Some(text) = self.texts.last_mut() {
                                    if text.pending {
                                        text.text.push_str(char);
                                        window.request_redraw();
                                    }
                                }
                            }
                            match event.logical_key {
                                Key::Enter => {
                                    self.start_typing = false;
                                    self.editing_text_index = None;
                                    if let Some(text) = self.texts.last_mut() {
                                        text.pending = false;
                                        self.actions.push(Action::Text(text.clone()));
                                    }
                                    window.request_redraw();
                                }
                                Key::Delete => {
                                    let text_entry = if let Some(index) = self.editing_text_index {
                                        self.texts.get_mut(index)
                                    } else {
                                        self.texts.last_mut()
                                    };
                                    if let Some(entry) = text_entry {
                                        entry.text.pop();
                                        window.request_redraw();
                                    }
                                }
                                Key::GoBack => {
                                    self.start_typing = false;
                                    self.editing_text_index = None;
                                    if let Some(text) = self.texts.last_mut() {
                                        text.pending = false;
                                        self.actions.push(Action::Text(text.clone()));
                                    }
                                    window.request_redraw();
                                }
                                Key::Backspace => {
                                    if self.editing_text_index.is_some() {
                                        let editing_text = self.texts
                                            [self.editing_text_index.unwrap()]
                                        .borrow_mut();
                                        if editing_text.pending
                                            && editing_text.text.chars().count() > 0
                                        {
                                            editing_text.text = editing_text
                                                .text
                                                .chars()
                                                .take(editing_text.text.chars().count() - 1)
                                                .collect();
                                            window.request_redraw();
                                        }
                                    } else if let Some(text) = self.texts.last_mut() {
                                        if text.pending && text.text.chars().count() > 0 {
                                            text.text = text
                                                .text
                                                .chars()
                                                .take(text.text.chars().count() - 1)
                                                .collect();
                                            window.request_redraw();
                                        }
                                    }
                                }
                                _ => {}
                            }
                        } else if self.pressed_keys.contains(&Key::Control)
                            && self.pressed_keys.contains(&Key::Character("z"))
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
                    _ => (),
                }
                true
            }
            WindowEvent::Resized(physical_size) => {
                self.size = *physical_size;
                self.resize(*physical_size);
                self.raw_input.screen_rect = Some(egui::Rect {
                    min: egui::pos2(0.0, 0.0),
                    max: egui::pos2(physical_size.width as f32, physical_size.height as f32),
                });
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
        let egui_ctx = egui::Context::default();
        let egui_renderer = Renderer::new(&device, surface_config.format, None, 1, true);
        let raw_input = RawInput::default();
        egui_extras::install_image_loaders(&egui_ctx);
        surface.configure(&device, &surface_config);

        let mut font_system = FontSystem::new();
        font_system
            .db_mut()
            .load_font_data(include_bytes!("assets/vazir.ttf").to_vec());
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, swapchain_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);

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
            scale_factor,
            surface,
            actions: Vec::new(),
            pressed_keys: HashSet::new(),
            surface_config,
            font_system,
            font_size: 16,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
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
            rectangle_shader: Some(rectangle_shader),
            shape_positions: Vec::new(),
            egui_renderer,
            show_modal_fonts: false,
            show_modal_colors: false,

            color: include_image!("assets/color.png"),
            font: include_image!("assets/font.png"),
            rect: include_image!("assets/rect.png"),
            prev: include_image!("assets/prev.png"),
            raw_input,
            egui_context: egui_ctx,
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

            let _ = self.render();
        }
    }

    fn update(&mut self) -> Result<(), egui_wgpu::wgpu::SurfaceError> {
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut all_vertices = Vec::new();

        let physical_width = (self.size.width as f64 * self.scale_factor) as f32;
        let physical_height = (self.size.height as f64 * self.scale_factor) as f32;

        for action in &self.actions {
            if let Action::Stroke(stroke) = action {
                if stroke.len() >= 2 {
                    for i in 0..(stroke.len() - 1) {
                        all_vertices.push(stroke[i]);
                        all_vertices.push(stroke[i + 1]);
                    }
                }
            }
        }

        if self.current_stroke.len() >= 2 {
            for i in 0..(self.current_stroke.len() - 1) {
                all_vertices.push(self.current_stroke[i]);
                all_vertices.push(self.current_stroke[i + 1]);
            }
        }

        let vertex_data = bytemuck::cast_slice(&all_vertices);
        self.vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Vertex Buffer"),
                contents: vertex_data,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
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

        let mut buffers = Vec::new();
        for text_entry in &self.texts {
            let mut text_buffer = Buffer::new(
                &mut self.font_system,
                Metrics::new(
                    text_entry.font_size as f32,
                    text_entry.font_size as f32 * 0.1,
                ),
            );

            text_buffer.set_size(
                &mut self.font_system,
                Some(physical_width),
                Some(physical_height),
            );
            text_buffer.shape_until_scroll(&mut self.font_system, false);

            let mut text = text_entry.text.clone();
            if text_entry.pending && self.cursor_visible {
                text.push('|');
            }

            let text = format!("\u{200E}\u{200C}{}", text);
            text_buffer.set_text(
                &mut self.font_system,
                &text,
                Attrs::new().family(Family::Name("Vazir")),
                Shaping::Advanced,
            );
            text_buffer.shape_until_scroll(&mut self.font_system, false);
            buffers.push(text_buffer);
        }

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

        Ok(())
    }

    fn render(&mut self) -> Result<(), egui_wgpu::wgpu::SurfaceError> {
        self.egui_context.begin_pass(self.raw_input.clone());
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

                let rectangle_vertex_buffer =
                    self.device
                        .create_buffer_init(&egui_wgpu::wgpu::util::BufferInitDescriptor {
                            label: Some("Rectangle Vertex Buffer"),
                            contents: bytemuck::cast_slice(&flattened_shapes),
                            usage: egui_wgpu::wgpu::BufferUsages::VERTEX,
                        });

                render_pass.set_pipeline(rectangle_shader);
                render_pass.set_vertex_buffer(0, rectangle_vertex_buffer.slice(..));
                render_pass.draw(0..flattened_shapes.len() as u32, 0..1);
            }

            if self.vertex_buffer.size() > 0 {
                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(
                    0..(self.vertex_buffer.size() as u32 / std::mem::size_of::<Vertex>() as u32),
                    0..1,
                );
            }
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [self.surface_config.width, self.surface_config.height],
            pixels_per_point: self.egui_context.pixels_per_point(),
        };
        let header_height = self.surface_config.height as f32;
        let header_width = (self.surface_config.width as f64 * self.scale_factor) as f32;

        let menu_color = egui::Color32::from_hex("#5C5C5C").expect("unable to get color");

        let sized = vec![10, 12, 14, 16, 18, 20, 24, 28, 32];

        if self.show_modal_colors {
            egui::Window::new("رنگ قلم")
                .collapsible(false)
                .order(egui::Order::Foreground)
                .movable(false)
                .resizable(false)
                // .fixed_pos(egui::Pos2 { x: 0.0, y: 10.0 })
                .anchor(Align2::CENTER_TOP, [0.0, 0.0])
                .show(&self.egui_context, |ui| {
                    ui.vertical(|ui| {
                        let colors = [
                            egui::Color32::from_rgb(255, 0, 0),     // Red
                            egui::Color32::from_rgb(0, 255, 0),     // Green
                            egui::Color32::from_rgb(0, 0, 255),     // Blue
                            egui::Color32::from_rgb(255, 255, 0),   // Yellow
                            egui::Color32::from_rgb(255, 0, 255),   // Magenta
                            egui::Color32::from_rgb(0, 255, 255),   // Cyan
                            egui::Color32::from_rgb(0, 0, 0),       // Black
                            egui::Color32::from_rgb(255, 255, 255), // White
                        ];

                        ui.horizontal_wrapped(|ui| {
                            for &color in &colors {
                                let size = egui::Vec2::splat(30.0);
                                if ui
                                    .add(egui::Button::new("").fill(color).min_size(size))
                                    .clicked()
                                {
                                    self.current_color = convert_to_buffer(color);
                                    self.show_modal_colors = false;
                                    self.egui_context.request_repaint();
                                }
                            }
                        });
                    });
                });
        }

        if self.show_modal_fonts {
            egui::Window::new("فونت")
                .collapsible(false)
                .order(egui::Order::Foreground)
                .resizable(false)
                // .fixed_pos(Pos2 { x: 0.0, y: 10.0 })
                .movable(false)
                .min_height(header_height * 2.0)
                .anchor(Align2::CENTER_TOP, [0.0, 0.0])
                .show(&self.egui_context, |ui| {
                    ui.horizontal(|ui| {
                        for size in sized {
                            if ui.button(format!("{} px", size)).clicked() {
                                self.font_size = size;
                                self.show_modal_fonts = false;
                                self.window.request_redraw();
                            }
                        }
                    });
                });
        }

        egui::Area::new("Header".into())
            .fixed_pos([0.0, 0.0])
            .movable(false)
            .order(egui::Order::Background)
            .default_size([header_width, header_height * 10.0])
            .show(&self.egui_context, |ui| {
                let custom_frame = egui::Frame::none()
                    .fill(menu_color)
                    .stroke(egui::Stroke::new(1.0, menu_color));
                custom_frame.show(ui, |ui| {
                    ui.set_min_width(header_width);
                    ui.vertical(|ui| {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.set_width(header_width);

                            ui.add_space(header_width * 0.4);
                            let prev = ImageButton::new(Image::new(self.prev.clone())).frame(false);
                            let prev_button = ui.add(prev);
                            if prev_button.clicked() {
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
                                self.window.request_redraw();
                            }
                            ui.add_space(header_width * 0.03);

                            let sqaure =
                                ImageButton::new(Image::new(self.rect.clone())).frame(false);
                            let sqaure_button = ui.add(sqaure);
                            if sqaure_button.clicked() {
                                self.create_rect = true;
                            }
                            ui.add_space(header_width * 0.03);

                            let font = ImageButton::new(Image::new(self.font.clone())).frame(false);
                            let font_button = ui.add(font);
                            if font_button.clicked() {
                                self.show_modal_fonts = true;
                                self.egui_context.request_repaint();
                            }

                            ui.add_space(header_width * 0.03);

                            let color_picker =
                                ImageButton::new(Image::new(self.color.clone())).frame(false);
                            let color_picker_button = ui.add(color_picker);
                            if color_picker_button.clicked() {
                                self.show_modal_colors = true;
                                self.egui_context.request_repaint();
                            }
                        });

                        ui.add_space(10.0);
                    });
                });
            });

        let full_output = self.egui_context.end_pass();

        let tris = self
            .egui_context
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, image_delta);
        }

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &tris,
            &screen_descriptor,
        );

        let rpass = encoder.begin_render_pass(&egui_wgpu::wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: egui_wgpu::wgpu::Operations {
                    load: egui_wgpu::wgpu::LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            label: Some("egui main render pass"),
            occlusion_query_set: None,
        });

        self.egui_renderer
            .render(&mut rpass.forget_lifetime(), &tris, &screen_descriptor);
        for x in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(x);
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

struct Application<'a> {
    window_state: Option<WindowState<'a>>,
}

const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f64 = 5.0;

impl<'a> Application<'a> {
    fn about_to_wait(&mut self) {
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

    fn window_event(&mut self, _window_id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.window_state else {
            return;
        };

        let window = &state.window;
        state.input(window.clone(), &event);
    }
}

fn convert_to_buffer(color: Color32) -> [f32; 4] {
    [
        color.r().into(),
        color.g().into(),
        color.b().into(),
        color.a().into(),
    ]
}

fn normalized_to_rgba(normalized: [f32; 4]) -> [u8; 4] {
    let red = (normalized[0] * 255.0) as u8;
    let green = (normalized[1] * 255.0) as u8;
    let blue = (normalized[2] * 255.0) as u8;
    let alpha = (normalized[3] * 255.0) as u8;
    [red, green, blue, alpha]
}

fn egui_key(key: Key) -> Option<KeyEgui> {
    match key {
        Key::Character(char) => KeyEgui::from_name(char),
        Key::Enter => Some(KeyEgui::Enter),
        Key::Space => Some(KeyEgui::Space),
        Key::Backspace => Some(KeyEgui::Backspace),
        Key::Tab => Some(KeyEgui::Tab),
        _ => None,
    }
}

fn is_persian(char: char) -> bool {
    ('\u{0600}'..='\u{06FF}').contains(&char) || ('\u{0750}'..='\u{077F}').contains(&char)
}
