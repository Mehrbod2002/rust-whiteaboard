use egui::ViewportId;
use egui_wgpu::{
    wgpu::{CommandEncoder, Device, Queue, RenderPass, StoreOp, TextureFormat, TextureView},
    Renderer, ScreenDescriptor,
};
use egui_winit::State;
use winit::{event::WindowEvent, window::Window};

pub struct EguiRenderer {
    pub state: State,
    pub renderer: Renderer,
    pub frame_started: bool,
}

impl EguiRenderer {
    pub fn new(
        window: &Window,
        device: &Device,
        output_format: TextureFormat,
        depth: Option<TextureFormat>,
        masa_sample: u32,
    ) -> Self {
        let egui_context = egui::Context::default();

        let fonts = egui::FontDefinitions::default();
        egui_context.set_fonts(fonts);
        let state = egui_winit::State::new(
            egui_context,
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(2 * 1024),
        );
        let renderer = Renderer::new(device, output_format, depth, masa_sample, true);
        EguiRenderer {
            state,
            renderer,
            frame_started: false,
        }
    }

    pub fn context(&self) -> &egui::Context {
        self.state.egui_ctx()
    }

    pub fn begin_pass(&mut self, window: &Window) {
        let raw_input = self.state.take_egui_input(window);
        self.state.egui_ctx().begin_pass(raw_input);
        self.frame_started = true;
    }

    pub fn handle_input(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.state.on_window_event(window, event);
    }

    pub fn set_pixels_per_point(&mut self, v: f32) {
        self.context().set_pixels_per_point(v);
    }

    pub fn end_frame_and_draw(
        &mut self,
        device: &Device,
        queue: &Queue,
        render_pass: RenderPass,
        encoder: &mut CommandEncoder,
        window: &Window,
        _window_surface_view: &TextureView,
        screen_descriptor: ScreenDescriptor,
    ) {
        if !self.frame_started {
            panic!("begin_frame must be called before end_frame_and_draw can be called!");
        }

        self.context()
            .set_pixels_per_point(screen_descriptor.pixels_per_point);

        let full_output = self.state.egui_ctx().end_pass();

        self.state
            .handle_platform_output(window, full_output.platform_output);

        let tris = self
            .state
            .egui_ctx()
            .tessellate(full_output.shapes, self.state.egui_ctx().pixels_per_point());
        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(device, queue, *id, image_delta);
        }
        self.renderer
            .update_buffers(device, queue, encoder, &tris, &screen_descriptor);

        self.renderer.render(
            &mut render_pass.forget_lifetime(),
            &tris,
            &screen_descriptor,
        );
        for x in &full_output.textures_delta.free {
            self.renderer.free_texture(x)
        }

        self.frame_started = false;
    }
}
