use crate::{
    car::{CarDatabase, CarInfo},
    driving::{DriveSession, DriveSnapshot},
    input_proxy::InputProxy,
    locations::{LocationKind, Locations},
    malfunctions::{FuelStarvation, StarvationAction},
    menu::{HudMode, MenuEffect, MenuPage, MenuState},
    model::{LifeSnapshot, Simulation},
    telemetry::{PACKET_SIZE, Telemetry, parse},
    wayland_overlay,
};
use egui;
use std::{
    env,
    net::UdpSocket,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::Command,
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
        xproto::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

const MENU_TIMEOUT: Duration = Duration::from_secs(4);
const INPUT_PROXY_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const REFUEL_LITERS_PER_SECOND: f32 = 2.25;
const MENU_SIZE: [f32; 2] = [290.0, 384.0];
const VEHICLE_CARD_SIZE: [f32; 2] = [540.0, 340.0];
const HUD_SIZE: [f32; 2] = [400.0, 291.0];
const ROBOTO_MEDIUM: &str = "Roboto Condensed Medium";
const ROBOTO_SEMIBOLD: &str = "Roboto Condensed Semibold";

#[derive(Clone, Copy)]
enum InputEvent {
    Primary,
    Up,
    Down,
    Back,
    Digit(char),
    Decimal,
    Backspace,
    Confirm,
    Cancel,
}

#[derive(Default)]
struct Latest {
    telemetry: Option<Telemetry>,
    life: Option<LifeSnapshot>,
    refuel_available: bool,
    refueling: bool,
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
            min_display: -15,
            max_display: 15,
        }
    }
}

impl BoostGaugeState {
    fn update(&mut self, telemetry: &Telemetry) {
        self.update_value(telemetry.car_ordinal, telemetry.boost_psi);
    }

    fn update_value(&mut self, car_ordinal: i32, boost: f32) {
        if self.car_ordinal != Some(car_ordinal) {
            *self = Self {
                car_ordinal: Some(car_ordinal),
                ..Self::default()
            };
        }
        self.current_boost = boost;
        self.visible |= boost > 1.6;
        if boost > self.max_display as f32 + 1.6 {
            self.max_display = ((boost / 5.0).ceil() as i32 * 5).max(15);
        }
        if boost < self.min_display as f32 - 1.6 {
            self.min_display = ((boost / 5.0).floor() as i32 * 5).min(-15);
        }
    }
}

pub fn run(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let latest = Arc::new(RwLock::new(Latest::default()));
    let state_path = state_path();
    let locations = Arc::new(Locations::bundled());
    let cars = Arc::new(CarDatabase::bundled());
    let simulation = Arc::new(RwLock::new(Simulation::load(&state_path)));
    spawn_receiver(
        port,
        Arc::clone(&latest),
        Arc::clone(&locations),
        Arc::clone(&cars),
        Arc::clone(&simulation),
        state_path.clone(),
    );
    let input = spawn_input_listener();
    let reload_simulation = Arc::clone(&simulation);
    let reload_state_path = state_path.clone();
    let reload_requested = wayland_overlay::run(move |context, screen_size| {
        install_forza_fonts(context);
        context.set_visuals(egui::Visuals {
            panel_fill: egui::Color32::TRANSPARENT,
            window_fill: egui::Color32::TRANSPARENT,
            ..egui::Visuals::dark()
        });
        let boost_background = load_boost_background(context);
        let hud_icons = load_hud_icons(context);
        let mut app = OverlayApp {
            latest,
            locations,
            cars,
            simulation,
            input,
            menu: MenuState::default(),
            odometer_input: String::new(),
            odometer_error: None,
            fuel_input: String::new(),
            fuel_error: None,
            last_menu_page: MenuPage::Main,
            last_menu_activity: Instant::now(),
            navigation_target: None,
            boost: BoostGaugeState::default(),
            boost_background,
            hud_icons,
            state_path,
            screen_size,
            port,
            hud_mode: HudMode::default(),
            session: DriveSession::default(),
        };
        move |context: &egui::Context, screen_size| {
            app.screen_size = screen_size;
            app.update(context)
        }
    })?;
    if !reload_requested {
        return Ok(());
    }

    reload_simulation
        .read()
        .unwrap()
        .save(&reload_state_path)
        .map_err(|error| format!("could not save before overlay reload: {error}"))?;
    Err(Command::new(env::current_exe()?)
        .arg("overlay")
        .arg(port.to_string())
        .exec()
        .into())
}

struct OverlayApp {
    latest: Arc<RwLock<Latest>>,
    locations: Arc<Locations>,
    cars: Arc<CarDatabase>,
    simulation: Arc<RwLock<Simulation>>,
    input: Receiver<InputEvent>,
    menu: MenuState,
    odometer_input: String,
    odometer_error: Option<String>,
    fuel_input: String,
    fuel_error: Option<String>,
    last_menu_page: MenuPage,
    last_menu_activity: Instant,
    navigation_target: Option<LocationKind>,
    boost: BoostGaugeState,
    boost_background: egui::TextureHandle,
    hud_icons: HudIcons,
    state_path: PathBuf,
    screen_size: [f32; 2],
    port: u16,
    hud_mode: HudMode,
    session: DriveSession,
}

impl OverlayApp {
    fn update(&mut self, context: &egui::Context) -> bool {
        let (telemetry, mut life, refuel_available, refueling, fresh, packets) = {
            let latest = self.latest.read().unwrap();
            (
                latest.telemetry.clone(),
                latest.life.clone(),
                latest.refuel_available,
                latest.refueling,
                latest
                    .received_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(2)),
                latest.packets,
            )
        };

        let active_telemetry = telemetry
            .as_ref()
            .filter(|telemetry| fresh && telemetry.car_ordinal > 0);
        let (updated_life, reload) =
            self.handle_input(active_telemetry.map(|telemetry| telemetry.car_ordinal));
        if let Some(updated) = updated_life {
            life = Some(updated);
        }
        if reload {
            return true;
        }
        if matches!(self.menu.page(), MenuPage::Main | MenuPage::Navigation)
            && self.last_menu_activity.elapsed() >= MENU_TIMEOUT
        {
            self.menu.close();
        }

