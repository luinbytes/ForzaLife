use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationKind {
    Gas,
    Workshop,
    ConvenienceStore,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub name: String,
    pub kind: LocationKind,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

    pub fn add(&mut self, kind: LocationKind, position: [f32; 3]) {
        let number = self
            .locations
            .iter()
            .filter(|location| location.kind == kind)
            .count()
            + 1;
        let label = match kind {
            LocationKind::Gas => "Gas station",
            LocationKind::Workshop => "Workshop",
            LocationKind::ConvenienceStore => "Konbini",
        };
        self.locations.push(Location {
            name: format!("{label} {number}"),
            kind,
            position,
        });
    }

    pub fn load(path: &Path) -> Self {
        let mut bundled = Self::bundled();
        let custom: Self = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        bundled.locations.extend(custom.locations);
        bundled
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bundled = Self::bundled();
        let locations = self
            .locations
            .strip_prefix(bundled.locations.as_slice())
            .unwrap_or(&self.locations);
        let temporary = path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(&Self {
                locations: locations.to_vec(),
            })?,
        )?;
        fs::rename(temporary, path)
    }
}

fn distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2) + (left[2] - right[2]).powi(2))
        .sqrt()
}
