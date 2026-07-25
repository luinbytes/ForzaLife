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

#[test]
fn bundled_world_map_matches_the_windows_points_of_interest() {
    let locations = Locations::bundled();

    assert_eq!(locations.locations.len(), 118);
    assert_eq!(
        locations
            .locations
            .iter()
            .filter(|location| location.kind == LocationKind::Gas)
            .count(),
        61
    );
    assert_eq!(
        locations
            .locations
            .iter()
            .filter(|location| location.kind == LocationKind::Workshop)
            .count(),
        20
    );
    assert_eq!(
        locations
            .locations
            .iter()
            .filter(|location| location.kind == LocationKind::ConvenienceStore)
            .count(),
        37
    );
}

#[test]
fn saving_does_not_duplicate_the_bundled_world_map() {
    let path =
        std::env::temp_dir().join(format!("forzalife-locations-{}.json", std::process::id()));
    let locations = Locations::load(&path);
    locations.save(&path).expect("save locations");
    let reloaded = Locations::load(&path);
    std::fs::remove_file(path).expect("remove test locations");

    assert_eq!(reloaded.locations.len(), 118);
}
