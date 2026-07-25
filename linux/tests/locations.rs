use forzalife::locations::{LocationKind, Locations};

#[test]
fn nearest_service_location_is_selected_by_world_distance() {
    let mut locations = Locations::default();
    locations.add(LocationKind::Gas, [100.0, 0.0, 0.0]);
    locations.add(LocationKind::Gas, [20.0, 0.0, 0.0]);
    locations.add(LocationKind::Workshop, [1.0, 0.0, 0.0]);

    let (nearest, distance) = locations
        .nearest(LocationKind::Gas, [0.0; 3])
        .expect("gas station");

    assert_eq!(nearest.name, "Gas station 2");
    assert_eq!(distance, 20.0);
}