        if let Some(data) = active_telemetry {
            self.boost.update(data);
            let drive = DriveSnapshot::from_telemetry(data);
            if let Some(life) = life.as_ref() {
                self.session.update(data, life);
            }
            render_race_hud(
                context,
                data,
                life.as_ref(),
                &self.boost,
                &self.boost_background,
                &self.hud_icons,
                self.screen_size,
                self.hud_mode,
            );
            render_drive_hud(
                context,
                &drive,
                &self.session,
                self.screen_size,
                self.hud_mode,
            );
            if refuel_available && let Some(life) = life.as_ref() {
                render_refueling(context, life, refueling, self.screen_size);
            }
            if let Some(target) = self.navigation_target {
                if self
                    .locations
                    .nearest(target, data.position)
                    .is_some_and(|(_, distance)| distance <= 2.0)
                {
                    self.navigation_target = None;
                } else {
                    render_navigation_hud(
                        context,
                        data,
                        &self.locations,
                        target,
                        &self.hud_icons,
                        self.screen_size,
                    );
                }
            }
        } else {
            render_intro(context, packets, self.port);
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
        if matches!(self.menu.page(), MenuPage::Main | MenuPage::Navigation) {
            self.last_menu_page = self.menu.page();
        }
        if menu_opacity > 0.0 {
            let rows = match self.last_menu_page {
                MenuPage::Navigation => navigation_menu_rows(),
                _ => main_menu_rows(
                    life.as_ref().is_some_and(|life| life.is_usage_paused),
                    self.hud_mode,
                ),
            };
            render_menu(
                context,
                &rows,
                self.menu.selected(),
                menu_opacity,
                self.screen_size,
            );
        }
        if card_opacity > 0.0 {
            if let (Some(data), Some(life)) = (active_telemetry, life.as_ref()) {
                render_vehicle_card(
                    context,
                    self.cars.get(data.car_ordinal),
                    life,
                    data.num_cylinders,
                    card_opacity,
                    self.screen_size,
                );
            } else {
                self.menu.close();
            }
        }
        if self.menu.page() == MenuPage::OdometerInput
            && let Some(life) = life.as_ref()
        {
            render_odometer_input(
                context,
                life.odometer_m,
                &self.odometer_input,
                self.odometer_error.as_deref(),
                self.screen_size,
            );
        }
        if self.menu.page() == MenuPage::FuelInput
            && let Some(life) = life.as_ref()
        {
            render_fuel_input(
                context,
                life.fuel_percent,
                &self.fuel_input,
                self.fuel_error.as_deref(),
                self.screen_size,
            );
        }
        false
    }
    fn handle_input(&mut self, car_ordinal: Option<i32>) -> (Option<LifeSnapshot>, bool) {
        let mut updated_life = None;
        while let Ok(input) = self.input.try_recv() {
            self.last_menu_activity = Instant::now();
            if self.menu.page() == MenuPage::FuelInput {
                match input {
                    InputEvent::Digit(digit) => {
                        self.fuel_error = None;
                        if self.fuel_input.len() < 5 {
                            self.fuel_input.push(digit);
                        }
                    }
                    InputEvent::Decimal => {
                        self.fuel_error = None;
                        if !self.fuel_input.contains('.') && self.fuel_input.len() < 5 {
                            self.fuel_input.push('.');
                        }
                    }
                    InputEvent::Backspace => {
                        self.fuel_error = None;
                        self.fuel_input.pop();
                    }
                    InputEvent::Primary | InputEvent::Confirm => {
                        let fuel_percent = self
                            .fuel_input
                            .parse::<f32>()
                            .ok()
                            .filter(|value| (0.0..=100.0).contains(value));
                        match (car_ordinal, fuel_percent) {
                            (Some(car_ordinal), Some(fuel_percent)) => {
                                let mut simulation = self.simulation.write().unwrap();
                                match simulation.set_fuel_percent_and_save(
                                    &self.state_path,
                                    car_ordinal,
                                    fuel_percent / 100.0,
                                ) {
                                    Ok(life) => {
                                        updated_life = Some(life);
                                        self.latest.write().unwrap().life = updated_life.clone();
                                        self.menu.close();
                                        self.fuel_error = None;
                                    }
                                    Err(error) => {
                                        eprintln!("could not save vehicle state: {error}");
                                        self.fuel_error = Some(
                                            "Could not save. Press Enter to retry.".to_owned(),
                                        );
                                    }
                                }
                            }
                            (None, _) => {
                                self.fuel_error = Some("Waiting for vehicle telemetry.".to_owned());
                            }
                            (_, None) => {
                                self.fuel_error =
                                    Some("Enter a percentage from 0 to 100.".to_owned());
                            }
                        }
                    }
                    InputEvent::Back | InputEvent::Cancel => {
                        self.menu.back();
                        self.fuel_input.clear();
                        self.fuel_error = None;
                    }
                    InputEvent::Up | InputEvent::Down => {}
                }
                continue;
            }
            if self.menu.page() == MenuPage::OdometerInput {
                match input {
                    InputEvent::Digit(digit) => {
                        self.odometer_error = None;
                        if self.odometer_input.len() < 10 {
                            self.odometer_input.push(digit);
                        }
                    }
                    InputEvent::Decimal => {
                        self.odometer_error = None;
                        if !self.odometer_input.contains('.') && self.odometer_input.len() < 10 {
                            self.odometer_input.push('.');
                        }
                    }
                    InputEvent::Backspace => {
                        self.odometer_error = None;
                        self.odometer_input.pop();
                    }
                    InputEvent::Primary | InputEvent::Confirm => {
                        let kilometers = self
                            .odometer_input
                            .parse::<f32>()
                            .ok()
                            .filter(|value| (0.0..=10_000_000.0).contains(value));
                        match (car_ordinal, kilometers) {
                            (Some(car_ordinal), Some(kilometers)) => {
                                let mut simulation = self.simulation.write().unwrap();
                                match simulation.set_odometer_and_save(
                                    &self.state_path,
                                    car_ordinal,
                                    kilometers * 1_000.0,
                                ) {
                                    Ok(life) => {
                                        updated_life = Some(life);
                                        self.latest.write().unwrap().life = updated_life.clone();
                                        self.menu.close();
                                        self.odometer_error = None;
                                    }
                                    Err(error) => {
                                        eprintln!("could not save vehicle state: {error}");
                                        self.odometer_error = Some(
                                            "Could not save. Press Enter to retry.".to_owned(),
                                        );
                                    }
                                }
                            }
                            (None, _) => {
                                self.odometer_error =
                                    Some("Waiting for vehicle telemetry.".to_owned());
                            }
                            (_, None) => {
                                self.odometer_error = Some("Enter a valid mileage.".to_owned());
                            }
                        }
                    }
                    InputEvent::Back | InputEvent::Cancel => {
                        self.menu.back();
                        self.odometer_input.clear();
                        self.odometer_error = None;
                    }
                    InputEvent::Up | InputEvent::Down => {}
                }
                continue;
            }

            match input {
                InputEvent::Up => self.menu.up(),
                InputEvent::Down => self.menu.down(),
                InputEvent::Back | InputEvent::Cancel => self.menu.back(),
                InputEvent::Primary | InputEvent::Confirm => match self.menu.primary() {
                    MenuEffect::None => {}
                    MenuEffect::ReloadOverlay => return (updated_life, true),
                    MenuEffect::ToggleUsage => {
                        let Some(car_ordinal) = car_ordinal else {
                            continue;
                        };
                        let mut simulation = self.simulation.write().unwrap();
                        simulation.toggle_usage(car_ordinal);
                        updated_life = simulation.current(car_ordinal);
                        self.latest.write().unwrap().life = updated_life.clone();
                    }
                    MenuEffect::OpenOdometerInput => {
                        self.odometer_input.clear();
                        self.odometer_error = None;
                    }
                    MenuEffect::OpenFuelInput => {
                        self.fuel_input.clear();
                        self.fuel_error = None;
                    }
                    MenuEffect::SetNavigation(target) => self.navigation_target = target,
                    MenuEffect::CycleHudMode => {
                        self.hud_mode = match self.hud_mode {
                            HudMode::Life => HudMode::Drive,
                            HudMode::Drive => HudMode::Minimal,
                            HudMode::Minimal => HudMode::Life,
                        };
                    }
                    MenuEffect::ResetSession => self.session.reset(),
                },
                InputEvent::Digit(_) | InputEvent::Decimal | InputEvent::Backspace => {}
            }
        }
        (updated_life, false)
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

fn install_forza_fonts(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "forza-medium".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/robotocondensed-mediumitalic.ttf"
        ))),
    );
    fonts.font_data.insert(
        "forza-semibold".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/robotocondensed-semibolditalic.ttf"
        ))),
    );
    let mut medium = vec!["forza-medium".to_owned()];
    medium.extend(fonts.families[&egui::FontFamily::Proportional].clone());
    fonts
        .families
        .insert(egui::FontFamily::Name(ROBOTO_MEDIUM.into()), medium);
    let mut semibold = vec!["forza-semibold".to_owned()];
    semibold.extend(fonts.families[&egui::FontFamily::Proportional].clone());
    fonts
        .families
        .insert(egui::FontFamily::Name(ROBOTO_SEMIBOLD.into()), semibold);
    context.set_fonts(fonts);
}

