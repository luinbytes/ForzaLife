use crate::{
    car::{CarDatabase, CarInfo},
    locations::{LocationKind, Locations},
    menu::{MenuEffect, MenuPage, MenuState},
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
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xinput::{ConnectionExt as _, EventMask, XIEventMask},
        xproto::{Atom, AtomEnum, ConnectionExt, PropMode},
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

const WINDOW_TITLE: &str = "ForzaLife Linux Overlay";
const MENU_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy)]
enum InputEvent {
    Primary,
    Up,
    Down,
}

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
    let cars = Arc::new(CarDatabase::bundled());
    let simulation = Arc::new(RwLock::new(Simulation::load(&state_path)));
    spawn_receiver(
        port,
        Arc::clone(&latest),
        Arc::clone(&locations),
        Arc::clone(&cars),
        Arc::clone(&simulation),
        state_path,
    );
    let input = spawn_input_listener();
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
                cars,
                simulation,
                input,
                menu: MenuState::default(),
                last_menu_activity: Instant::now(),
                navigation_target: None,
                classified: false,
                classification_error_reported: false,
                boost: BoostGaugeState::default(),
                screen_size: [width, height],
                port,
            }))
        }),
    )
}

struct OverlayApp {
    latest: Arc<RwLock<Latest>>,
    locations: Arc<RwLock<Locations>>,
    cars: Arc<CarDatabase>,
    simulation: Arc<RwLock<Simulation>>,
    input: Receiver<InputEvent>,
    menu: MenuState,
    last_menu_activity: Instant,
    navigation_target: Option<LocationKind>,
    classified: bool,
    classification_error_reported: bool,
    boost: BoostGaugeState,
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
        let (telemetry, mut life, fresh, packets, rejected) = {
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
        if let Some(updated) =
            self.handle_input(active_telemetry.map(|telemetry| telemetry.car_ordinal))
        {
            life = Some(updated);
        }
        if matches!(self.menu.page(), MenuPage::Main | MenuPage::Navigation)
            && self.last_menu_activity.elapsed() >= MENU_TIMEOUT
        {
            self.menu.close();
        }

        if let Some(data) = active_telemetry {
            self.boost.update(data);
            render_race_hud(context, data, life.as_ref(), &self.boost, self.screen_size);
            let locations = self.locations.read().unwrap();
            if let Some(target) = self.navigation_target {
                if locations
                    .nearest(target, data.position)
                    .is_some_and(|(_, distance)| distance <= 2.0)
                {
                    self.navigation_target = None;
                } else {
                    render_navigation_hud(context, data, &locations, target, self.screen_size);
                }
            }
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

        let menu_opacity = context.animate_bool_with_time(
            egui::Id::new("forzalife-menu-opacity"),
            matches!(self.menu.page(), MenuPage::Main | MenuPage::Navigation),
            0.2,
        );
        let card_opacity = context.animate_bool_with_time(
            egui::Id::new("forzalife-card-opacity"),
            self.menu.page() == MenuPage::VehicleCard,
            0.25,
        );
        match self.menu.page() {
            MenuPage::Main => render_menu(
                context,
                &main_menu_rows(life.as_ref().is_some_and(|life| life.is_usage_paused)),
                self.menu.selected(),
                menu_opacity,
            ),
            MenuPage::Navigation => render_menu(
                context,
                &navigation_menu_rows(),
                self.menu.selected(),
                menu_opacity,
            ),
            MenuPage::VehicleCard => {
                if let (Some(data), Some(life)) = (active_telemetry, life.as_ref()) {
                    render_vehicle_card(
                        context,
                        self.cars.get(data.car_ordinal),
                        life,
                        data.num_cylinders,
                        card_opacity,
                    );
                } else {
                    self.menu.close();
                }
            }
            MenuPage::Closed => {}
        }
    }
}

impl OverlayApp {
    fn handle_input(&mut self, car_ordinal: Option<i32>) -> Option<LifeSnapshot> {
        let mut updated_life = None;
        while let Ok(input) = self.input.try_recv() {
            let Some(car_ordinal) = car_ordinal else {
                continue;
            };
            self.last_menu_activity = Instant::now();
            match input {
                InputEvent::Up => self.menu.up(),
                InputEvent::Down => self.menu.down(),
                InputEvent::Primary => match self.menu.primary() {
                    MenuEffect::None => {}
                    MenuEffect::ToggleUsage => {
                        let mut simulation = self.simulation.write().unwrap();
                        simulation.toggle_usage(car_ordinal);
                        updated_life = simulation.current(car_ordinal);
                        self.latest.write().unwrap().life = updated_life.clone();
                    }
                    MenuEffect::SetNavigation(target) => self.navigation_target = target,
                },
            }
        }
        updated_life
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

struct MenuRow {
    icon: &'static str,
    text: String,
    enabled: bool,
}

fn main_menu_rows(usage_paused: bool) -> Vec<MenuRow> {
    vec![
        MenuRow {
            icon: "🌐",
            text: "Set mini nav".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "🚗",
            text: "Vehicle info card".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: if usage_paused { "▶" } else { "⏸" },
            text: if usage_paused {
                "Resume fuel usage for this car"
            } else {
                "Pause fuel usage for this car"
            }
            .to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "💼",
            text: "Jobs".to_owned(),
            enabled: false,
        },
        MenuRow {
            icon: "❌",
            text: "Close menu".to_owned(),
            enabled: true,
        },
    ]
}

fn navigation_menu_rows() -> Vec<MenuRow> {
    vec![
        MenuRow {
            icon: "⛽",
            text: "Closest gas station".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "🔧",
            text: "Closest workshop".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "📴",
            text: "End navigation".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "↩",
            text: "Back".to_owned(),
            enabled: true,
        },
    ]
}

fn render_menu(context: &egui::Context, rows: &[MenuRow], selected: usize, opacity: f32) {
    egui::Area::new(egui::Id::new("forzalife-menu"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            ui.set_opacity(opacity);
            ui.set_width(430.0);
            ui.vertical_centered(|ui| {
                for (index, row) in rows.iter().enumerate() {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(280.0, 46.0), egui::Sense::hover());
                    let selected = selected == index;
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(
                        rect,
                        0.0,
                        if selected {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_black_alpha(221)
                        },
                    );
                    let color = if selected {
                        egui::Color32::BLACK
                    } else if row.enabled {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_white_alpha(102)
                    };
                    painter.text(
                        rect.left_center() + egui::vec2(18.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        row.icon,
                        egui::FontId::proportional(20.0),
                        color,
                    );
                    painter.text(
                        rect.left_center() + egui::vec2(58.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &row.text,
                        egui::FontId::proportional(16.0),
                        color,
                    );
                    ui.add_space(3.0);
                }
            });
        });
}

fn render_vehicle_card(
    context: &egui::Context,
    car: Option<&CarInfo>,
    life: &LifeSnapshot,
    num_cylinders: i32,
    opacity: f32,
) {
    let title = car
        .map(CarInfo::display_name)
        .unwrap_or_else(|| format!("CAR {}", life.car_ordinal));
    let year = car
        .filter(|car| car.year > 0)
        .map(|car| car.year.to_string())
        .unwrap_or_default();
    let country = car.map(|car| car.country.clone()).unwrap_or_default();
    let tank_liters = car
        .map(|car| car.tank_capacity_liters(num_cylinders))
        .unwrap_or_else(|| CarInfo::fallback_tank_capacity_liters(num_cylinders));
    let fuel_color = if life.fuel_percent <= 0.0 {
        egui::Color32::RED
    } else if life.fuel_percent <= 0.2 {
        egui::Color32::from_rgb(255, 153, 0)
    } else {
        egui::Color32::WHITE
    };
    let oil_color = if life.oil_remaining_m <= 0.0 {
        egui::Color32::RED
    } else {
        egui::Color32::WHITE
    };
    let mut items = vec![
        ("Production year", year, egui::Color32::WHITE),
        ("Country", country, egui::Color32::WHITE),
    ];
    items.push(if num_cylinders == 0 {
        (
            "Battery level",
            format!("{:.1} / {tank_liters:.0} kW", life.fuel_liters),
            fuel_color,
        )
    } else {
        (
            "Fuel level",
            format!("{:.1} / {tank_liters:.0} L", life.fuel_liters),
            fuel_color,
        )
    });
    items.extend([
        (
            "Trip Odometer",
            format!("{:.1} km", life.trip_m / 1_000.0),
            egui::Color32::WHITE,
        ),
        (
            "Odometer",
            format!("{:.0} km", life.odometer_m / 1_000.0),
            egui::Color32::WHITE,
        ),
    ]);
    if num_cylinders != 0 {
        items.push((
            "Next oil service at",
            format!(
                "{:.0} km",
                (life.odometer_m + life.oil_remaining_m) / 1_000.0
            ),
            oil_color,
        ));
    }

    egui::Area::new(egui::Id::new("forzalife-vehicle-card"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            ui.set_opacity(opacity);
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(221))
                .inner_margin(egui::Margin::symmetric(30, 24))
                .show(ui, |ui| {
                    ui.set_width(720.0);
                    ui.label(
                        egui::RichText::new(title)
                            .size(28.0)
                            .strong()
                            .italics()
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        egui::RichText::new("Vehicle Info Card")
                            .size(15.0)
                            .color(forza_pink()),
                    );
                    ui.add_space(22.0);
                    for row in items.chunks(3) {
                        ui.columns(3, |columns| {
                            for (column, (label, value, color)) in
                                columns.iter_mut().zip(row.iter())
                            {
                                column.label(
                                    egui::RichText::new(*label)
                                        .size(12.0)
                                        .color(egui::Color32::from_white_alpha(140)),
                                );
                                column.label(
                                    egui::RichText::new(value).size(19.0).strong().color(*color),
                                );
                            }
                        });
                        ui.add_space(18.0);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Press [L] to close")
                                .size(12.0)
                                .color(egui::Color32::from_white_alpha(150)),
                        );
                    });
                });
        });
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
                        if !life.is_usage_paused {
                            fuel_and_oil(ui, life);
                        }
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
    target: LocationKind,
    screen_size: [f32; 2],
) {
    let Some((location, distance)) = locations.nearest(target, telemetry.position) else {
        return;
    };
    egui::Area::new(egui::Id::new("forzalife-navigation"))
        .fixed_pos([32.0, (screen_size[1] - 180.0).max(0.0)])
        .show(context, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(221))
                .inner_margin(egui::Margin::symmetric(15, 10))
                .show(ui, |ui| {
                    navigation(ui, telemetry, location.position, distance)
                });
        });
}

