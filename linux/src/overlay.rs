use crate::{
    locations::{LocationKind, Locations},
    model::{LifeSnapshot, Simulation},
    telemetry::{PACKET_SIZE, Telemetry, parse},
};
use eframe::egui;
use std::{
    env,
    net::UdpSocket,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{Atom, AtomEnum, ConnectionExt, GrabMode, ModMask, PropMode, Window},
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

const WINDOW_TITLE: &str = "ForzaLife Linux Overlay";

#[derive(Default)]
struct Latest {
    telemetry: Option<Telemetry>,
    life: Option<LifeSnapshot>,
    received_at: Option<Instant>,
    packets: u64,
    rejected: u64,
}

pub fn run(port: u16) -> eframe::Result {
    let latest = Arc::new(RwLock::new(Latest::default()));
    let state_path = state_path();
    let locations_path = locations_path();
    let locations = Arc::new(RwLock::new(Locations::load(&locations_path)));
    spawn_receiver(
        port,
        Arc::clone(&latest),
        Arc::clone(&locations),
        state_path,
    );
    let menu_open = spawn_menu_toggle();
    let [width, height] = screen_size().unwrap_or([1920.0, 1080.0]);
    spawn_gamescope_classifier();

    let viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([width, height])
        .with_position([0.0, 0.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_taskbar(false);

    eframe::run_native(
        WINDOW_TITLE,
        eframe::NativeOptions {
            viewport,
            vsync: true,
            ..Default::default()
        },
        Box::new(|context| {
            context.egui_ctx.set_visuals(egui::Visuals {
                panel_fill: egui::Color32::TRANSPARENT,
                window_fill: egui::Color32::TRANSPARENT,
                ..egui::Visuals::dark()
            });
            Ok(Box::new(OverlayApp {
                latest,
                locations,
                locations_path,
                menu_open,
                applied_menu_open: false,
                port,
            }))
        }),
    )
}

struct OverlayApp {
    latest: Arc<RwLock<Latest>>,
    locations: Arc<RwLock<Locations>>,
    locations_path: PathBuf,
    menu_open: Arc<AtomicBool>,
    applied_menu_open: bool,
    port: u16,
}

impl eframe::App for OverlayApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        context.request_repaint_after(Duration::from_millis(50));
        let menu_open = self.menu_open.load(Ordering::Relaxed);
        if menu_open != self.applied_menu_open {
            context.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(!menu_open));
            context.send_viewport_cmd(egui::ViewportCommand::CursorVisible(menu_open));
            self.applied_menu_open = menu_open;
        }
        let latest = self.latest.read().unwrap();
        let fresh = latest
            .received_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2));

        egui::Area::new(egui::Id::new("forzalife-hud"))
            .fixed_pos([24.0, 24.0])
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(185))
                    .corner_radius(12)
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.set_width(490.0);
                        ui.horizontal(|ui| {
                            let color = if fresh {
                                egui::Color32::from_rgb(140, 210, 30)
                            } else {
                                egui::Color32::from_rgb(255, 170, 40)
                            };
                            ui.colored_label(color, "▮");
                            ui.strong(if fresh {
                                "FORZALIFE"
                            } else {
                                "WAITING FOR HORIZON 6"
                            });
                        });

                        if let Some(data) = latest.telemetry.as_ref() {
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                metric(ui, "SPEED", &format!("{:.0} km/h", data.speed_mps * 3.6));
                                metric(
                                    ui,
                                    "RPM",
                                    &format!(
                                        "{:.0} / {:.0}",
                                        data.current_engine_rpm, data.engine_max_rpm
                                    ),
                                );
                                metric(ui, "BOOST", &format!("{:.1} psi", data.boost_psi));
                                if let Some(life) = latest.life.as_ref() {
                                    metric(
                                        ui,
                                        "FUEL",
                                        &format!("{:.0}%", life.fuel_percent * 100.0),
                                    );
                                    metric(
                                        ui,
                                        "ODO",
                                        &format!("{:.1} km", life.odometer_m / 1_000.0),
                                    );
                                    metric(
                                        ui,
                                        "OIL",
                                        &format!("{:.0} km", life.oil_remaining_m / 1_000.0),
                                    );
                                }
                                metric(ui, "GEAR", &data.gear.to_string());
                            });
                            let locations = self.locations.read().unwrap();
                            navigation(ui, data, &locations);
                        } else {
                            ui.add_space(8.0);
                            ui.label(format!(
                                "UDP 127.0.0.1:{} • enable Data Out in-game",
                                self.port
                            ));
                        }
                        ui.add_space(4.0);
                        ui.small(format!(
                            "{} packets • {} rejected • L opens service menu",
                            latest.packets, latest.rejected
                        ));
                    });
            });

        if menu_open {
            let telemetry = latest.telemetry.clone();
            drop(latest);
            egui::Window::new("ForzaLife service locations")
                .default_pos([24.0, 180.0])
                .show(context, |ui| {
                    ui.label("Park at a real location, then add it once.");
                    if let Some(data) = telemetry.as_ref() {
                        if ui.button("Add gas station here").clicked() {
                            self.add_location(LocationKind::Gas, data.position);
                        }
                        if ui.button("Add workshop here").clicked() {
                            self.add_location(LocationKind::Workshop, data.position);
                        }
                    } else {
                        ui.label("Waiting for telemetry.");
                    }
                    ui.separator();
                    let locations = self.locations.read().unwrap();
                    ui.label(format!("{} saved locations", locations.locations.len()));
                    for location in &locations.locations {
                        ui.label(format!("{:?}: {}", location.kind, location.name));
                    }
                });
        }
    }
}

