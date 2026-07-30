use forzalife::locations::{Location, LocationKind, Locations};

#[test]
fn nearest_service_location_is_selected_by_world_distance() {
    let locations = Locations {
        locations: vec![
            Location {
                name: "Gas station 1".to_owned(),
                kind: LocationKind::Gas,
                position: [100.0, 0.0, 0.0],
            },
            Location {
                name: "Gas station 2".to_owned(),
                kind: LocationKind::Gas,
                position: [20.0, 0.0, 0.0],
            },
            Location {
                name: "Workshop 1".to_owned(),
                kind: LocationKind::Workshop,
                position: [1.0, 0.0, 0.0],
            },
        ],
    };

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
