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
const REFUEL_LITERS_PER_SECOND: f32 = 0.75;
const MENU_SIZE: [f32; 2] = [290.0, 220.0];
const VEHICLE_CARD_SIZE: [f32; 2] = [750.0, 480.0];
const HUD_SIZE: [f32; 2] = [400.0, 291.0];
const ROBOTO_MEDIUM: &str = "Roboto Condensed Medium";
const ROBOTO_SEMIBOLD: &str = "Roboto Condensed Semibold";

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
            install_forza_fonts(&context.egui_ctx);
            context.egui_ctx.set_visuals(egui::Visuals {
                panel_fill: egui::Color32::TRANSPARENT,
                window_fill: egui::Color32::TRANSPARENT,
                ..egui::Visuals::dark()
            });
            let boost_background = load_boost_background(&context.egui_ctx);
            Ok(Box::new(OverlayApp {
                latest,
                locations,
                cars,
                simulation,
                input,
                menu: MenuState::default(),
                last_menu_page: MenuPage::Main,
                last_menu_activity: Instant::now(),
                navigation_target: None,
                classified: false,
                classification_error_reported: false,
                boost: BoostGaugeState::default(),
                boost_background,
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
    last_menu_page: MenuPage,
    last_menu_activity: Instant,
    navigation_target: Option<LocationKind>,
    classified: bool,
    classification_error_reported: bool,
    boost: BoostGaugeState,
    boost_background: egui::TextureHandle,
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
        let (telemetry, mut life, fresh, packets) = {
            let latest = self.latest.read().unwrap();
            (
                latest.telemetry.clone(),
                latest.life.clone(),
                latest
                    .received_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(2)),
                latest.packets,
            )
        };

        let active_telemetry = telemetry
            .as_ref()
            .filter(|telemetry| fresh && telemetry.car_ordinal > 0);
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
            render_race_hud(
                context,
                data,
                life.as_ref(),
                &self.boost,
                &self.boost_background,
                self.screen_size,
            );
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
                _ => main_menu_rows(life.as_ref().is_some_and(|life| life.is_usage_paused)),
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
    let image = image::load_from_memory(include_bytes!("../assets/boost_bg.png"))
        .expect("bundled boost gauge background")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    context.load_texture(
        "forzalife-boost-background",
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    )
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

            for (index, row) in rows.iter().take(5).enumerate() {
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
                    blur: 15,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(179),
                }
                .as_shape(outer, 16),
            );
            painter.rect_filled(
                outer,
                16.0,
                egui::Color32::from_rgba_unmultiplied(17, 17, 17, 153),
            );
            let border = outer.shrink(5.0);
            painter.rect_stroke(
                border,
                10.0,
                egui::Stroke::new(2.5_f32, forza_pink()),
                egui::StrokeKind::Inside,
            );
            let content = border.shrink2(egui::vec2(22.5, 24.5));
            let header_left = content.left() + 10.0;
            painter.text(
                egui::pos2(header_left, content.top()),
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::proportional(28.0),
                egui::Color32::WHITE,
            );
            painter.text(
                egui::pos2(header_left, content.top() + 36.0),
                egui::Align2::LEFT_TOP,
                "Vehicle Info Card",
                egui::FontId::proportional(14.0),
                forza_pink(),
            );
            let separator_y = content.top() + 67.0;
            painter.line_segment(
                [
                    egui::pos2(header_left, separator_y),
                    egui::pos2(content.right() - 10.0, separator_y),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(51)),
            );

            let grid = egui::Rect::from_min_max(
                egui::pos2(content.left(), separator_y + 13.0),
                egui::pos2(content.right(), content.bottom() - 36.0),
            );
            let cell_size = egui::vec2(grid.width() / 3.0, grid.height() / 2.0);
            for (index, (label, value, color)) in items.iter().enumerate() {
                let column = index % 3;
                let row = index / 3;
                let cell = egui::Rect::from_min_size(
                    grid.min + egui::vec2(column as f32 * cell_size.x, row as f32 * cell_size.y),
                    cell_size,
                );
                let card = cell.shrink2(egui::vec2(10.0, 5.0));
                painter.rect_filled(
                    card,
                    6.0,
                    egui::Color32::from_rgba_unmultiplied(17, 17, 17, 221),
                );
                painter.text(
                    card.min + egui::vec2(20.0, 14.0),
                    egui::Align2::LEFT_TOP,
                    *label,
                    egui::FontId::proportional(16.0),
                    egui::Color32::from_white_alpha(119),
                );
                painter.text(
                    card.min + egui::vec2(20.0, 40.0),
                    egui::Align2::LEFT_TOP,
                    value,
                    egui::FontId::proportional(28.0),
                    *color,
                );
            }
            painter.text(
                egui::pos2(content.center().x, content.bottom()),
                egui::Align2::CENTER_BOTTOM,
                "Press [L] to close",
                egui::FontId::proportional(18.0),
                egui::Color32::from_white_alpha(153),
            );
        });
}

