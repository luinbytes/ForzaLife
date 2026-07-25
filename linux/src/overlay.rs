use crate::{
    locations::{LocationKind, Locations},
    model::{LifeSnapshot, Simulation},
    telemetry::{PACKET_SIZE, Telemetry, parse},
};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
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
        xproto::{Atom, AtomEnum, ConnectionExt, GrabMode, ModMask, PropMode},
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

struct BoostGaugeState {
    car_ordinal: Option<i32>,
    visible: bool,
    current_boost: f32,
    min_display: i32,
    max_display: i32,
}

impl Default for BoostGaugeState {
    fn default() -> Self {
        Self {
            car_ordinal: None,
            visible: false,
            current_boost: 0.0,
            min_display: -1,
            max_display: 1,
        }
    }
}

impl BoostGaugeState {
    fn update(&mut self, telemetry: &Telemetry) {
        self.update_value(telemetry.car_ordinal, telemetry.boost_psi / 14.5038);
    }

    fn update_value(&mut self, car_ordinal: i32, boost: f32) {
        if self.car_ordinal != Some(car_ordinal) {
            *self = Self {
                car_ordinal: Some(car_ordinal),
                ..Self::default()
            };
        }
        self.current_boost = boost;
        self.visible |= boost > 0.11;
        if boost > self.max_display as f32 + 0.11 {
            self.max_display = boost.ceil() as i32;
        }
        if boost < self.min_display as f32 - 0.11 {
            self.min_display = (boost.floor() as i32).min(-1);
        }
    }
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
                classified: false,
                classification_error_reported: false,
                boost: BoostGaugeState::default(),
                menu_selection: 0,
                screen_size: [width, height],
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
    classified: bool,
    classification_error_reported: bool,
    boost: BoostGaugeState,
    menu_selection: usize,
    screen_size: [f32; 2],
    port: u16,
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, context: &egui::Context, frame: &mut eframe::Frame) {
        context.request_repaint_after(Duration::from_millis(50));
        if !self.classified {
            match classify_own_window(frame) {
                Ok(classified) => self.classified = classified,
                Err(error) if !self.classification_error_reported => {
                    eprintln!("could not classify gamescope overlay window: {error}");
                    self.classification_error_reported = true;
                }
                Err(_) => {}
            }
        }
        let menu_open = self.menu_open.load(Ordering::Relaxed);
        if menu_open != self.applied_menu_open {
            context.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(!menu_open));
            context.send_viewport_cmd(egui::ViewportCommand::CursorVisible(menu_open));
            if menu_open {
                context.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.menu_selection = 0;
            }
            self.applied_menu_open = menu_open;
        }
        let (telemetry, life, fresh, packets, rejected) = {
            let latest = self.latest.read().unwrap();
            (
                latest.telemetry.clone(),
                latest.life.clone(),
                latest
                    .received_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(2)),
                latest.packets,
                latest.rejected,
            )
        };

        let active_telemetry = telemetry.as_ref().filter(|_| fresh);
        if let Some(data) = active_telemetry {
            self.boost.update(data);
            render_race_hud(context, data, life.as_ref(), &self.boost, self.screen_size);
            let locations = self.locations.read().unwrap();
            render_navigation_hud(context, data, &locations, self.screen_size);
        } else {
            render_intro(
                context,
                fresh,
                packets,
                rejected,
                self.port,
                self.screen_size,
            );
        }

        if menu_open {
            let can_add = active_telemetry.is_some();
            if !can_add {
                self.menu_selection = 4;
            }
            if context.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
                self.menu_selection = move_menu_selection(self.menu_selection, -1, can_add);
            }
            if context.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                self.menu_selection = move_menu_selection(self.menu_selection, 1, can_add);
            }
            let mut action = context
                .input(|input| input.key_pressed(egui::Key::Enter))
                .then_some(self.menu_selection);
            let location_count = self.locations.read().unwrap().locations.len();
            egui::Area::new(egui::Id::new("forzalife-menu"))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(context, |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_black_alpha(221))
                        .stroke(egui::Stroke::new(2.5_f32, forza_pink()))
                        .inner_margin(egui::Margin::symmetric(75, 24))
                        .show(ui, |ui| {
                            ui.set_width(280.0);
                            ui.label(
                                egui::RichText::new("FORZALIFE")
                                    .strong()
                                    .size(18.0)
                                    .color(egui::Color32::WHITE),
                            );
                            ui.add_space(12.0);
                            let rows = [
                                "⛽ Add gas station here".to_owned(),
                                "🔧 Add workshop here".to_owned(),
                                format!("📍 {location_count} saved locations"),
                                "💼 Jobs".to_owned(),
                                "❌ Close menu".to_owned(),
                            ];
                            for (index, text) in rows.into_iter().enumerate() {
                                let enabled = index == 4 || (can_add && index < 2);
                                let button = egui::Button::new(egui::RichText::new(text).color(
                                    if enabled {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_white_alpha(90)
                                    },
                                ))
                                .fill(if self.menu_selection == index {
                                    forza_pink()
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(egui::Stroke::NONE);
                                let response = ui
                                    .add_enabled_ui(enabled, |ui| {
                                        ui.add_sized([280.0, 38.0], button)
                                    })
                                    .inner;
                                if response.clicked() {
                                    self.menu_selection = index;
                                    action = Some(index);
                                }
                            }
                        });
                });
            match action {
                Some(0) => {
                    if let Some(data) = active_telemetry {
                        self.add_location(LocationKind::Gas, data.position);
                    }
                }
                Some(1) => {
                    if let Some(data) = active_telemetry {
                        self.add_location(LocationKind::Workshop, data.position);
                    }
                }
                Some(4) => self.menu_open.store(false, Ordering::Relaxed),
                _ => {}
            }
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

