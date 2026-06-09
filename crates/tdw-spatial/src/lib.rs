#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpatialError {
    NonFiniteCoordinate,
    LatitudeOutOfRange,
    LongitudeOutOfRange,
    InvalidBoundingBox,
}

impl BoundingBox {
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        self.try_contains(point).unwrap_or(false)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_contains(&self, point: Point) -> Result<bool, SpatialError> {
        validate_bounding_box(self)?;
        validate_point(point)?;
        Ok(self.contains_unchecked(point))
    }

    fn contains_unchecked(&self, point: Point) -> bool {
        point.lat >= self.min.lat
            && point.lat <= self.max.lat
            && point.lon >= self.min.lon
            && point.lon <= self.max.lon
    }
}

#[must_use]
pub fn manhattan_distance(left: Point, right: Point) -> f64 {
    try_manhattan_distance(left, right).unwrap_or(f64::NAN)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn try_manhattan_distance(left: Point, right: Point) -> Result<f64, SpatialError> {
    validate_point(left)?;
    validate_point(right)?;
    Ok((left.lat - right.lat).abs() + (left.lon - right.lon).abs())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_point(point: Point) -> Result<(), SpatialError> {
    if !point.lat.is_finite() || !point.lon.is_finite() {
        return Err(SpatialError::NonFiniteCoordinate);
    }
    if !(-90.0..=90.0).contains(&point.lat) {
        return Err(SpatialError::LatitudeOutOfRange);
    }
    if !(-180.0..=180.0).contains(&point.lon) {
        return Err(SpatialError::LongitudeOutOfRange);
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_bounding_box(bbox: &BoundingBox) -> Result<(), SpatialError> {
    validate_point(bbox.min)?;
    validate_point(bbox.max)?;
    if bbox.min.lat > bbox.max.lat || bbox.min.lon > bbox.max.lon {
        return Err(SpatialError::InvalidBoundingBox);
    }
    Ok(())
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

    #[test]
    fn checked_spatial_paths_reject_bad_coordinates_and_boxes() {
        assert_eq!(
            validate_point(Point {
                lat: f64::NAN,
                lon: 0.0,
            }),
            Err(SpatialError::NonFiniteCoordinate)
        );
        assert_eq!(
            validate_bounding_box(&BoundingBox {
                min: Point {
                    lat: 42.0,
                    lon: -73.0
                },
                max: Point {
                    lat: 41.0,
                    lon: -74.0
                },
            }),
            Err(SpatialError::InvalidBoundingBox)
        );
    }
}
