//! Turn a price series into a chart payload for the `/chart` command.
//!
//! By default this builds the `tdw-charting` Plotly figure JSON (a spec, not
//! pixels) from the bars' close series. `tdw-charting` deliberately emits a
//! figure *spec* so a `plotly.js` client renders it with zero native graphics
//! dependency — but a chat client wants an image. Rasterizing Plotly JSON in
//! pure Rust is not feasible, so under the NON-DEFAULT `charts` feature we
//! instead render a simple line PNG straight from the close series with the
//! pure-Rust `plotters` bitmap backend (no native/system graphics dependency).
//! Without `charts`, the adapter posts the caption plus the figure-JSON spec.

use tdw_charting::LinePoint;
use tdw_domain::MarketDataBar;

use crate::response::ChartReply;

/// Project a bar series to the close-price line points `tdw-charting` consumes.
fn close_points(bars: &[MarketDataBar]) -> Vec<LinePoint> {
    bars.iter()
        .map(|bar| LinePoint::new(bar.ts.clone(), Some(bar.close)))
        .collect()
}

/// Build a [`ChartReply`] for `ticker` from its recent bars.
///
/// The Plotly figure JSON is always populated. When built with the `charts`
/// feature, a rasterized PNG of the close series is attached as well.
#[must_use]
pub fn chart_reply(ticker: &str, bars: &[MarketDataBar]) -> ChartReply {
    let points = close_points(bars);
    let figure_json = tdw_charting::line(&points);
    let caption = format!("{ticker} — {} points", points.len());
    ChartReply {
        caption,
        figure_json,
        png: render_png(ticker, &points),
    }
}

/// Rasterize the close series to a PNG. Returns `None` unless the `charts`
/// feature is enabled (the default build attaches only the figure spec).
///
/// The `f32` casts (sample index, close price, point count) are inherent to a
/// pixel chart: the rasterizer's coordinate space is `f32`, and a sub-pixel
/// rounding of an index or price is exactly the precision a chart already has.
#[cfg(feature = "charts")]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "pixel-space chart coordinates are intentionally f32"
)]
#[must_use]
fn render_png(ticker: &str, points: &[LinePoint]) -> Option<Vec<u8>> {
    use plotters::prelude::{
        BLUE, BitMapBackend, ChartBuilder, IntoDrawingArea, LineSeries, WHITE,
    };
    use plotters::style::Color as _;

    const WIDTH: u32 = 640;
    const HEIGHT: u32 = 360;

    // The ticker is intentionally unused in the raster: drawing text needs a
    // font backend we deliberately do not enable (keeping the build pure-Rust
    // with no fontconfig/freetype). The caption travels on `ChartReply.caption`.
    let _ = ticker;

    let values: Vec<(f32, f32)> = points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.y.map(|y| (i as f32, y as f32)))
        .collect();
    if values.len() < 2 {
        return None;
    }

    let (y_min, y_max) = values
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &(_, y)| {
            (lo.min(y), hi.max(y))
        });
    // Pad the y-range so the line is not flush against the frame; guard a flat
    // series (y_min == y_max) so the range stays non-degenerate.
    let pad = ((y_max - y_min) * 0.05).max(1.0);
    let x_max = (values.len() - 1) as f32;

    let mut buffer = vec![0_u8; (WIDTH * HEIGHT * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut buffer, (WIDTH, HEIGHT)).into_drawing_area();
        if root.fill(&WHITE).is_err() {
            return None;
        }
        // No caption / labels: text rendering needs a font backend we do not
        // enable. We draw only the gridless line series into a bare coordinate.
        let built = ChartBuilder::on(&root)
            .margin(10)
            .build_cartesian_2d(0_f32..x_max, (y_min - pad)..(y_max + pad));
        let Ok(mut chart) = built else {
            return None;
        };
        if chart
            .draw_series(LineSeries::new(values, BLUE.stroke_width(2)))
            .is_err()
        {
            return None;
        }
        if root.present().is_err() {
            return None;
        }
    }
    encode_png(&buffer, WIDTH, HEIGHT)
}

/// The no-`charts` build attaches only the figure spec — no PNG is produced.
#[cfg(not(feature = "charts"))]
#[must_use]
const fn render_png(_ticker: &str, _points: &[LinePoint]) -> Option<Vec<u8>> {
    None
}

/// Encode the `plotters`-filled RGB buffer to in-memory PNG bytes with the
/// pure-Rust `png` crate (no native codec dependency).
#[cfg(feature = "charts")]
fn encode_png(rgb: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgb).ok()?;
    }
    Some(out)
}