fn forza_pink() -> egui::Color32 {
    egui::Color32::from_rgb(255, 1, 136)
}

fn forza_green() -> egui::Color32 {
    egui::Color32::from_rgb(202, 255, 3)
}

fn warning_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(255, 153, 0, 128)
}

fn move_menu_selection(current: usize, direction: i32, can_add: bool) -> usize {
    let enabled: &[usize] = if can_add { &[0, 1, 4] } else { &[4] };
    let position = enabled
        .iter()
        .position(|&index| index == current)
        .unwrap_or(0);
    enabled[(position as i32 + direction).rem_euclid(enabled.len() as i32) as usize]
}

fn render_intro(
    context: &egui::Context,
    fresh: bool,
    packets: u64,
    rejected: u64,
    port: u16,
    screen_size: [f32; 2],
) {
    egui::Area::new(egui::Id::new("forzalife-intro"))
        .fixed_pos([screen_size[0] - 410.0, screen_size[1] - 95.0])
        .show(context, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(204))
                .corner_radius(15)
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.set_width(350.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(if fresh { "⌛" } else { "●" })
                                .size(20.0)
                                .color(if fresh { forza_green() } else { forza_pink() }),
                        );
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("FORZALIFE")
                                    .size(15.0)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "Waiting for FH6 Data Out on 127.0.0.1:{port}"
                                ))
                                .size(13.0)
                                .color(egui::Color32::from_white_alpha(180)),
                            );
                        });
                    });
                    ui.small(format!("{packets} packets · {rejected} rejected"));
                });
        });
}

fn render_race_hud(
    context: &egui::Context,
    telemetry: &Telemetry,
    life: Option<&LifeSnapshot>,
    boost: &BoostGaugeState,
    screen_size: [f32; 2],
) {
    egui::Area::new(egui::Id::new("forzalife-race-hud"))
        .fixed_pos([
            (screen_size[0] - 400.0).max(0.0),
            (screen_size[1] - 291.0).max(0.0),
        ])
        .show(context, |ui| {
            ui.set_width(360.0);
            ui.horizontal(|ui| {
                if boost.visible {
                    boost_gauge(ui, boost);
                    ui.add_space(16.0);
                }
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("{:.0}", telemetry.speed_mps * 3.6))
                            .size(28.0)
                            .strong()
                            .italics()
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        egui::RichText::new("KM/H")
                            .size(12.0)
                            .strong()
                            .color(egui::Color32::from_white_alpha(128)),
                    );
                    ui.add_space(8.0);
                    if let Some(life) = life {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.0}", life.odometer_m / 1_000.0))
                                    .size(23.0)
                                    .strong()
                                    .italics()
                                    .color(egui::Color32::from_white_alpha(128)),
                            );
                            ui.label(
                                egui::RichText::new("KM")
                                    .size(14.0)
                                    .strong()
                                    .color(egui::Color32::from_white_alpha(128)),
                            );
                        });
                        ui.add_space(7.0);
                        fuel_and_oil(ui, life);
                    }
                });
            });
        });
}

fn boost_gauge(ui: &mut egui::Ui, boost: &BoostGaugeState) {
    let min_display = boost.min_display as f32;
    let max_display = boost.max_display as f32;
    let angle = boost_needle_angle(boost.current_boost, min_display, max_display);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(104.0, 104.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();

    let mut previous = None;
    for step in 0..=40 {
        let degrees = -135.0 + step as f32 / 40.0 * 170.0;
        let radians = degrees.to_radians();
        let point = center + egui::vec2(radians.cos(), radians.sin()) * 46.0;
        if let Some(previous) = previous {
            painter.line_segment(
                [previous, point],
                egui::Stroke::new(3.0_f32, egui::Color32::from_white_alpha(68)),
            );
        }
        previous = Some(point);
    }

    let radians = angle.to_radians();
    let needle = center + egui::vec2(radians.cos(), radians.sin()) * 42.0;
    painter.line_segment(
        [center, needle],
        egui::Stroke::new(4.0_f32, egui::Color32::WHITE),
    );
    painter.circle_filled(center, 5.0, forza_pink());
    painter.text(
        rect.left_bottom() + egui::vec2(20.0, -20.0),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}", min_display.abs()),
        egui::FontId::proportional(14.0),
        egui::Color32::WHITE,
    );
    painter.text(
        rect.right_top() + egui::vec2(-31.0, 20.0),
        egui::Align2::CENTER_CENTER,
        format!("{max_display:.0}"),
        egui::FontId::proportional(14.0),
        egui::Color32::WHITE,
    );
}

