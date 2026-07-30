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
        torque_nm: 0.0,
        race_position: 0,
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
fn rpm_above_half_redline_smoothly_increases_fuel_burn() {
    let burn_at_rpm = |rpm| {
        let mut sample = telemetry(1_000, 0.0, 80_000.0);
        sample.current_engine_rpm = rpm;
        let mut simulation = Simulation::default();
        simulation.update(&sample);
        sample.timestamp_ms = 2_000;
        DEFAULT_TANK_LITERS - simulation.update(&sample).fuel_liters
    };

    let quarter_redline_burn = burn_at_rpm(2_000.0);
    let half_redline_burn = burn_at_rpm(4_000.0);
    let three_quarter_burn = burn_at_rpm(6_000.0);
    let ninety_percent_burn = burn_at_rpm(7_200.0);
    let redline_burn = burn_at_rpm(8_000.0);
    assert_eq!(quarter_redline_burn, half_redline_burn);
    assert!(three_quarter_burn > half_redline_burn * 1.58);
    assert!(three_quarter_burn < half_redline_burn * 1.62);
    assert!(ninety_percent_burn > half_redline_burn * 2.38);
    assert!(ninety_percent_burn < half_redline_burn * 2.42);
    assert!(redline_burn > half_redline_burn * 2.78);
    assert!(redline_burn < half_redline_burn * 2.82);
}

#[test]
fn power_torque_and_throttle_increase_fuel_burn() {
    let burn = |power_w, torque_nm, throttle| {
        let mut sample = telemetry(1_000, 0.0, power_w);
        sample.current_engine_rpm = 6_000.0;
        sample.torque_nm = torque_nm;
        sample.throttle = throttle;
        let mut simulation = Simulation::default();
        simulation.update(&sample);
        sample.timestamp_ms = 2_000;
        DEFAULT_TANK_LITERS - simulation.update(&sample).fuel_liters
    };

    assert!(burn(300_000.0, 0.0, 255) > burn(80_000.0, 0.0, 255) * 3.0);
    assert!(burn(0.0, 500.0, 255) > burn(0.0, 150.0, 255) * 3.0);
    assert!(burn(0.0, 500.0, 255) > burn(0.0, 500.0, 64) * 3.0);
}

#[test]
fn economy_uses_distance_and_measured_fuel_burn() {
    let mut simulation = Simulation::default();
    for second in 1..=60 {
        simulation.update_with_vehicle(&telemetry(second * 1_000, 0.0, 80_000.0), 60.0, 2020);
    }

    let life = simulation.current(42).expect("car state");
    let mpg = life.average_mpg.expect("enough driving for economy");
    let km_per_liter = life
        .average_km_per_liter
        .expect("enough driving for economy");
    assert!(mpg.is_finite() && mpg > 0.0);
    assert!((km_per_liter - mpg * 1.609_344 / 4.546_09).abs() < 0.01);
}

#[test]
fn economy_readout_responds_to_recent_driving() {
    let mut simulation = Simulation::default();
    for second in 1..=50 {
        let mut sample = telemetry(second * 1_000, 0.0, 35_000.0);
        sample.current_engine_rpm = 2_000.0;
        sample.throttle = 80;
        simulation.update_with_vehicle(&sample, 60.0, 2020);
    }
    let eco = simulation
        .current(42)
        .and_then(|life| life.average_km_per_liter)
        .expect("eco economy");

    for second in 51..=100 {
        let mut sample = telemetry(second * 1_000, 0.0, 300_000.0);
        sample.current_engine_rpm = 7_600.0;
        sample.throttle = 255;
        sample.torque_nm = 500.0;
        simulation.update_with_vehicle(&sample, 60.0, 2020);
    }
    let ragging = simulation
        .current(42)
        .and_then(|life| life.average_km_per_liter)
        .expect("ragging economy");

    assert!(ragging < eco * 0.75, "eco={eco}, ragging={ragging}");
}

#[test]
fn vehicle_profiles_produce_different_economy() {
    let economy_for = |num_cylinders| {
        let mut simulation = Simulation::default();
        for second in 1..=50 {
            let mut sample = telemetry(second * 1_000, 0.0, 100_000.0);
            sample.num_cylinders = num_cylinders;
            sample.throttle = 180;
            simulation.update_with_vehicle(&sample, 60.0, 2020);
        }
        simulation
            .current(42)
            .and_then(|life| life.average_km_per_liter)
            .expect("profile economy")
    };

    let four_cylinder = economy_for(4);
    let eight_cylinder = economy_for(8);
    assert!(
        eight_cylinder < four_cylinder * 0.9,
        "four-cylinder={four_cylinder}, eight-cylinder={eight_cylinder}"
    );
}

#[test]
fn odometer_uses_speed_when_forza_distance_is_stuck() {
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 0.0, 0.0));
    let life = simulation.update(&telemetry(2_000, 0.0, 0.0));

    assert_eq!(life.odometer_m, 30.0);
    assert_eq!(life.trip_m, 30.0);
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
    assert_eq!(life.trip_m, 30.0);
    assert!(life.is_usage_paused);
}

#[test]
fn vehicle_capacity_controls_fuel_percentage_and_refill() {
    let mut simulation = Simulation::default();
    let life = simulation.update_with_capacity(&telemetry(1_000, 0.0, 0.0), 130.0);
    assert_eq!(life.fuel_liters, 130.0);
    assert_eq!(life.fuel_percent, 1.0);

    simulation.refuel(42, 20.0);
    let mut stopped = telemetry(2_000, 0.0, 0.0);
    stopped.current_engine_rpm = 0.0;
    let life = simulation.update_with_capacity(&stopped, 130.0);
    assert_eq!(life.fuel_liters, 130.0);
}