fn vehicle_card_rect(screen_size: [f32; 2]) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(screen_size[0] / 2.0, screen_size[1] / 2.0),
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

fn render_race_hud(
    context: &egui::Context,
    telemetry: &Telemetry,
    life: Option<&LifeSnapshot>,
    boost: &BoostGaugeState,
    boost_background: &egui::TextureHandle,
    screen_size: [f32; 2],
) {
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
                        hud.min + egui::vec2(20.0, -70.0),
                        egui::vec2(104.0, 104.0),
                    ),
                    boost,
                    boost_background,
                );
            }
            if let Some(life) = life {
                let odometer_right = hud.left() + 165.0;
                let odometer_bottom = hud.bottom() - 1.0;
                painter.text(
                    egui::pos2(odometer_right - 28.0, odometer_bottom),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{:.0}", life.odometer_m / 1_000.0),
                    forza_font(23.0, true),
                    egui::Color32::from_white_alpha(128),
                );
                painter.text(
                    egui::pos2(odometer_right, odometer_bottom - 1.0),
                    egui::Align2::RIGHT_BOTTOM,
                    "KM",
                    forza_font(14.0, true),
                    egui::Color32::from_white_alpha(128),
                );
                if !life.is_usage_paused {
                    fuel_and_oil(painter, hud, life, telemetry.num_cylinders == 0);
                }
            }
        });
}

fn hud_rect(screen_size: [f32; 2]) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(
            (screen_size[0] - HUD_SIZE[0] - 2.0).max(0.0),
            (screen_size[1] - HUD_SIZE[1] - 2.0).max(0.0),
        ),
        egui::vec2(HUD_SIZE[0], HUD_SIZE[1]),
    )
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
        48.0,
        -135.0,
        -5.0,
        egui::Color32::from_white_alpha(68),
    );
    gauge_arc(painter, center, 48.0, -5.0, 35.0, forza_pink());

    let radians = (angle - 90.0).to_radians();
    let direction = egui::vec2(radians.cos(), radians.sin());
    let perpendicular = egui::vec2(-direction.y, direction.x);
    let base_left = center - perpendicular * 2.68;
    let base_right = center + perpendicular * 2.68;
    let tip_center = center + direction * 48.42;
    let tip_left = tip_center - perpendicular * 1.68;
    let tip_right = tip_center + perpendicular * 1.68;
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
    painter.text(
        rect.left_bottom() + egui::vec2(23.0, -20.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{min_display:.0}"),
        forza_font(14.0, true),
        egui::Color32::from_white_alpha(68),
    );
    painter.text(
        rect.right_top() + egui::vec2(-31.0, 13.0),
        egui::Align2::RIGHT_TOP,
        format!("{max_display:.0}"),
        forza_font(14.0, true),
        forza_pink(),
    );
    painter.text(
        rect.left_top() + egui::vec2(13.0, 36.0),
        egui::Align2::LEFT_TOP,
        "0",
        forza_font(14.0, true),
        egui::Color32::from_white_alpha(68),
    );
}