fn load_boost_background(context: &egui::Context) -> egui::TextureHandle {
    load_texture(
        context,
        "forzalife-boost-background",
        include_bytes!("../assets/boost_bg.png"),
    )
}

struct HudIcons {
    fuel: egui::TextureHandle,
    battery: egui::TextureHandle,
    oil: egui::TextureHandle,
    workshop: egui::TextureHandle,
}

fn load_hud_icons(context: &egui::Context) -> HudIcons {
    HudIcons {
        fuel: load_texture(
            context,
            "forzalife-fuel-icon",
            include_bytes!("../assets/fuel_icon.png"),
        ),
        battery: load_texture(
            context,
            "forzalife-fuel-battery-icon",
            include_bytes!("../assets/fuel_battery_icon.png"),
        ),
        oil: load_texture(
            context,
            "forzalife-oil-icon",
            include_bytes!("../assets/oil_icon.png"),
        ),
        workshop: load_texture(
            context,
            "forzalife-workshop-icon",
            include_bytes!("../assets/workshop_icon.png"),
        ),
    }
}

fn load_texture(context: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
    let image = image::load_from_memory(bytes)
        .expect("bundled overlay texture")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    context.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    )
}

struct MenuRow {
    icon: &'static str,
    text: String,
    enabled: bool,
}

fn main_menu_rows(usage_paused: bool, hud_mode: HudMode) -> Vec<MenuRow> {
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
            icon: "KM",
            text: "Set odometer".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "%",
            text: "Set fuel level (testing)".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "⟳",
            text: "Reset driving session".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "HUD",
            text: format!("HUD mode: {}", hud_mode_label(hud_mode)),
            enabled: true,
        },
        MenuRow {
            icon: "↺",
            text: "Reset drive session".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "HUD",
            text: "Cycle HUD mode".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "❌",
            text: "Close menu".to_owned(),
            enabled: true,
        },
        MenuRow {
            icon: "↻",
            text: "Reload overlay".to_owned(),
            enabled: true,
        },
    ]
}

fn hud_mode_label(mode: HudMode) -> &'static str {
    match mode {
        HudMode::Life => "Life",
        HudMode::Drive => "Drive",
        HudMode::Minimal => "Minimal",
    }
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

fn menu_rect(screen_size: [f32; 2]) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(20.0, screen_size[1] - 430.0 - MENU_SIZE[1]),
        egui::vec2(MENU_SIZE[0], MENU_SIZE[1]),
    )
}

fn render_menu(
    context: &egui::Context,
    rows: &[MenuRow],
    selected: usize,
    opacity: f32,
    screen_size: [f32; 2],
) {
    let target = menu_rect(screen_size);
    egui::Area::new(egui::Id::new("forzalife-menu"))
        .fixed_pos(target.min)
        .show(context, |ui| {
            ui.set_opacity(opacity);
            let (base, _) = ui.allocate_exact_size(target.size(), egui::Sense::hover());
            let scale = 0.9 + opacity * 0.1;
            let scaled = egui::Rect::from_center_size(base.center(), base.size() * scale);
            let painter = ui.painter();
            painter.rect_filled(
                scaled,
                4.0 * scale,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 17),
            );
            painter.rect_stroke(
                scaled,
                4.0 * scale,
                egui::Stroke::new(2.0 * scale, forza_pink()),
                egui::StrokeKind::Inside,
            );

            for (index, row) in rows.iter().enumerate() {
                let row_rect = egui::Rect::from_min_size(
                    base.min + egui::vec2(5.0, 6.0 + index as f32 * 42.0),
                    egui::vec2(280.0, 40.0),
                );
                let row_rect = egui::Rect::from_center_size(
                    scaled.center() + (row_rect.center() - base.center()) * scale,
                    row_rect.size() * scale,
                );
                let is_selected = selected == index;
                painter.rect_filled(
                    row_rect,
                    3.0 * scale,
                    if is_selected {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_black_alpha(221)
                    },
                );
                let color = if is_selected {
                    egui::Color32::BLACK
                } else if row.enabled {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_white_alpha(102)
                };
                painter.text(
                    row_rect.left_center() + egui::vec2(22.5 * scale, 0.0),
                    egui::Align2::CENTER_CENTER,
                    row.icon,
                    egui::FontId::proportional(18.0 * scale),
                    color,
                );
                painter.text(
                    row_rect.left_center() + egui::vec2(50.0 * scale, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &row.text,
                    egui::FontId::proportional(14.0 * scale),
                    color,
                );
            }
        });
}