impl OverlayApp {
    fn add_location(&self, kind: LocationKind, position: [f32; 3]) {
        let mut locations = self.locations.write().unwrap();
        locations.add(kind, position);
        if let Err(error) = locations.save(&self.locations_path) {
            eprintln!("could not save service locations: {error}");
        }
    }
}

fn metric(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.vertical(|ui| {
        ui.small(name);
        ui.strong(value);
    });
    ui.separator();
}

fn spawn_receiver(
    port: u16,
    latest: Arc<RwLock<Latest>>,
    locations: Arc<RwLock<Locations>>,
    state_path: PathBuf,
) {
    thread::spawn(move || {
        let socket = match UdpSocket::bind(("127.0.0.1", port)) {
            Ok(socket) => socket,
            Err(error) => {
                eprintln!("cannot listen on UDP 127.0.0.1:{port}: {error}");
                return;
            }
        };
        let mut simulation = Simulation::load(&state_path);
        let mut last_save = Instant::now();
        let mut packet = [0_u8; PACKET_SIZE + 1];
        loop {
            match socket.recv(&mut packet) {
                Ok(size) => {
                    let mut state = latest.write().unwrap();
                    match parse(&packet[..size]) {
                        Ok(telemetry) => {
                            let mut life = simulation.update(&telemetry);
                            if telemetry.speed_mps.abs() < 1.0 {
                                let locations = locations.read().unwrap();
                                if locations
                                    .nearest(LocationKind::Gas, telemetry.position)
                                    .is_some_and(|(_, distance)| distance <= 25.0)
                                {
                                    simulation.refuel(telemetry.car_ordinal);
                                    life = simulation.update(&telemetry);
                                }
                                if locations
                                    .nearest(LocationKind::Workshop, telemetry.position)
                                    .is_some_and(|(_, distance)| distance <= 25.0)
                                {
                                    simulation.service_oil(telemetry.car_ordinal);
                                    life = simulation.update(&telemetry);
                                }
                            }
                            state.telemetry = Some(telemetry);
                            state.life = Some(life);
                            state.received_at = Some(Instant::now());
                            state.packets += 1;
                            if last_save.elapsed() >= Duration::from_secs(5) {
                                if let Err(error) = simulation.save(&state_path) {
                                    eprintln!("could not save vehicle state: {error}");
                                }
                                last_save = Instant::now();
                            }
                        }
                        Err(_) => state.rejected += 1,
                    }
                }
                Err(error) => {
                    eprintln!("UDP receive failed: {error}");
                    return;
                }
            }
        }
    });
}