fn boost_needle_angle(boost: f32, min_display: f32, max_display: f32) -> f32 {
    let angle = if boost < 0.0 {
        -79.0 + boost / min_display.min(-1.0) * -56.0
    } else if boost > 0.0 {
        -79.0 + boost / max_display.max(1.0) * 114.0
    } else {
        -79.0
    };
    angle.clamp(-135.0, 35.0)
}

fn fuel_and_oil(ui: &mut egui::Ui, life: &LifeSnapshot) {
    let fuel = life.fuel_percent.clamp(0.0, 1.0);
    let fuel_color = if fuel <= 0.0 {
        egui::Color32::from_rgb(221, 0, 0)
    } else if fuel <= 0.2 {
        warning_color()
    } else {
        egui::Color32::from_white_alpha(128)
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(78.0, 15.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(60));
        let fill =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * fuel, rect.height()));
        painter.rect_filled(fill, 2.0, fuel_color);
        ui.label(
            egui::RichText::new(format!("{:.0}%", fuel * 100.0))
                .size(13.0)
                .strong()
                .color(fuel_color),
        );
        if life.oil_remaining_m <= 0.0 {
            ui.label(
                egui::RichText::new("◆ OIL")
                    .strong()
                    .color(egui::Color32::from_rgb(221, 0, 0)),
            );
        }
    });
}

fn render_navigation_hud(
    context: &egui::Context,
    telemetry: &Telemetry,
    locations: &Locations,
    screen_size: [f32; 2],
) {
    if locations.locations.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("forzalife-navigation"))
        .fixed_pos([32.0, (screen_size[1] - 180.0).max(0.0)])
        .show(context, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(110))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| navigation(ui, telemetry, locations));
        });
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

fn screen_size() -> Result<[f32; 2], Box<dyn std::error::Error>> {
    let (connection, screen) = RustConnection::connect(None)?;
    let screen = &connection.setup().roots[screen];
    Ok([
        f32::from(screen.width_in_pixels),
        f32::from(screen.height_in_pixels),
    ])
}

fn classify_own_window(frame: &eframe::Frame) -> Result<bool, Box<dyn std::error::Error>> {
    let mut window = match frame.window_handle()?.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window)?,
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return Ok(false),
    };
    let (connection, screen) = RustConnection::connect(None)?;
    let root = connection.setup().roots[screen].root;
    loop {
        let parent = connection.query_tree(window)?.reply()?.parent;
        if parent == root || parent == window {
            break;
        }
        window = parent;
    }
    let overlay_atom = intern(&connection, b"GAMESCOPE_EXTERNAL_OVERLAY")?;
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            overlay_atom,
            AtomEnum::CARDINAL,
            &[1],
        )?
        .check()?;
    connection.flush()?;
    Ok(true)
}

fn intern(connection: &RustConnection, name: &[u8]) -> Result<Atom, Box<dyn std::error::Error>> {
    Ok(connection.intern_atom(false, name)?.reply()?.atom)
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

#[cfg(test)]
mod tests {
    use super::{BoostGaugeState, boost_needle_angle, move_menu_selection};

    #[test]
    fn boost_needle_matches_the_windows_gauge_endpoints() {
        assert_eq!(boost_needle_angle(0.0, -1.0, 1.0), -79.0);
        assert_eq!(boost_needle_angle(-1.0, -1.0, 1.0), -135.0);
        assert_eq!(boost_needle_angle(1.0, -1.0, 1.0), 35.0);
    }

    #[test]
    fn boost_scale_persists_until_the_car_changes() {
        let mut boost = BoostGaugeState::default();
        boost.update_value(1, 2.2);
        boost.update_value(1, 0.0);
        assert_eq!(boost.max_display, 3);

        boost.update_value(2, 0.0);
        assert_eq!(boost.max_display, 1);
        assert!(!boost.visible);
    }

    #[test]
    fn menu_navigation_wraps_over_enabled_rows() {
        assert_eq!(move_menu_selection(0, -1, true), 4);
        assert_eq!(move_menu_selection(1, 1, true), 4);
        assert_eq!(move_menu_selection(4, 1, true), 0);
        assert_eq!(move_menu_selection(0, 1, false), 4);
    }
}