fn render_vehicle_card(
    context: &egui::Context,
    car: Option<&CarInfo>,
    life: &LifeSnapshot,
    num_cylinders: i32,
    opacity: f32,
    screen_size: [f32; 2],
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

    let target = vehicle_card_rect(screen_size);
    egui::Area::new(egui::Id::new("forzalife-vehicle-card"))
        .fixed_pos(target.min)
        .show(context, |ui| {
            ui.set_opacity(opacity);
            let (outer, _) = ui.allocate_exact_size(target.size(), egui::Sense::hover());
            let painter = ui.painter();
            painter.add(
                egui::epaint::Shadow {
                    offset: [0, 0],
                    blur: 10,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(179),
                }
                .as_shape(outer, 10),
            );
            painter.rect_filled(
                outer,
                10.0,
                egui::Color32::from_rgba_unmultiplied(17, 17, 17, 153),
            );
            let border = outer.shrink(4.0);
            painter.rect_stroke(
                border,
                7.0,
                egui::Stroke::new(2.0_f32, forza_pink()),
                egui::StrokeKind::Inside,
            );
            let content = border.shrink2(egui::vec2(14.0, 14.0));
            let header_left = content.left() + 6.0;
            painter.text(
                egui::pos2(header_left, content.top()),
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::proportional(22.0),
                egui::Color32::WHITE,
            );
            painter.text(
                egui::pos2(header_left, content.top() + 29.0),
                egui::Align2::LEFT_TOP,
                "Vehicle Info Card",
                egui::FontId::proportional(12.0),
                forza_pink(),
            );
            let separator_y = content.top() + 52.0;
            painter.line_segment(
                [
                    egui::pos2(header_left, separator_y),
                    egui::pos2(content.right() - 6.0, separator_y),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(51)),
            );

            let grid = egui::Rect::from_min_max(
                egui::pos2(content.left(), separator_y + 10.0),
                egui::pos2(content.right(), content.bottom() - 24.0),
            );
            let cell_size = egui::vec2(grid.width() / 3.0, grid.height() / 2.0);
            for (index, (label, value, color)) in items.iter().enumerate() {
                let column = index % 3;
                let row = index / 3;
                let cell = egui::Rect::from_min_size(
                    grid.min + egui::vec2(column as f32 * cell_size.x, row as f32 * cell_size.y),
                    cell_size,
                );
                let card = cell.shrink2(egui::vec2(6.0, 4.0));
                painter.rect_filled(
                    card,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(17, 17, 17, 221),
                );
                painter.text(
                    card.min + egui::vec2(8.0, 8.0),
                    egui::Align2::LEFT_TOP,
                    *label,
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_white_alpha(119),
                );
                painter.text(
                    card.min + egui::vec2(8.0, 25.0),
                    egui::Align2::LEFT_TOP,
                    value,
                    egui::FontId::proportional(18.0),
                    *color,
                );
            }
            painter.text(
                egui::pos2(content.center().x, content.bottom()),
                egui::Align2::CENTER_BOTTOM,
                "Press [LEFT] to go back",
                egui::FontId::proportional(12.0),
                egui::Color32::from_white_alpha(153),
            );
        });
}

fn render_odometer_input(
    context: &egui::Context,
    current_odometer_m: f32,
    input: &str,
    error: Option<&str>,
    screen_size: [f32; 2],
) {
    render_number_input(
        context,
        "forzalife-odometer-input",
        "SET ODOMETER",
        &format!("Current: {:.1} km", current_odometer_m / 1_000.0),
        "Type dashboard mileage",
        "KM",
        input,
        error,
        screen_size,
    );
}

fn render_fuel_input(
    context: &egui::Context,
    current_fuel_percent: f32,
    input: &str,
    error: Option<&str>,
    screen_size: [f32; 2],
) {
    render_number_input(
        context,
        "forzalife-fuel-input",
        "SET FUEL FOR TESTING",
        &format!(
            "Current: {:.0}%",
            current_fuel_percent.clamp(0.0, 1.0) * 100.0
        ),
        "Type fuel percentage",
        "%",
        input,
        error,
        screen_size,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_number_input(
    context: &egui::Context,
    id: &'static str,
    title: &str,
    current: &str,
    placeholder: &str,
    unit: &str,
    input: &str,
    error: Option<&str>,
    screen_size: [f32; 2],
) {
    let size = egui::vec2(520.0, 210.0);
    let position = egui::pos2(
        (screen_size[0] - size.x) * 0.5,
        (screen_size[1] - size.y) * 0.5,
    );
    egui::Area::new(egui::Id::new(id))
        .fixed_pos(position)
        .show(context, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                12.0,
                egui::Color32::from_rgba_unmultiplied(17, 17, 17, 230),
            );
            painter.rect_stroke(
                rect.shrink(5.0),
                8.0,
                egui::Stroke::new(2.5_f32, forza_pink()),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.min + egui::vec2(28.0, 26.0),
                egui::Align2::LEFT_TOP,
                title,
                forza_font(26.0, true),
                egui::Color32::WHITE,
            );
            painter.text(
                rect.min + egui::vec2(28.0, 64.0),
                egui::Align2::LEFT_TOP,
                current,
                forza_font(15.0, false),
                egui::Color32::from_white_alpha(160),
            );
            let input_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(28.0, 96.0),
                egui::vec2(464.0, 48.0),
            );
            painter.rect_filled(input_rect, 5.0, egui::Color32::from_black_alpha(221));
            painter.text(
                input_rect.left_center() + egui::vec2(16.0, 0.0),
                egui::Align2::LEFT_CENTER,
                if input.is_empty() { placeholder } else { input },
                forza_font(21.0, true),
                if input.is_empty() {
                    egui::Color32::from_white_alpha(90)
                } else {
                    egui::Color32::WHITE
                },
            );
            painter.text(
                input_rect.right_center() - egui::vec2(16.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                unit,
                forza_font(15.0, true),
                forza_pink(),
            );
            painter.text(
                rect.min + egui::vec2(28.0, 166.0),
                egui::Align2::LEFT_TOP,
                error.unwrap_or("Right or Enter to save   •   Left or Esc to cancel"),
                forza_font(14.0, false),
                if error.is_some() {
                    egui::Color32::from_rgb(221, 0, 0)
                } else {
                    egui::Color32::from_white_alpha(150)
                },
            );
        });
}

fn vehicle_card_rect(screen_size: [f32; 2]) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(24.0, (screen_size[1] - VEHICLE_CARD_SIZE[1]) * 0.5),
        egui::vec2(VEHICLE_CARD_SIZE[0], VEHICLE_CARD_SIZE[1]),
    )
}

fn render_intro(context: &egui::Context, packets: u64, port: u16) {
    egui::Area::new(egui::Id::new("forzalife-intro"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-30.0, -60.0))
        .show(context, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(204))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("⌛").size(15.0).color(if packets == 0 {
                            forza_green()
                        } else {
                            forza_pink()
                        }));
                        ui.label(
                            egui::RichText::new(if packets == 0 {
                                format!("Waiting for first telemetry on port {port}")
                            } else {
                                "ForzaLife is waiting for Horizon 6".to_owned()
                            })
                            .font(forza_font(15.0, false))
                            .color(egui::Color32::WHITE),
                        );
                    });
                });
        });
}

