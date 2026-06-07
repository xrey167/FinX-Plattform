# tdw-spatial — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `Point` | `{ lat: f64, lon: f64 }`. |
| `BoundingBox` | `{ min: Point, max: Point }`. |
| `SpatialError` | `NonFiniteCoordinate`, `LatitudeOutOfRange`, `LongitudeOutOfRange`, `InvalidBoundingBox`. |
| `BoundingBox::contains` / `try_contains` | Point-in-box (convenience / checked). |
| `manhattan_distance` / `try_manhattan_distance` | L1 distance (convenience / checked). |
| `validate_point` / `validate_bounding_box` | Public coordinate guards. |
| `contains_unchecked` (private) | Range comparison after validation. |

## Key types and traits

- `Point` and `BoundingBox` derive `Clone, Copy, Debug, PartialEq, Serialize,
  Deserialize` (`Copy` because they are small POD; not `Eq` — they hold `f64`).
- `SpatialError` is a `Copy`, `Eq` enum (no `serde`).

## Containment / distance model

```
validate_point(p):
    lat & lon finite?            else NonFiniteCoordinate
    -90  <= lat <= 90 ?          else LatitudeOutOfRange
    -180 <= lon <= 180 ?         else LongitudeOutOfRange

validate_bounding_box(b):
    validate_point(min)?; validate_point(max)?
    min.lat <= max.lat && min.lon <= max.lon ?  else InvalidBoundingBox

try_contains(point):
    validate_bounding_box(self)?; validate_point(point)?
    ▶ min.lat <= p.lat <= max.lat && min.lon <= p.lon <= max.lon

try_manhattan_distance(a, b):
    validate_point(a)?; validate_point(b)?
    ▶ |a.lat - b.lat| + |a.lon - b.lon|
```

The convenience wrappers degrade gracefully: `contains` returns `false` on any
validation error, `manhattan_distance` returns `f64::NAN`. Use the `try_*` forms
when the distinction between "outside" and "invalid input" matters.

## Invariants

- Coordinates must be **finite** and within WGS84 ranges: latitude `[-90, 90]`,
  longitude `[-180, 180]`.
- A `BoundingBox` is valid only when `min` is the lower-left corner
  (`min.lat <= max.lat` and `min.lon <= max.lon`); an inverted box is
  `InvalidBoundingBox`.
- Containment is **inclusive** on all four edges.
- `manhattan_distance` is the L1 (taxicab) metric over raw lat/lon degrees — it is
  a cheap ordering/proximity heuristic, not a great-circle distance.
- Pure and deterministic: no I/O, no global state.