fn navigation(ui: &mut egui::Ui, telemetry: &Telemetry, locations: &Locations) {
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        for kind in [LocationKind::Gas, LocationKind::Workshop] {
            if let Some((location, distance)) = locations.nearest(kind, telemetry.position) {
                let color = match kind {
                    LocationKind::Gas => egui::Color32::from_rgb(140, 210, 30),
                    LocationKind::Workshop => egui::Color32::from_rgb(80, 170, 255),
                };
                ui.colored_label(color, format!("{} • {:.0} m", location.name, distance));
            }
        }
    });
    let (response, painter) = ui.allocate_painter(egui::vec2(490.0, 92.0), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, egui::Color32::from_black_alpha(100));
    let center = rect.center();
    painter.circle_filled(center, 4.0, egui::Color32::WHITE);
    let sin = telemetry.yaw.sin();
    let cos = telemetry.yaw.cos();
    for location in &locations.locations {
        let dx = location.position[0] - telemetry.position[0];
        let dz = location.position[2] - telemetry.position[2];
        let right = dx * cos - dz * sin;
        let forward = dx * sin + dz * cos;
        let point = center + egui::vec2(right, -forward) * 0.04;
        if rect.shrink(5.0).contains(point) {
            let color = match location.kind {
                LocationKind::Gas => egui::Color32::from_rgb(140, 210, 30),
                LocationKind::Workshop => egui::Color32::from_rgb(80, 170, 255),
            };
            painter.circle_filled(point, 4.0, color);
        }
    }
    painter.text(
        rect.left_top() + egui::vec2(6.0, 5.0),
        egui::Align2::LEFT_TOP,
        "SERVICE MAP • 25 m refuel/service radius",
        egui::FontId::monospace(10.0),
        egui::Color32::GRAY,
    );
}

fn spawn_gamescope_classifier() {
    thread::spawn(|| {
        for _ in 0..50 {
            if classify_own_window().unwrap_or(false) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        eprintln!("overlay window was not found on DISPLAY; gamescope property was not set");
    });
}

fn screen_size() -> Result<[f32; 2], Box<dyn std::error::Error>> {
    let (connection, screen) = RustConnection::connect(None)?;
    let screen = &connection.setup().roots[screen];
    Ok([
        f32::from(screen.width_in_pixels),
        f32::from(screen.height_in_pixels),
    ])
}

fn classify_own_window() -> Result<bool, Box<dyn std::error::Error>> {
    let (connection, screen) = RustConnection::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let pid_atom = intern(&connection, b"_NET_WM_PID")?;
    let overlay_atom = intern(&connection, b"GAMESCOPE_EXTERNAL_OVERLAY")?;
    let pid = std::process::id();

    for window in descendants(&connection, root)? {
        let reply = connection
            .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)?
            .reply()?;
        if reply.value32().and_then(|mut values| values.next()) == Some(pid) {
            connection.change_property32(
                PropMode::REPLACE,
                window,
                overlay_atom,
                AtomEnum::CARDINAL,
                &[1],
            )?;
            connection.flush()?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn intern(connection: &RustConnection, name: &[u8]) -> Result<Atom, Box<dyn std::error::Error>> {
    Ok(connection.intern_atom(false, name)?.reply()?.atom)
}

fn descendants(
    connection: &RustConnection,
    root: Window,
) -> Result<Vec<Window>, Box<dyn std::error::Error>> {
    let mut pending = vec![root];
    let mut windows = Vec::new();
    while let Some(window) = pending.pop() {
        let children = connection.query_tree(window)?.reply()?.children;
        pending.extend(children.iter().copied());
        windows.extend(children);
    }
    Ok(windows)
}

fn spawn_menu_toggle() -> Arc<AtomicBool> {
    let open = Arc::new(AtomicBool::new(false));
    let thread_open = Arc::clone(&open);
    thread::spawn(move || {
        if let Err(error) = watch_menu_key(&thread_open) {
            eprintln!("could not register the L menu key: {error}");
        }
    });
    open
}

fn watch_menu_key(open: &AtomicBool) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, screen) = RustConnection::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let setup = connection.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let mapping = connection.get_keyboard_mapping(first, count)?.reply()?;
    let keycode = mapping
        .keysyms
        .chunks(usize::from(mapping.keysyms_per_keycode))
        .position(|symbols| symbols.contains(&u32::from(b'l')))
        .map(|index| first + index as u8)
        .ok_or("L key was not found in the X11 keymap")?;

    connection.grab_key(
        false,
        root,
        ModMask::ANY,
        keycode,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
    )?;
    connection.flush()?;
    let mut pressed = false;
    loop {
        match connection.wait_for_event()? {
            Event::KeyPress(event) if event.detail == keycode && !pressed => {
                open.fetch_xor(true, Ordering::Relaxed);
                pressed = true;
            }
            Event::KeyRelease(event) if event.detail == keycode => pressed = false,
            _ => {}
        }
    }
}

fn state_path() -> PathBuf {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("forzalife/vehicles.json")
}

fn locations_path() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("forzalife/locations.json")
}
