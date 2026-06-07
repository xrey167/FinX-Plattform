# tdw-spatial

Minimal, validated 2-D geospatial primitives: lat/lon `Point`s, axis-aligned
`BoundingBox` containment, and Manhattan distance — with coordinate validation
that rejects non-finite or out-of-range values.

## Purpose

`tdw-spatial` provides the small spatial vocabulary the platform needs for venue
/ region geolocation:

- `Point { lat, lon }` and `BoundingBox { min, max }`;
- `BoundingBox::contains(point)` — point-in-box test;
- `manhattan_distance(a, b)` — L1 distance between two points;
- `validate_point` / `validate_bounding_box` — coordinate-range guards.

Each operation has a checked (`try_*`) form returning `Result<_, SpatialError>`
and a convenience form that returns `false`/`NaN` on invalid input.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `Point` / `BoundingBox` (de)serialization.

## Quickstart

```rust
use tdw_spatial::{BoundingBox, Point, manhattan_distance};

let bbox = BoundingBox {
    min: Point { lat: 40.0, lon: -75.0 },
    max: Point { lat: 41.0, lon: -73.0 },
};
let nyc = Point { lat: 40.7, lon: -74.0 };

assert!(bbox.contains(nyc));
assert!(manhattan_distance(nyc, Point { lat: 40.8, lon: -73.9 }) > 0.0);
```

Run the worked example:

```text
cargo run -p tdw-spatial --example tdw-spatial-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — coordinate validation and containment model.
