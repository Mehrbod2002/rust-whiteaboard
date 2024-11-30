mod ui;
use egui::{
    color_picker, include_image, Color32, ColorImage, Context, Id, Image, ImageButton, ImageSource,
    Response, TextureHandle, TextureId, Ui, Vec2,
};
use egui_wgpu::{
    wgpu::{
        self, util::DeviceExt, vertex_attr_array, CommandEncoderDescriptor, CompositeAlphaMode,
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
use resvg::{tiny_skia::Pixmap, usvg};
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
    event_loop::{self, ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey, SmolStr},
    window::{Window, WindowId},
};

pub struct AppState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface: wgpu::Surface<'static>,
    pub scale_factor: f32,
    pub egui_renderer: EguiRenderer,

    pressed_keys: HashSet<Key>,
    egui_rendererd: bool,
    last_cursor_position: PhysicalPosition<f64>,
    actions: Vec<Action>,
    modifiers: ModifiersState,
    size: PhysicalSize<u32>,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: glyphon::Viewport,
    texts: Vec<TextEntries>,
    atlas: glyphon::TextAtlas,
    text_renderer: glyphon::TextRenderer,
    text_buffer: glyphon::Buffer,

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

    font_button: Option<Response>,
    color_picker_button: Option<Response>,
    sqaure_button: Option<Response>,
    prev_button: Option<Response>,
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

impl AppState {
    async fn new(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        window: &Window,
        width: u32,
        height: u32,
    ) -> Self {
        let physical_size = window.inner_size();
        let scale_factor = window.scale_factor();
        let power_pref = wgpu::PowerPreference::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power_pref,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect("Failed to find an appropriate adapter");

        let features = wgpu::Features::empty();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: features,
                    required_limits: Default::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .expect("Failed to create device");

        let swapchain_capabilities = surface.get_capabilities(&adapter);
        let selected_format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let swapchain_format = swapchain_capabilities
            .formats
            .iter()
            .find(|d| **d == selected_format)
            .expect("failed to select proper surface texture format!");

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *swapchain_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 0,
            alpha_mode: swapchain_capabilities.alpha_modes[0],
            view_formats: vec![],
        };

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, *swapchain_format);
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

        surface.configure(&device, &surface_config);

        let egui_renderer = EguiRenderer::new(&device, surface_config.format, None, 1, &window);

        let scale_factor = 1.0;

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

        let shader = device.create_shader_module(egui_wgpu::wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: egui_wgpu::wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
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

        Self {
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
            prev_button: None,
            font_button: None,
            color_picker_button: None,
            sqaure_button: None,
        }
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }
}

pub struct App {
    instance: wgpu::Instance,
    state: Option<AppState>,
    window: Option<Arc<Window>>,
}

impl App {
    pub fn new() -> Self {
        let instance = egui_wgpu::wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        Self {
            instance,
            state: None,
            window: None,
        }
    }

    async fn set_window(&mut self, window: Window) {
        let window = Arc::new(window);
        let initial_width = 1360;
        let initial_height = 768;

        let _ = window.request_inner_size(PhysicalSize::new(initial_width, initial_height));

        let surface = self
            .instance
            .create_surface(window.clone())
            .expect("Failed to create surface!");

        let state = AppState::new(
            &self.instance,
            surface,
            &window,
            initial_width,
            initial_width,
        )
        .await;

        self.window.get_or_insert(window);
        self.state.get_or_insert(state);
    }

    fn handle_resized(&mut self, width: u32, height: u32) {
        self.state.as_mut().unwrap().resize_surface(width, height);
    }

    fn handle_redraw(&mut self) {
        let state = self.state.as_mut().unwrap();

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [state.surface_config.width, state.surface_config.height],
            pixels_per_point: self.window.as_ref().unwrap().scale_factor() as f32
                * state.scale_factor,
        };

        let surface_texture = state
            .surface
            .get_current_texture()
            .expect("Failed to acquire next swap chain texture");

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        let window = self.window.as_ref().unwrap();