#[test]
fn refuelling_adds_fuel_in_steps_and_stops_at_capacity() {
    let mut simulation = Simulation::default();
    simulation.update_with_capacity(&telemetry(1_000, 0.0, 0.0), 60.0);
    simulation.refuel(42, 0.75);
    assert_eq!(simulation.current(42).expect("car state").fuel_liters, 60.0);

    let consuming = telemetry(2_000, 0.0, 100_000_000.0);
    simulation.update_with_capacity(&consuming, 60.0);
    let before = simulation.current(42).expect("car state").fuel_liters;
    simulation.refuel(42, 0.75);
    let after = simulation.current(42).expect("car state").fuel_liters;
    assert_eq!(after - before, 0.75);

    simulation.refuel(42, 1_000.0);
    assert_eq!(simulation.current(42).expect("car state").fuel_liters, 60.0);
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

    assert_eq!(loaded.current(42).expect("car state").trip_m, 30.0);
}

#[test]
fn every_car_restores_its_complete_life_state() {
    let path = std::env::temp_dir().join(format!(
        "forzalife-per-car-state-{}.json",
        std::process::id()
    ));
    let mut simulation = Simulation::default();

    simulation.update_with_vehicle(&telemetry(1_000, 0.0, 80_000.0), 60.0, 2020);
    simulation.update_with_vehicle(&telemetry(2_000, 0.0, 80_000.0), 60.0, 2020);
    simulation.set_odometer(42, 17_200.0);
    let car_42 = simulation.current(42).expect("first car state");

    let mut other = telemetry(3_000, 0.0, 120_000.0);
    other.car_ordinal = 99;
    other.num_cylinders = 8;
    simulation.update_with_vehicle(&other, 75.0, 2018);
    other.timestamp_ms = 4_000;
    simulation.update_with_vehicle(&other, 75.0, 2018);
    let car_99 = simulation.current(99).expect("second car state");

    simulation.save(&path).expect("save all vehicle state");
    let loaded = Simulation::load(&path);
    std::fs::remove_file(path).expect("remove test state");

    assert!(car_42.average_mpg.is_some());
    assert!(car_99.average_mpg.is_some());
    assert_eq!(loaded.current(42).expect("restored first car"), car_42);
    assert_eq!(loaded.current(99).expect("restored second car"), car_99);
}

#[test]
fn manual_odometer_sync_preserves_the_trip() {
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 0.0, 0.0));
    simulation.update(&telemetry(2_000, 0.0, 0.0));
    simulation.set_odometer(42, 17_200.0);

    let life = simulation.current(42).expect("car state");
    assert_eq!(life.odometer_m, 17_200.0);
    assert_eq!(life.trip_m, 30.0);
}

#[test]
fn failed_odometer_save_rolls_back_the_value() {
    let blocker =
        std::env::temp_dir().join(format!("forzalife-save-blocker-{}", std::process::id()));
    std::fs::write(&blocker, b"not a directory").expect("create save blocker");
    let path = blocker.join("vehicles.json");
    let mut simulation = Simulation::default();
    simulation.update(&telemetry(1_000, 0.0, 0.0));
    simulation.update(&telemetry(2_000, 0.0, 0.0));

    assert!(
        simulation
            .set_odometer_and_save(&path, 42, 17_200.0)
            .is_err()
    );
    assert_eq!(simulation.current(42).expect("car state").odometer_m, 30.0);
    std::fs::remove_file(blocker).expect("remove save blocker");
}

#[test]
fn test_fuel_percentage_is_saved_without_changing_distance() {
    let path = std::env::temp_dir().join(format!("forzalife-fuel-{}.json", std::process::id()));
    let mut simulation = Simulation::default();
    simulation.update_with_capacity(&telemetry(1_000, 0.0, 0.0), 60.0);
    simulation.update_with_capacity(&telemetry(2_000, 0.0, 0.0), 60.0);
    let before = simulation.current(42).expect("car state");

    let changed = simulation
        .set_fuel_percent_and_save(&path, 42, 0.25)
        .expect("save test fuel");
    let loaded = Simulation::load(&path);
    std::fs::remove_file(path).expect("remove test state");

    assert_eq!(changed.fuel_liters, 15.0);
    assert_eq!(changed.odometer_m, before.odometer_m);
    assert_eq!(changed.trip_m, before.trip_m);
    assert_eq!(loaded.current(42).expect("car state").fuel_liters, 15.0);
}

#[test]
fn legacy_economy_history_is_discarded_before_a_new_sample() {
    let path = std::env::temp_dir().join(format!(
        "forzalife-legacy-economy-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        r#"{"vehicles":{"42":{"fuel_liters":50.0,"tank_liters":60.0,"odometer_m":10000.0,"trip_m":1000.0,"fuel_used_liters":20.0,"is_electric":false,"oil_since_service_m":0.0,"is_usage_paused":false}}}"#,
    )
    .expect("write legacy state");
    let mut simulation = Simulation::load(&path);
    std::fs::remove_file(path).expect("remove legacy state");

    let life = simulation.update_with_vehicle(&telemetry(1_000, 0.0, 80_000.0), 60.0, 2020);

    assert_eq!(life.average_mpg, None);
    assert_eq!(life.average_km_per_liter, None);
}