fn render_drive_hud(
    context: &egui::Context,
    drive: &DriveSnapshot,
    session: &DriveSession,
    screen_size: [f32; 2],
    mode: HudMode,
) {
    if mode == HudMode::Life {
        return;
    }
    let scale = overlay_scale(screen_size);
    let compact = mode == HudMode::Minimal;
    let size = egui::vec2(
        if compact { 230.0 } else { 440.0 },
        if compact { 82.0 } else { 158.0 },
    ) * scale;
    egui::Area::new(egui::Id::new("forzalife-drive-hud"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-24.0, 24.0) * scale)
        .show(context, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                8.0 * scale,
                egui::Color32::from_rgba_unmultiplied(17, 17, 17, 215),
            );
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 4.0 * scale)),
                0.0,
                forza_pink(),
            );
            painter.text(
                rect.min + egui::vec2(18.0, 14.0) * scale,
                egui::Align2::LEFT_TOP,
                format!("{:.0} km/h", drive.speed_kmh),
                forza_font(if compact { 30.0 } else { 42.0 } * scale, true),
                egui::Color32::WHITE,
            );
            painter.text(
                rect.min + egui::vec2(if compact { 164.0 } else { 350.0 }, 18.0) * scale,
                egui::Align2::LEFT_TOP,
                drive.gear.label(),
                forza_font(if compact { 30.0 } else { 42.0 } * scale, true),
                forza_pink(),
            );
            if compact {
                return;
            }
            painter.text(
                rect.min + egui::vec2(20.0, 68.0) * scale,
                egui::Align2::LEFT_TOP,
                format!("{:.0} mph   {} RPM", drive.speed_mph, drive.rpm),
                forza_font(15.0 * scale, false),
                egui::Color32::from_white_alpha(190),
            );
            let bar = egui::Rect::from_min_size(
                rect.min + egui::vec2(20.0, 96.0) * scale,
                egui::vec2(400.0, 12.0) * scale,
            );
            painter.rect_filled(bar, 3.0 * scale, egui::Color32::from_white_alpha(35));
            let fill_color = match drive.shift_stage() {
                3 => egui::Color32::RED,
                2 => egui::Color32::YELLOW,
                1 => forza_pink(),
                _ => egui::Color32::from_rgb(70, 190, 120),
            };
            painter.rect_filled(
                egui::Rect::from_min_max(
                    bar.min,
                    egui::pos2(bar.left() + bar.width() * drive.rpm_percent, bar.bottom()),
                ),
                3.0 * scale,
                fill_color,
            );
            painter.text(
                rect.min + egui::vec2(20.0, 122.0) * scale,
                egui::Align2::LEFT_TOP,
                format!(
                    "THROTTLE {:>3}%   {}{}   {:.0}s  {:.1} km  {:.0} avg/{:.0} max  {:.1} L  {} refuels",
                    drive.throttle_percent,
                    if drive.race_on { "RACE" } else { "FREE ROAM" },
                    drive
                        .race_position
                        .map(|position| format!("  P{position}"))
                        .unwrap_or_default(),
                    session.elapsed_s,
                    session.distance_m / 1_000.0,
                    session.average_speed_mps() * 3.6,
                    session.max_speed_mps * 3.6,
                    session.fuel_used_liters,
                    session.refuels,
                ),
                forza_font(12.0 * scale, true),
                egui::Color32::from_white_alpha(175),
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn render_race_hud(
    context: &egui::Context,
    telemetry: &Telemetry,
    life: Option<&LifeSnapshot>,
    boost: &BoostGaugeState,
    boost_background: &egui::TextureHandle,
    hud_icons: &HudIcons,
    screen_size: [f32; 2],
    hud_mode: HudMode,
) {
    if hud_mode != HudMode::Life {
        return;
    }
    let scale = overlay_scale(screen_size);
    let target = hud_rect(screen_size);
    egui::Area::new(egui::Id::new("forzalife-race-hud"))
        .fixed_pos(target.min)
        .show(context, |ui| {
            let (hud, _) = ui.allocate_exact_size(target.size(), egui::Sense::hover());
            let painter = ui.painter();
            if boost.visible {
                boost_gauge(
                    painter,
                    egui::Rect::from_min_size(
                        hud.min + egui::vec2(42.0, -55.0) * scale,
                        egui::vec2(104.0, 104.0) * scale,
                    ),
                    boost,
                    boost_background,
                );
            }
            if let Some(life) = life {
                let odometer_right = hud.left() + 165.0 * scale;
                let odometer_bottom = hud.bottom() - scale;
                painter.text(
                    egui::pos2(odometer_right - 28.0 * scale, odometer_bottom),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{:.0}", life.odometer_m / 1_000.0),
                    forza_font(23.0 * scale, true),
                    egui::Color32::from_white_alpha(128),
                );
                painter.text(
                    egui::pos2(odometer_right, odometer_bottom - scale),
                    egui::Align2::RIGHT_BOTTOM,
                    "KM",
                    forza_font(14.0 * scale, true),
                    egui::Color32::from_white_alpha(128),
                );
                if let (Some(mpg), Some(km_per_liter)) =
                    (life.average_mpg, life.average_km_per_liter)
                {
                    painter.text(
                        egui::pos2(hud.left() + 178.0 * scale, odometer_bottom),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{:.1} MPG  {:.1} KM/L", mpg, km_per_liter),
                        forza_font(11.0 * scale, true),
                        egui::Color32::from_white_alpha(128),
                    );
                }
                if !life.is_usage_paused {
                    fuel_and_oil(
                        painter,
                        hud,
                        life,
                        telemetry.num_cylinders == 0,
                        hud_icons,
                        scale,
                    );
                }
            }
        });
}

fn hud_rect(screen_size: [f32; 2]) -> egui::Rect {
    let scale = overlay_scale(screen_size);
    let size = egui::vec2(HUD_SIZE[0], HUD_SIZE[1]) * scale;
    egui::Rect::from_min_size(
        egui::pos2(
            (screen_size[0] - size.x - 2.0 * scale).max(0.0),
            (screen_size[1] - size.y - 2.0 * scale).max(0.0),
        ),
        size,
    )
}

fn overlay_scale(screen_size: [f32; 2]) -> f32 {
    (screen_size[0] / 1_920.0)
        .min(screen_size[1] / 1_080.0)
        .max(0.1)
}

fn forza_font(size: f32, semibold: bool) -> egui::FontId {
    egui::FontId::new(
        size,
        egui::FontFamily::Name(
            if semibold {
                ROBOTO_SEMIBOLD
            } else {
                ROBOTO_MEDIUM
            }
            .into(),
        ),
    )
}

fn boost_gauge(
    painter: &egui::Painter,
    rect: egui::Rect,
    boost: &BoostGaugeState,
    background: &egui::TextureHandle,
) {
    let scale = rect.width() / 104.0;
    let min_display = boost.min_display as f32;
    let max_display = boost.max_display as f32;
    let angle = boost_needle_angle(boost.current_boost, min_display, max_display);
    let center = rect.center();
    painter.image(
        background.id(),
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    gauge_arc(
        painter,
        center,
        48.0 * scale,
        -135.0,
        -5.0,
        egui::Color32::from_white_alpha(68),
        scale,
    );
    gauge_arc(
        painter,
        center,
        48.0 * scale,
        -5.0,
        35.0,
        forza_pink(),
        scale,
    );

    let radians = (angle - 90.0).to_radians();
    let direction = egui::vec2(radians.cos(), radians.sin());
    let perpendicular = egui::vec2(-direction.y, direction.x);
    let base_left = center - perpendicular * 2.68 * scale;
    let base_right = center + perpendicular * 2.68 * scale;
    let tip_center = center + direction * 48.42 * scale;
    let tip_left = tip_center - perpendicular * 1.68 * scale;
    let tip_right = tip_center + perpendicular * 1.68 * scale;
    let mut needle = egui::Mesh::default();
    let base_color = egui::Color32::TRANSPARENT;
    let tip_color = egui::Color32::WHITE;
    needle.colored_vertex(base_left, base_color);
    needle.colored_vertex(base_right, base_color);
    needle.colored_vertex(tip_right, tip_color);
    needle.colored_vertex(tip_left, tip_color);
    needle.add_triangle(0, 1, 2);
    needle.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(needle));
    let readout = egui::Rect::from_center_size(
        rect.center() + egui::vec2(8.0, 3.0) * scale,
        egui::vec2(42.0, 17.0) * scale,
    );
    painter.rect_filled(readout, 2.0 * scale, egui::Color32::from_black_alpha(218));
    painter.rect_stroke(
        readout,
        2.0 * scale,
        egui::Stroke::new(1.0 * scale, egui::Color32::from_white_alpha(28)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        readout.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.1}", boost.current_boost),
        forza_font(12.0 * scale, true),
        forza_pink(),
    );
    painter.text(
        rect.left_bottom() + egui::vec2(23.0, -20.0) * scale,
        egui::Align2::LEFT_BOTTOM,
        format!("{min_display:.0}"),
        forza_font(14.0 * scale, true),
        egui::Color32::from_white_alpha(68),
    );
    painter.text(
        rect.right_top() + egui::vec2(-31.0, 13.0) * scale,
        egui::Align2::RIGHT_TOP,
        format!("{max_display:.0}"),
        forza_font(14.0 * scale, true),
        forza_pink(),
    );
    painter.text(
        rect.left_top() + egui::vec2(13.0, 36.0) * scale,
        egui::Align2::LEFT_TOP,
        "0",
        forza_font(14.0 * scale, true),
        egui::Color32::from_white_alpha(68),
    );
    painter.text(
        rect.right_bottom() + egui::vec2(-10.0, -8.0) * scale,
        egui::Align2::RIGHT_BOTTOM,
        "PSI",
        forza_font(9.0 * scale, true),
        egui::Color32::from_white_alpha(90),
    );
}

fn gauge_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    from: f32,
    to: f32,
    color: egui::Color32,
    scale: f32,
) {
    let points = (0..=32)
        .map(|step| {
            let angle = from + (to - from) * step as f32 / 32.0;
            let radians = (angle - 90.0).to_radians();
            center + egui::vec2(radians.cos(), radians.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(3.2 * scale, color),
    ));
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

fn fuel_and_oil(
    painter: &egui::Painter,
    hud: egui::Rect,
    life: &LifeSnapshot,
    is_electric: bool,
    icons: &HudIcons,
    scale: f32,
) {
    let fuel = life.fuel_percent.clamp(0.0, 1.0);
    let fuel_color = if fuel <= 0.0 {
        egui::Color32::from_rgb(221, 0, 0)
    } else if fuel <= 0.2 {
        warning_color()
    } else {
        egui::Color32::from_white_alpha(128)
    };
    let icon = if is_electric {
        &icons.battery
    } else {
        &icons.fuel
    };
    painter.image(
        icon.id(),
        egui::Rect::from_min_size(
            hud.min + egui::vec2(337.0, -2.0) * scale,
            egui::vec2(27.0, 27.0) * scale,
        ),
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        fuel_color,
    );
    painter.line(
        vec![
            hud.min + egui::vec2(381.0, 88.0) * scale,
            hud.min + egui::vec2(371.0, 88.0) * scale,
            hud.min + egui::vec2(371.0, 1.0) * scale,
            hud.min + egui::vec2(381.0, 1.0) * scale,
            hud.min + egui::vec2(381.0, 88.0) * scale,
        ],
        egui::Stroke::new(
            2.0 * scale,
            egui::Color32::from_rgba_unmultiplied(128, 128, 128, 221),
        ),
    );
    let bar = egui::Rect::from_min_size(
        hud.min + egui::vec2(373.0, 3.0) * scale,
        egui::vec2(6.0, 82.0) * scale,
    );
    painter.rect_filled(
        bar,
        0.0,
        egui::Color32::from_rgba_unmultiplied(128, 128, 128, 68),
    );
    let fill = egui::Rect::from_min_max(
        egui::pos2(bar.left(), bar.bottom() - bar.height() * fuel),
        bar.right_bottom(),
    );
    painter.rect_filled(fill, 0.0, fuel_color);
    if life.oil_remaining_m < 0.0 && !is_electric {
        let oil_color = if life.oil_remaining_m < -250_000.0 {
            egui::Color32::from_rgb(221, 0, 0)
        } else {
            warning_color()
        };
        painter.image(
            icons.oil.id(),
            egui::Rect::from_min_size(
                hud.min + egui::vec2(84.0, 216.0) * scale,
                egui::vec2(48.0, 48.0) * scale,
            ),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            oil_color,
        );
    }
}

fn render_refueling(
    context: &egui::Context,
    life: &LifeSnapshot,
    refueling: bool,
    screen_size: [f32; 2],
) {
    let scale = overlay_scale(screen_size);
    let size = egui::vec2(360.0, 78.0) * scale;
    let position = egui::pos2((screen_size[0] - size.x) * 0.5, 54.0 * scale);
    egui::Area::new(egui::Id::new("forzalife-refueling"))
        .fixed_pos(position)
        .show(context, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                5.0 * scale,
                egui::Color32::from_rgba_unmultiplied(17, 17, 17, 220),
            );
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 4.0 * scale)),
                0.0,
                forza_pink(),
            );
            painter.text(
                rect.min + egui::vec2(18.0, 16.0) * scale,
                egui::Align2::LEFT_TOP,
                if refueling {
                    "REFUELING"
                } else {
                    "HOLD ENTER / B TO REFUEL"
                },
                forza_font(19.0 * scale, true),
                egui::Color32::WHITE,
            );
            painter.text(
                rect.right_top() + egui::vec2(-18.0, 18.0) * scale,
                egui::Align2::RIGHT_TOP,
                format!("{:.0}%", life.fuel_percent.clamp(0.0, 1.0) * 100.0),
                forza_font(15.0 * scale, true),
                forza_pink(),
            );
            let track = egui::Rect::from_min_size(
                rect.min + egui::vec2(18.0, 52.0) * scale,
                egui::vec2(324.0, 8.0) * scale,
            );
            painter.rect_filled(track, 1.0 * scale, egui::Color32::from_white_alpha(28));
            painter.rect_filled(
                egui::Rect::from_min_max(
                    track.min,
                    egui::pos2(
                        track.left() + track.width() * life.fuel_percent.clamp(0.0, 1.0),
                        track.bottom(),
                    ),
                ),
                1.0 * scale,
                forza_pink(),
            );
        });
}