        {
            state.egui_renderer.begin_frame(window);

            egui::Window::new("winit + egui + wgpu says hello!")
                .resizable(true)
                .vscroll(true)
                .default_open(false)
                .show(state.egui_renderer.context(), |ui| {
                    ui.label("Label!");

                    if ui.button("Button!").clicked() {
                        println!("boom!")
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Pixels per point: {}",
                            state.egui_renderer.context().pixels_per_point()
                        ));
                        if ui.button("-").clicked() {
                            state.scale_factor = (state.scale_factor - 0.1).max(0.3);
                        }
                        if ui.button("+").clicked() {
                            state.scale_factor = (state.scale_factor + 0.1).min(3.0);
                        }
                    });
                });

            state.egui_renderer.end_frame_and_draw(
                &state.device,
                &state.queue,
                &mut encoder,
                window,
                &surface_view,
                screen_descriptor,
            );
        }

        state.queue.submit(Some(encoder.finish()));
        surface_texture.present();
    }

    fn update(&mut self) -> bool {
        let buffers: Vec<glyphon::Buffer> = Vec::new();
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut all_vertices = Vec::new();
        let Some(state) = &mut self.state else {
            return false;
        };

        state.actions.iter().for_each(|x| {
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

        if state.start_typing {
            let elapsed = state.cursor_timer.elapsed().as_secs_f32();
            if elapsed >= CURSOR_BLINK_INTERVAL {
                state.cursor_visible = !state.cursor_visible;
                state.cursor_timer = Instant::now();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }

        for (index, (text_entry, buffer)) in state.texts.iter_mut().zip(buffers.iter()).enumerate()
        {
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
                right: state.size.width as i32,
                bottom: state.size.height as i32,
            };

            let normalized_color = normalized_to_rgba(state.current_color);
            let default_color = if Some(index) == state.editing_text_index {
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

        if state.current_stroke.len() >= 2 {
            for i in 0..(state.current_stroke.len() - 1) {
                all_vertices.push(state.current_stroke[i]);
                all_vertices.push(state.current_stroke[i + 1]);
            }
        }

        if !all_vertices.is_empty() {
            let vertex_data = bytemuck::cast_slice(&all_vertices);
            state.vertex_buffer =
                state
                    .device
                    .create_buffer_init(&egui_wgpu::wgpu::util::BufferInitDescriptor {
                        label: Some("Vertex Buffer"),
                        contents: vertex_data,
                        usage: egui_wgpu::wgpu::BufferUsages::VERTEX
                            | egui_wgpu::wgpu::BufferUsages::COPY_DST,
                    });
        }

        let mut buffers: Vec<glyphon::Buffer> = Vec::new();

        for text_entry in state.texts.iter() {
            let mut buffer = state.text_buffer.clone();
            let mut text = text_entry.text.clone();
            if text_entry.pending && state.cursor_visible {
                text.push('|');
            }

            buffer.set_text(
                &mut state.font_system,
                &text,
                Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
            );

            buffers.push(buffer);
        }

        let mut text_areas: Vec<TextArea> = Vec::new();

        for (text_entry, buffer) in state.texts.iter().zip(buffers.iter()) {
            let x = text_entry.position[0];
            let y = text_entry.position[1];

            let text_bounds = TextBounds {
                left: 0,
                top: 0,
                right: state.size.width as i32,
                bottom: state.size.height as i32,
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

        let _ = state.text_renderer.prepare(
            &state.device,
            &state.queue,
            &mut state.font_system,
            &mut state.atlas,
            &state.viewport,
            text_areas,
            &mut state.swash_cache,
        );

        if let Some(window) = &self.window {
            state.egui_renderer.begin_frame(&window);
        }

        true
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(Window::default_attributes())
            .unwrap();
        pollster::block_on(self.set_window(window));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        {
            let state = self.state.as_mut().unwrap();

            state
                .egui_renderer
                .handle_input(self.window.as_ref().unwrap(), &event);

            match event {
                WindowEvent::CloseRequested => {
                    println!("The close button was pressed; stopping");
                    event_loop.exit();
                }
                WindowEvent::RedrawRequested => {
                    state.viewport.update(
                        &state.queue,
                        Resolution {
                            width: state.surface_config.width,
                            height: (state.surface_config.height as f32 * 0.8) as u32,
                        },
                    );
                    // let _ = self.update();

                    self.handle_redraw();

                    self.window.as_ref().unwrap().request_redraw();
                }
                WindowEvent::Resized(new_size) => {
                    self.handle_resized(new_size.width, new_size.height);
                }
                _ => (),
            }
        }
    }
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        pollster::block_on(run());
    }
}

async fn run() {
    let event_loop = EventLoop::new().unwrap();

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();

    event_loop.run_app(&mut app).expect("Failed to run app");
}

fn normalized_to_rgba(normalized: [f32; 4]) -> [u8; 4] {
    let red = (normalized[0] * 255.0) as u8;
    let green = (normalized[1] * 255.0) as u8;
    let blue = (normalized[2] * 255.0) as u8;
    let alpha = (normalized[3] * 255.0) as u8;
    [red, green, blue, alpha]
}
