#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocationKind {
    Gas,
    Workshop,
    ConvenienceStore,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub name: String,
    pub kind: LocationKind,
    pub position: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Locations {
    pub locations: Vec<Location>,
}

impl Locations {
    pub fn bundled() -> Self {
        let mut locations = Vec::new();
        let mut counts = [0_usize; 3];
        for line in include_str!("../assets/world_map.csv").lines().skip(1) {
            let fields: Vec<_> = line.split(',').collect();
            if fields.len() != 4 {
                continue;
            }
            let (kind, label, count_index) = match fields[0] {
                "1" => (LocationKind::Gas, "Gas station", 0),
                "2" => (LocationKind::Workshop, "Workshop", 1),
                "3" => (LocationKind::ConvenienceStore, "Konbini", 2),
                _ => continue,
            };
            let Some(position) = fields[1..4]
                .iter()
                .map(|value| value.parse().ok())
                .collect::<Option<Vec<f32>>>()
                .and_then(|values| values.try_into().ok())
            else {
                continue;
            };
            counts[count_index] += 1;
            locations.push(Location {
                name: format!("{label} {}", counts[count_index]),
                kind,
                position,
            });
        }
        Self { locations }
    }

    pub fn nearest(&self, kind: LocationKind, position: [f32; 3]) -> Option<(&Location, f32)> {
        self.locations
            .iter()
            .filter(|location| location.kind == kind)
            .map(|location| (location, distance(location.position, position)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
    }
}

fn distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2) + (left[2] - right[2]).powi(2))
        .sqrt()
}