fn spawn_receiver(
    port: u16,
    latest: Arc<RwLock<Latest>>,
    locations: Arc<RwLock<Locations>>,
    cars: Arc<CarDatabase>,
    simulation: Arc<RwLock<Simulation>>,
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
        let mut last_save = Instant::now();
        let mut packet = [0_u8; PACKET_SIZE + 1];
        loop {
            match socket.recv(&mut packet) {
                Ok(size) => match parse(&packet[..size]) {
                    Ok(telemetry) => {
                        let tank_liters = cars
                            .get(telemetry.car_ordinal)
                            .map(|car| car.tank_capacity_liters(telemetry.num_cylinders))
                            .unwrap_or_else(|| {
                                CarInfo::fallback_tank_capacity_liters(telemetry.num_cylinders)
                            });
                        let mut simulation = simulation.write().unwrap();
                        let mut life = simulation.update_with_capacity(&telemetry, tank_liters);
                        if telemetry.speed_mps.abs() < 1.0 {
                            let locations = locations.read().unwrap();
                            if locations
                                .nearest(LocationKind::Gas, telemetry.position)
                                .is_some_and(|(_, distance)| distance <= 25.0)
                            {
                                simulation.refuel(telemetry.car_ordinal);
                                life = simulation.update_with_capacity(&telemetry, tank_liters);
                            }
                            if locations
                                .nearest(LocationKind::Workshop, telemetry.position)
                                .is_some_and(|(_, distance)| distance <= 25.0)
                            {
                                simulation.service_oil(telemetry.car_ordinal);
                                life = simulation.update_with_capacity(&telemetry, tank_liters);
                            }
                        }
                        if last_save.elapsed() >= Duration::from_secs(5) {
                            if let Err(error) = simulation.save(&state_path) {
                                eprintln!("could not save vehicle state: {error}");
                            }
                            last_save = Instant::now();
                        }
                        drop(simulation);

                        let mut state = latest.write().unwrap();
                        state.telemetry = Some(telemetry);
                        state.life = Some(life);
                        state.received_at = Some(Instant::now());
                        state.packets += 1;
                    }
                    Err(_) => latest.write().unwrap().rejected += 1,
                },
                Err(error) => {
                    eprintln!("UDP receive failed: {error}");
                    return;
                }
            }
        }
    });
}