fn gauge_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    from: f32,
    to: f32,
    color: egui::Color32,
) {
    let points = (0..=32)
        .map(|step| {
            let angle = from + (to - from) * step as f32 / 32.0;
            let radians = (angle - 90.0).to_radians();
            center + egui::vec2(radians.cos(), radians.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(3.2_f32, color)));
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

fn fuel_and_oil(painter: &egui::Painter, hud: egui::Rect, life: &LifeSnapshot, is_electric: bool) {
    let fuel = life.fuel_percent.clamp(0.0, 1.0);
    let fuel_color = if fuel <= 0.0 {
        egui::Color32::from_rgb(221, 0, 0)
    } else if fuel <= 0.2 {
        warning_color()
    } else {
        egui::Color32::from_white_alpha(128)
    };
    painter.text(
        hud.min + egui::vec2(337.0, -2.0),
        egui::Align2::LEFT_TOP,
        if is_electric { "⚡" } else { "⛽" },
        egui::FontId::proportional(27.0),
        fuel_color,
    );
    painter.line(
        vec![
            hud.min + egui::vec2(378.0, 88.0),
            hud.min + egui::vec2(371.0, 88.0),
            hud.min + egui::vec2(371.0, 1.0),
            hud.min + egui::vec2(378.0, 1.0),
        ],
        egui::Stroke::new(
            2.0_f32,
            egui::Color32::from_rgba_unmultiplied(128, 128, 128, 221),
        ),
    );
    let bar = egui::Rect::from_min_size(hud.min + egui::vec2(374.0, 3.0), egui::vec2(6.0, 82.0));
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
    if life.oil_remaining_m <= 0.0 && !is_electric {
        painter.text(
            hud.min + egui::vec2(84.0, 216.0),
            egui::Align2::LEFT_TOP,
            "◆",
            egui::FontId::proportional(48.0),
            egui::Color32::from_rgb(221, 0, 0),
        );
    }
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
        .fixed_pos([240.0, (screen_size[1] - 358.0).max(0.0)])
        .show(context, |ui| {
            navigation(ui, telemetry, location.position, distance, target);
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
        let mut last_refuel_sample: Option<(i32, u32)> = None;
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
                            let at_gas = locations
                                .nearest(LocationKind::Gas, telemetry.position)
                                .is_some_and(|(_, distance)| distance <= 25.0);
                            if at_gas {
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
                                    REFUEL_LITERS_PER_SECOND * elapsed_s,
                                );
                                life = simulation
                                    .current(telemetry.car_ordinal)
                                    .expect("updated vehicle state");
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
                                life = simulation.update_with_capacity(&telemetry, tank_liters);
                            }
                        } else {
                            last_refuel_sample = None;
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

fn navigation(
    ui: &mut egui::Ui,
    telemetry: &Telemetry,
    target_position: [f32; 3],
    distance: f32,
    target: LocationKind,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(66.0, 67.0), egui::Sense::hover());
    let painter = ui.painter();
    let arrow_rect = egui::Rect::from_min_size(rect.min, egui::vec2(58.0, 58.0));
    let center = arrow_rect.center();
    painter.circle_stroke(
        center,
        27.0,
        egui::Stroke::new(4.0_f32, egui::Color32::from_white_alpha(17)),
    );
    let angle = (target_position[0] - telemetry.position[0])
        .atan2(target_position[2] - telemetry.position[2])
        - telemetry.yaw;
    let direction = egui::vec2(angle.sin(), -angle.cos());
    let right = egui::vec2(-direction.y, direction.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + direction * 15.0,
            center - direction * 10.0 + right * 10.0,
            center - direction * 6.5,
            center - direction * 10.0 - right * 9.0,
        ],
        egui::Color32::WHITE,
        egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(29, 29, 27)),
    ));
    let (value, unit) = if distance >= 1_000.0 {
        (format!("{:.1}", distance / 1_000.0), " KM")
    } else {
        (format!("{distance:.0}"), " M")
    };
    painter.text(
        rect.min + egui::vec2(29.0, -16.0),
        egui::Align2::CENTER_TOP,
        format!("{value}{unit}"),
        forza_font(20.0, true),
        egui::Color32::WHITE,
    );
    painter.text(
        rect.min + egui::vec2(40.0, 41.0),
        egui::Align2::LEFT_TOP,
        match target {
            LocationKind::Gas => "⛽",
            LocationKind::Workshop => "🔧",
            LocationKind::ConvenienceStore => "●",
        },
        egui::FontId::proportional(18.0),
        egui::Color32::from_white_alpha(128),
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
    use super::{BoostGaugeState, boost_needle_angle, hud_rect, menu_rect, vehicle_card_rect};
    use eframe::egui;

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
    fn windows_layout_uses_the_original_1080p_anchors() {
        assert_eq!(
            menu_rect([1920.0, 1080.0]),
            egui::Rect::from_min_size(egui::pos2(20.0, 430.0), egui::vec2(290.0, 220.0),)
        );
        assert_eq!(
            vehicle_card_rect([1920.0, 1080.0]),
            egui::Rect::from_min_size(egui::pos2(585.0, 300.0), egui::vec2(750.0, 480.0),)
        );
        assert_eq!(
            hud_rect([1920.0, 1080.0]),
            egui::Rect::from_min_size(egui::pos2(1518.0, 787.0), egui::vec2(400.0, 291.0),)
        );
    }
}
