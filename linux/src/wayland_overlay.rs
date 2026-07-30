use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor, wgpu};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::{
    error::Error,
    os::fd::AsRawFd,
    ptr::NonNull,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface::WlSurface},
};

const FRAME_TIME: Duration = Duration::from_nanos(1_000_000_000 / 120);

pub fn run<D, F>(init: F) -> Result<bool, Box<dyn Error>>
where
    D: FnMut(&egui::Context, [f32; 2]) -> bool,
    F: FnOnce(&egui::Context, [f32; 2]) -> D,
{
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let wl_surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        wl_surface,
        Layer::Overlay,
        Some("forzalife-overlay"),
        None,
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(0, 0);

    let input_region = Region::new(&compositor)?;
    layer
        .wl_surface()
        .set_input_region(Some(input_region.wl_region()));
    layer.commit();

    let mut state = State {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        _compositor: compositor,
        _layer_shell: layer_shell,
        configured: false,
        closed: false,
        width: 0,
        height: 0,
    };
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }

    let mut gpu = Gpu::new(
        &connection,
        layer.wl_surface(),
        state.width.max(1),
        state.height.max(1),
    )?;
    let context = egui::Context::default();
    let mut draw = init(&context, state.size());
    let mut renderer = Renderer::new(&gpu.device, gpu.config.format, RendererOptions::default());
    let started = Instant::now();
    let mut next_frame = Instant::now();

    let mut reload_requested = false;
    while !state.closed {
        pump_wayland(&connection, &mut event_queue, &mut state)?;
        if state.width != gpu.config.width || state.height != gpu.config.height {
            gpu.resize(state.width, state.height);
        }

        let size = state.size();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            time: Some(started.elapsed().as_secs_f64()),
            predicted_dt: FRAME_TIME.as_secs_f32(),
            ..Default::default()
        };
        let mut reload = false;
        let output = context.run(input, |context| reload = draw(context, size));
        let paint_jobs = context.tessellate(output.shapes, output.pixels_per_point);
        let screen = ScreenDescriptor {
            size_in_pixels: [gpu.config.width, gpu.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        for (id, delta) in &output.textures_delta.set {
            renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }

        let frame = match gpu.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                gpu.resize(state.width, state.height);
                continue;
            }
            Err(wgpu::SurfaceError::Timeout) => continue,
            Err(error) => return Err(error.into()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("forzalife-overlay"),
            });
        let mut command_buffers =
            renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &paint_jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forzalife-overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            renderer.render(&mut pass.forget_lifetime(), &paint_jobs, &screen);
        }
        command_buffers.push(encoder.finish());
        gpu.queue.submit(command_buffers);
        frame.present();
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        if reload {
            reload_requested = true;
            break;
        }

        next_frame += FRAME_TIME;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else {
            next_frame = now;
        }
    }
    Ok(reload_requested)
}

fn pump_wayland(
    connection: &Connection,
    event_queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
) -> Result<(), Box<dyn Error>> {
    event_queue.dispatch_pending(state)?;
    connection.flush()?;
    if let Some(guard) = connection.prepare_read() {
        let mut descriptor = libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if ready > 0 && descriptor.revents & libc::POLLIN != 0 {
            guard.read()?;
            event_queue.dispatch_pending(state)?;
        }
    }
    if let Some(error) = connection.backend().last_error() {
        return Err(format!("Wayland connection lost: {error}").into());
    }
    Ok(())
}

struct State {
    registry: RegistryState,
    output: OutputState,
    _compositor: CompositorState,
    _layer_shell: LayerShell,
    configured: bool,
    closed: bool,
    width: u32,
    height: u32,
}

impl State {
    fn size(&self) -> [f32; 2] {
        [self.width.max(1) as f32, self.height.max(1) as f32]
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.width = configure.new_size.0.max(1);
        self.height = configure.new_size.1.max(1);
        self.configured = true;
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers!(OutputState);
}

delegate_compositor!(State);
delegate_layer!(State);
delegate_output!(State);
delegate_registry!(State);

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    fn new(
        connection: &Connection,
        surface: &WlSurface,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let display = NonNull::new(connection.backend().display_ptr().cast())
            .ok_or("Wayland returned a null display")?;
        let window =
            NonNull::new(surface.id().as_ptr().cast()).ok_or("Wayland returned a null surface")?;
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display)),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
        };
        let surface: wgpu::Surface<'static> = unsafe { instance.create_surface_unsafe(target)? };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("forzalife-overlay"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            }))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let alpha_mode = capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::CompositeAlphaMode::PreMultiplied)
            .unwrap_or(capabilities.alpha_modes[0]);
        let present_mode = if capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
        {
            wgpu::PresentMode::Immediate
        } else if capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        Ok(Self {
            surface,
            device,
            queue,
            config,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }
}