fn navigation(ui: &mut egui::Ui, telemetry: &Telemetry, target_position: [f32; 3], distance: f32) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let angle = (target_position[0] - telemetry.position[0])
            .atan2(target_position[2] - telemetry.position[2])
            - telemetry.yaw;
        let direction = egui::vec2(angle.sin(), -angle.cos());
        let right = egui::vec2(-direction.y, direction.x);
        painter.add(egui::Shape::convex_polygon(
            vec![
                rect.center() + direction * 24.0,
                rect.center() - direction * 14.0 + right * 11.0,
                rect.center() - direction * 8.0,
                rect.center() - direction * 14.0 - right * 11.0,
            ],
            egui::Color32::WHITE,
            egui::Stroke::NONE,
        ));
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let (value, unit) = if distance >= 1_000.0 {
                (format!("{:.1}", distance / 1_000.0), "KM")
            } else {
                (format!("{distance:.0}"), "M")
            };
            ui.label(
                egui::RichText::new(value)
                    .size(28.0)
                    .strong()
                    .italics()
                    .color(egui::Color32::WHITE),
            );
            ui.label(
                egui::RichText::new(unit)
                    .size(12.0)
                    .strong()
                    .color(egui::Color32::from_white_alpha(128)),
            );
        });
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

fn spawn_input_listener() -> Receiver<InputEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        if let Err(error) = watch_input(sender) {
            eprintln!("could not register the ForzaLife menu keys: {error}");
        }
    });
    receiver
}

