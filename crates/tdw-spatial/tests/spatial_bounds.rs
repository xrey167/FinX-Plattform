#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-spatial: BoundingBox edge inclusion, four-way
// rejection, manhattan distance symmetry and self-distance, degenerate boxes.
// Authored offline. Verify with `cargo test --package tdw-spatial`.

use tdw_spatial::{BoundingBox, Point, manhattan_distance};

fn nyc_box() -> BoundingBox {
    BoundingBox {
        min: Point {
            lat: 40.0,
            lon: -75.0,
        },
        max: Point {
            lat: 41.0,
            lon: -73.0,
        },
    }
}

#[test]
fn contains_interior_point() {
    assert!(nyc_box().contains(Point {
        lat: 40.5,
        lon: -74.0
    }));
}

#[test]
fn contains_treats_corners_as_inclusive() {
    let bbox = nyc_box();
    assert!(bbox.contains(bbox.min));
    assert!(bbox.contains(bbox.max));
    assert!(bbox.contains(Point {
        lat: 40.0,
        lon: -73.0
    }));
    assert!(bbox.contains(Point {
        lat: 41.0,
        lon: -75.0
    }));
}

#[test]
fn contains_rejects_points_outside_in_each_direction() {
    let bbox = nyc_box();
    assert!(
        !bbox.contains(Point {
            lat: 39.9,
            lon: -74.0
        }),
        "below min lat"
    );
    assert!(
        !bbox.contains(Point {
            lat: 41.1,
            lon: -74.0
        }),
        "above max lat"
    );
    assert!(
        !bbox.contains(Point {
            lat: 40.5,
            lon: -75.1
        }),
        "below min lon"
    );
    assert!(
        !bbox.contains(Point {
            lat: 40.5,
            lon: -72.9
        }),
        "above max lon"
    );
}

#[test]
fn degenerate_box_contains_only_the_single_point() {
    let p = Point {
        lat: 10.0,
        lon: 10.0,
    };
    let bbox = BoundingBox { min: p, max: p };
    assert!(bbox.contains(p));
    assert!(!bbox.contains(Point {
        lat: 10.0,
        lon: 10.0001
    }));
}

#[test]
fn manhattan_distance_is_zero_for_identical_points() {
    let p = Point { lat: 1.0, lon: 2.0 };
    assert_eq!(manhattan_distance(p, p), 0.0);
}

#[test]
fn manhattan_distance_is_symmetric() {
    let a = Point { lat: 1.0, lon: 2.0 };
    let b = Point { lat: 4.0, lon: 6.0 };
    let ab = manhattan_distance(a, b);
    let ba = manhattan_distance(b, a);
    assert!((ab - ba).abs() < f64::EPSILON);
    assert!((ab - 7.0).abs() < f64::EPSILON, "|1-4|+|2-6| = 7.0");
}

#[test]
fn manhattan_distance_handles_negative_coordinates() {
    let a = Point {
        lat: -1.0,
        lon: -1.0,
    };
    let b = Point { lat: 2.0, lon: 3.0 };
    let d = manhattan_distance(a, b);
    assert!((d - 7.0).abs() < f64::EPSILON);
}

#[test]
fn point_round_trips_via_serde() {
    let p = Point {
        lat: 40.7,
        lon: -74.0,
    };
    let json = serde_json::to_string(&p).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: Point = serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, p);
}

#[test]
fn bounding_box_round_trips_via_serde() {
    let bbox = nyc_box();
    let json = serde_json::to_string(&bbox).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: BoundingBox =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, bbox);
}
