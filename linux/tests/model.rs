use forzalife::{
    model::{DEFAULT_TANK_LITERS, OIL_INTERVAL_METERS, Simulation},
    telemetry::Telemetry,
};

fn telemetry(timestamp_ms: u32, distance_m: f32, power_w: f32) -> Telemetry {
    Telemetry {
        race_on: false,
        timestamp_ms,
        engine_max_rpm: 8_000.0,
        engine_idle_rpm: 900.0,
        current_engine_rpm: 4_000.0,
        yaw: 0.0,
        power_w,
        throttle: 200,
        car_ordinal: 42,
        num_cylinders: 4,
        position: [0.0; 3],
        speed_mps: 30.0,
        boost_psi: 5.0,
        fuel: 1.0,
        distance_m,
        gear: 4,
    }
}

#[test]
fn driving_consumes_fuel_and_advances_per_car_maintenance() {
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 100.0, 80_000.0));
    let life = simulation.update(&telemetry(2_000, 130.0, 80_000.0));

    assert!(life.fuel_liters < DEFAULT_TANK_LITERS);
    assert_eq!(life.odometer_m, 30.0);
    assert_eq!(life.oil_remaining_m, OIL_INTERVAL_METERS - 30.0);
}

#[test]
fn implausible_distance_resets_do_not_inflate_the_odometer() {
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 10_000.0, 0.0));
    let life = simulation.update(&telemetry(2_000, 5.0, 0.0));

    assert_eq!(life.odometer_m, 0.0);
}

#[test]
fn each_car_keeps_independent_life_state() {
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 0.0, 100_000.0));
    let car_42 = simulation.update(&telemetry(2_000, 100.0, 100_000.0));

    let mut other = telemetry(3_000, 500.0, 0.0);
    other.car_ordinal = 99;
    let car_99 = simulation.update(&other);

    assert!(car_42.fuel_liters < car_99.fuel_liters);
    assert_eq!(car_99.odometer_m, 0.0);
}

#[test]
fn paused_usage_stops_fuel_consumption_but_keeps_distance() {
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 0.0, 100_000.0));
    assert!(simulation.toggle_usage(42));
    let life = simulation.update(&telemetry(2_000, 100.0, 100_000.0));

    assert_eq!(life.fuel_liters, DEFAULT_TANK_LITERS);
    assert_eq!(life.trip_m, 100.0);
    assert!(life.is_usage_paused);
}

#[test]
fn vehicle_capacity_controls_fuel_percentage_and_refill() {
    let mut simulation = Simulation::default();
    let life = simulation.update_with_capacity(&telemetry(1_000, 0.0, 0.0), 130.0);
    assert_eq!(life.fuel_liters, 130.0);
    assert_eq!(life.fuel_percent, 1.0);

    simulation.refuel(42);
    let mut stopped = telemetry(2_000, 0.0, 0.0);
    stopped.current_engine_rpm = 0.0;
    let life = simulation.update_with_capacity(&stopped, 130.0);
    assert_eq!(life.fuel_liters, 130.0);
}

#[test]
fn trip_odometer_survives_a_save_and_load() {
    let path = std::env::temp_dir().join(format!("forzalife-model-{}.json", std::process::id()));
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 0.0, 0.0));
    simulation.update(&telemetry(2_000, 100.0, 0.0));
    simulation.save(&path).expect("save state");

    let loaded = Simulation::load(&path);
    std::fs::remove_file(path).expect("remove test state");

    assert_eq!(loaded.current(42).expect("car state").trip_m, 100.0);
}
