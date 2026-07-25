use forzalife::car::CarDatabase;

#[test]
fn bundled_car_database_matches_the_windows_vehicle_metadata() {
    let cars = CarDatabase::bundled();
    let car = cars.get(247).expect("Toyota 2000GT");

    assert_eq!(cars.len(), 638);
    assert_eq!(car.make, "Toyota");
    assert_eq!(car.model_full, "2000GT");
    assert_eq!(car.year, 1969);
    assert_eq!(car.country, "Japan");
    assert_eq!(car.fuel_capacity_liters, 60.0);

    let peel = cars.get(2987).expect("Peel P50");
    assert_eq!(peel.tank_capacity_liters(1), 35.0);
    assert_eq!(peel.tank_capacity_liters(8), 80.0);
    assert_eq!(
        forzalife::car::CarInfo::fallback_tank_capacity_liters(6),
        65.0
    );
}