fn refuel_liters(at_station: bool, held: bool, elapsed_s: f32) -> f32 {
    if at_station && held && (0.0..=1.0).contains(&elapsed_s) {
        REFUEL_LITERS_PER_SECOND * elapsed_s
    } else {
        0.0
    }
}

fn render_navigation_hud(
    context: &egui::Context,
    telemetry: &Telemetry,
    locations: &Locations,
    target: LocationKind,
    icons: &HudIcons,
    screen_size: [f32; 2],
) {
    let Some((location, distance)) = locations.nearest(target, telemetry.position) else {
        return;
    };
    let scale = overlay_scale(screen_size);
    egui::Area::new(egui::Id::new("forzalife-navigation"))
        .fixed_pos([240.0 * scale, (screen_size[1] - 358.0 * scale).max(0.0)])
        .show(context, |ui| {
            let icon = match target {
                LocationKind::Gas => Some(&icons.fuel),
                LocationKind::Workshop => Some(&icons.workshop),
                LocationKind::ConvenienceStore => None,
            };
            navigation(ui, telemetry, location.position, distance, icon, scale);
        });
}

fn spawn_receiver(
    port: u16,
    latest: Arc<RwLock<Latest>>,
    locations: Arc<Locations>,
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
        let mut last_refuel_sample: Option<(i32, u32)> = None;
        let mut fuel_starvation = FuelStarvation::default();
        let input_proxy_disabled = env::var_os("FORZALIFE_DISABLE_INPUT_PROXY").is_some();
        let mut input_proxy = if input_proxy_disabled {
            None
        } else {
            match InputProxy::start() {
                Ok(proxy) => Some(proxy),
                Err(error) => {
                    eprintln!("cannot initialize throttle proxy: {error}");
                    None
                }
            }
        };
        let mut last_input_proxy_attempt = Instant::now();
        let mut packet = [0_u8; PACKET_SIZE + 1];
        loop {
            match socket.recv(&mut packet) {
                Ok(size) => match parse(&packet[..size]) {
                    Ok(telemetry) => {
                        if !input_proxy_disabled
                            && input_proxy.is_none()
                            && last_input_proxy_attempt.elapsed() >= INPUT_PROXY_RETRY_INTERVAL
                        {
                            last_input_proxy_attempt = Instant::now();
                            match InputProxy::start() {
                                Ok(proxy) => {
                                    proxy.set_restricted(fuel_starvation.enabled());
                                    input_proxy = Some(proxy);
                                    eprintln!("input proxy initialized");
                                }
                                Err(error) => {
                                    eprintln!("cannot initialize throttle proxy: {error}");
                                }
                            }
                        }
                        let car = cars.get(telemetry.car_ordinal);
                        let tank_liters = car.map_or_else(
                            || CarInfo::fallback_tank_capacity_liters(telemetry.num_cylinders),
                            |car| car.tank_capacity_liters(telemetry.num_cylinders),
                        );
                        let model_year = car.map_or(2020, |car| car.year);
                        let mut simulation = simulation.write().unwrap();
                        let mut life =
                            simulation.update_with_vehicle(&telemetry, tank_liters, model_year);
                        let mut refuel_available = false;
                        let mut refueling = false;
                        if telemetry.speed_mps.abs() < 1.0 {
                            let at_gas = locations
                                .nearest(LocationKind::Gas, telemetry.position)
                                .is_some_and(|(_, distance)| distance <= 25.0);
                            refuel_available = at_gas && life.fuel_percent < 1.0 - f32::EPSILON;
                            let refuel_held =
                                input_proxy.as_ref().is_some_and(InputProxy::refuel_pressed);
                            if refuel_available && refuel_held {
                                let elapsed_s = last_refuel_sample
                                    .filter(|(car, _)| *car == telemetry.car_ordinal)
                                    .map(|(_, timestamp)| {
                                        telemetry.timestamp_ms.wrapping_sub(timestamp) as f32
                                            / 1_000.0
                                    })
                                    .filter(|elapsed| (0.0..=1.0).contains(elapsed))
                                    .unwrap_or_default();
                                simulation.refuel(
                                    telemetry.car_ordinal,
                                    refuel_liters(at_gas, refuel_held, elapsed_s),
                                );
                                life = simulation
                                    .current(telemetry.car_ordinal)
                                    .expect("updated vehicle state");
                                refueling = life.fuel_percent < 1.0 - f32::EPSILON;
                                refuel_available = refueling;
                                last_refuel_sample =
                                    Some((telemetry.car_ordinal, telemetry.timestamp_ms));
                            } else {
                                last_refuel_sample = None;
                            }
                            if locations
                                .nearest(LocationKind::Workshop, telemetry.position)
                                .is_some_and(|(_, distance)| distance <= 25.0)
                            {
                                simulation.service_oil(telemetry.car_ordinal);
                                life = simulation.update_with_vehicle(
                                    &telemetry,
                                    tank_liters,
                                    model_year,
                                );
                            }
                        } else {
                            last_refuel_sample = None;
                        }
                        if let Some(action) = fuel_starvation.update(&life, &telemetry)
                            && let Some(proxy) = &input_proxy
                        {
                            proxy.set_restricted(action == StarvationAction::Enable);
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
                        state.refuel_available = refuel_available;
                        state.refueling = refueling;
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

fn navigation(
    ui: &mut egui::Ui,
    telemetry: &Telemetry,
    target_position: [f32; 3],
    distance: f32,
    target_icon: Option<&egui::TextureHandle>,
    scale: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(66.0, 67.0) * scale, egui::Sense::hover());
    let painter = ui.painter();
    let arrow_rect = egui::Rect::from_min_size(rect.min, egui::vec2(58.0, 58.0) * scale);
    let center = arrow_rect.center();
    painter.circle_stroke(
        center,
        27.0 * scale,
        egui::Stroke::new(4.0 * scale, egui::Color32::from_white_alpha(17)),
    );
    let angle = (target_position[0] - telemetry.position[0])
        .atan2(target_position[2] - telemetry.position[2])
        - telemetry.yaw;
    let direction = egui::vec2(angle.sin(), -angle.cos());
    let right = egui::vec2(-direction.y, direction.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + direction * 15.0 * scale,
            center - direction * 10.0 * scale + right * 10.0 * scale,
            center - direction * 6.5 * scale,
            center - direction * 10.0 * scale - right * 9.0 * scale,
        ],
        egui::Color32::WHITE,
        egui::Stroke::new(2.0 * scale, egui::Color32::from_rgb(29, 29, 27)),
    ));
    let (value, unit) = if distance >= 1_000.0 {
        (format!("{:.1}", distance / 1_000.0), " KM")
    } else {
        (format!("{distance:.0}"), " M")
    };
    painter.text(
        rect.min + egui::vec2(29.0, -16.0) * scale,
        egui::Align2::CENTER_TOP,
        format!("{value}{unit}"),
        forza_font(20.0 * scale, true),
        egui::Color32::WHITE,
    );
    if let Some(icon) = target_icon {
        painter.image(
            icon.id(),
            egui::Rect::from_min_size(
                rect.min + egui::vec2(40.0, 41.0) * scale,
                egui::vec2(26.0, 26.0) * scale,
            ),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::from_white_alpha(128),
        );
    }
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
    const BACKSPACE_KEYSYM: u32 = 0xff08;
    const RETURN_KEYSYM: u32 = 0xff0d;
    const ESCAPE_KEYSYM: u32 = 0xff1b;
    const LEFT_KEYSYM: u32 = 0xff51;
    const UP_KEYSYM: u32 = 0xff52;
    const RIGHT_KEYSYM: u32 = 0xff53;
    const DOWN_KEYSYM: u32 = 0xff54;
    let (connection, screen) = loop {
        match RustConnection::connect(None) {
            Ok(connection) => break connection,
            Err(error) => {
                eprintln!("waiting for the gamescope input display: {error}");
                thread::sleep(Duration::from_millis(250));
            }
        }
    };
    let root = connection.setup().roots[screen].root;
    let setup = connection.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let mapping = connection.get_keyboard_mapping(first, count)?.reply()?;
    let keycode = |symbol: u32| {
        mapping
            .keysyms
            .chunks(usize::from(mapping.keysyms_per_keycode))
            .position(|symbols| symbols.contains(&symbol))
            .map(|index| first + index as u8)
    };
    let mut bindings = vec![
        (
            keycode(RIGHT_KEYSYM).ok_or("Right arrow key was not found in the X11 keymap")?,
            InputEvent::Primary,
        ),
        (
            keycode(DOWN_KEYSYM).ok_or("Down arrow key was not found in the X11 keymap")?,
            InputEvent::Down,
        ),
        (
            keycode(UP_KEYSYM).ok_or("Up arrow key was not found in the X11 keymap")?,
            InputEvent::Up,
        ),
        (
            keycode(LEFT_KEYSYM).ok_or("Left arrow key was not found in the X11 keymap")?,
            InputEvent::Back,
        ),
        (
            keycode(BACKSPACE_KEYSYM).ok_or("Backspace key was not found in the X11 keymap")?,
            InputEvent::Backspace,
        ),
        (
            keycode(RETURN_KEYSYM).ok_or("Enter key was not found in the X11 keymap")?,
            InputEvent::Confirm,
        ),
        (
            keycode(ESCAPE_KEYSYM).ok_or("Escape key was not found in the X11 keymap")?,
            InputEvent::Cancel,
        ),
        (
            keycode(u32::from(b'.')).ok_or("period key was not found in the X11 keymap")?,
            InputEvent::Decimal,
        ),
    ];
    for digit in b'0'..=b'9' {
        bindings.push((
            keycode(u32::from(digit)).ok_or("digit key was not found in the X11 keymap")?,
            InputEvent::Digit(char::from(digit)),
        ));
    }
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

#[cfg(test)]
mod tests {
    use super::{
        BoostGaugeState, boost_needle_angle, hud_rect, menu_rect, overlay_scale, vehicle_card_rect,
    };
    use egui;

    #[test]
    fn boost_needle_matches_the_windows_gauge_endpoints() {
        assert_eq!(boost_needle_angle(0.0, -15.0, 15.0), -79.0);
        assert_eq!(boost_needle_angle(-15.0, -15.0, 15.0), -135.0);
        assert_eq!(boost_needle_angle(15.0, -15.0, 15.0), 35.0);
    }

    #[test]
    fn boost_scale_persists_until_the_car_changes() {
        let mut boost = BoostGaugeState::default();
        boost.update_value(1, 22.0);
        boost.update_value(1, 0.0);
        assert_eq!(boost.max_display, 25);

        boost.update_value(2, 0.0);
        assert_eq!(boost.max_display, 15);
        assert!(!boost.visible);
    }

    #[test]
    fn windows_layout_uses_the_original_1080p_anchors() {
        assert_eq!(
            menu_rect([1920.0, 1080.0]),
            egui::Rect::from_min_size(egui::pos2(20.0, 266.0), egui::vec2(290.0, 384.0),)
        );
        assert_eq!(
            vehicle_card_rect([1920.0, 1080.0]),
            egui::Rect::from_min_size(egui::pos2(24.0, 370.0), egui::vec2(540.0, 340.0),)
        );
        assert_eq!(
            hud_rect([1920.0, 1080.0]),
            egui::Rect::from_min_size(egui::pos2(1518.0, 787.0), egui::vec2(400.0, 291.0),)
        );
    }

    #[test]
    fn race_hud_uses_the_windows_resolution_scale() {
        assert_eq!(overlay_scale([3840.0, 2160.0]), 2.0);
        assert_eq!(
            hud_rect([3840.0, 2160.0]),
            egui::Rect::from_min_size(egui::pos2(3036.0, 1574.0), egui::vec2(800.0, 582.0),)
        );
    }

    #[test]
    fn refueling_requires_the_button_to_be_held_and_uses_the_slower_rate() {
        assert_eq!(super::refuel_liters(false, true, 1.0), 0.0);
        assert_eq!(super::refuel_liters(true, false, 1.0), 0.0);
        assert_eq!(super::refuel_liters(true, true, 1.0), 2.25);
    }
}