fn watch_input(sender: Sender<InputEvent>) -> Result<(), Box<dyn std::error::Error>> {
    let (connection, screen) = RustConnection::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let setup = connection.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let mapping = connection.get_keyboard_mapping(first, count)?.reply()?;
    let keycode = |symbol: u8| {
        mapping
            .keysyms
            .chunks(usize::from(mapping.keysyms_per_keycode))
            .position(|symbols| symbols.contains(&u32::from(symbol)))
            .map(|index| first + index as u8)
    };
    let bindings = [
        (
            keycode(b'l').ok_or("L key was not found in the X11 keymap")?,
            InputEvent::Primary,
        ),
        (
            keycode(b';').ok_or("semicolon key was not found in the X11 keymap")?,
            InputEvent::Down,
        ),
        (
            keycode(b'\'').ok_or("apostrophe key was not found in the X11 keymap")?,
            InputEvent::Up,
        ),
    ];
    connection.xinput_xi_query_version(2, 0)?.reply()?;
    connection
        .xinput_xi_select_events(
            root,
            &[EventMask {
                deviceid: 1,
                mask: vec![XIEventMask::RAW_KEY_PRESS | XIEventMask::RAW_KEY_RELEASE],
            }],
        )?
        .check()?;
    connection.flush()?;
    let mut pressed = [false; 256];
    loop {
        match connection.wait_for_event()? {
            Event::XinputRawKeyPress(event)
                if event.detail < 256 && !pressed[event.detail as usize] =>
            {
                if let Some((_, input)) = bindings
                    .iter()
                    .find(|(keycode, _)| u32::from(*keycode) == event.detail)
                {
                    sender
                        .send(*input)
                        .map_err(|_| "overlay input receiver closed")?;
                    pressed[event.detail as usize] = true;
                }
            }
            Event::XinputRawKeyRelease(event) if event.detail < 256 => {
                pressed[event.detail as usize] = false;
            }
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
    use super::{BoostGaugeState, boost_needle_angle};

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
}
