#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: Point,
    pub max: Point,
}

impl BoundingBox {
    pub fn contains(&self, point: Point) -> bool {
        point.lat >= self.min.lat
            && point.lat <= self.max.lat
            && point.lon >= self.min.lon
            && point.lon <= self.max.lon
    }
}

pub fn manhattan_distance(left: Point, right: Point) -> f64 {
    (left.lat - right.lat).abs() + (left.lon - right.lon).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounding_box_contains_point_and_distance_is_nonzero() {
        let bbox = BoundingBox {
            min: Point {
                lat: 40.0,
                lon: -75.0,
            },
            max: Point {
                lat: 41.0,
                lon: -73.0,
            },
        };
        let nyc = Point {
            lat: 40.7,
            lon: -74.0,
        };

        assert!(bbox.contains(nyc));
        assert!(
            manhattan_distance(
                nyc,
                Point {
                    lat: 40.8,
                    lon: -73.9
                }
            ) > 0.0
        );
    }
}
