//! Offline `tdw-spatial` example: test point-in-box containment, compute a
//! Manhattan distance, and show that an inverted bounding box is rejected.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-spatial --example basic
//! ```

use tdw_spatial::{BoundingBox, Point, manhattan_distance, validate_bounding_box};

fn main() {
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
    let other = Point {
        lat: 40.8,
        lon: -73.9,
    };

    // Meaningful operations on inline data.
    println!("box contains NYC: {}", bbox.contains(nyc));
    println!(
        "manhattan(nyc, other): {:.3}",
        manhattan_distance(nyc, other)
    );

    // An inverted bounding box (min above max) is invalid.
    let inverted = BoundingBox {
        min: Point {
            lat: 42.0,
            lon: -73.0,
        },
        max: Point {
            lat: 41.0,
            lon: -74.0,
        },
    };
    println!(
        "inverted box rejected: {}",
        validate_bounding_box(&inverted).is_err()
    );
}
