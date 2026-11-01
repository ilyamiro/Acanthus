//! Line chart and sparkline: a canvas plotting a list of numbers as a stroked line over an optional area fill and grid lines.

use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{Node, NodeId, Path, Scene};

use crate::component::{Component, Prop, PropSpec, Props};

/// The value range a plot covers: `min` to `max` when `max > min`, otherwise the range of the data (a flat series gets a range of one around it).
pub fn bounds(values: &[f32], min: f32, max: f32) -> (f32, f32) {
    if max > min {
        return (min, max);
    }
    let (lo, hi) = values.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if lo > hi {
        (0.0, 1.0)
    } else if hi > lo {
        (lo, hi)
    } else {
        (lo - 0.5, hi + 0.5)
    }
}

/// Canvas points of `values` in a `size` area kept `inset` away from every edge; the first value sits at the left and the last at the right, higher values are higher up.
pub fn plot(values: &[f32], size: Vec2, inset: f32, min: f32, max: f32) -> Vec<Vec2> {
    let (lo, hi) = bounds(values, min, max);
    let (w, h) = ((size.x - 2.0 * inset).max(0.0), (size.y - 2.0 * inset).max(0.0));
    let last = values.len().saturating_sub(1).max(1) as f32;
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = if values.len() == 1 { w / 2.0 } else { w * i as f32 / last };
            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            Vec2::new(inset + x, inset + h * (1.0 - t))
        })
        .collect()
}

/// Description of a line chart.
pub struct LineChart {
    /// The plotted series, oldest first.
    pub values: Signal<Vec<f32>>,
    /// Line colour.
    pub color: Signal<Color>,
    /// Grid line colour.
    pub grid_color: Signal<Color>,
    /// Lowest plotted value; ignored unless `max` is greater.
    pub min: Signal<f32>,
    /// Highest plotted value; ignored unless greater than `min`.
    pub max: Signal<f32>,
    /// Width of the chart.
    pub width: f32,
    /// Height of the chart.
    pub height: f32,
    /// Stroke width of the line.
    pub thickness: f32,
    /// Opacity of the area under the line, 0 for none.
    pub fill: f32,
    /// Number of horizontal grid lines, evenly spread.
    pub grid: u32,
}

/// Builds the chart under `parent` and returns it; it repaints when a signal it reads changes.
pub fn line_chart(scene: &mut Scene, parent: NodeId, chart: LineChart) -> NodeId {
    let LineChart { values, color, grid_color, min, max, width, height, thickness, fill, grid } = chart;
    let size = Vec2::new(width, height);
    let inset = thickness / 2.0;
    scene.add(
        parent,
        Node::canvas(move |painting| {
            for i in 0..grid {
                let y = inset + (height - thickness) * (i as f32 + 0.5) / grid as f32;
                painting.stroke(
                    &Path::new().move_to(Vec2::new(0.0, y)).line_to(Vec2::new(width, y)),
                    grid_color.get(),
                    1.0,
                );
            }
            let points = plot(&values.get(), size, inset, min.get(), max.get());
            let Some((first, rest)) = points.split_first() else { return };
            let line = rest.iter().fold(Path::new().move_to(*first), |path, &p| path.line_to(p));
            let tint = color.get();
            if fill > 0.0 && !rest.is_empty() {
                let last = points[points.len() - 1];
                let area = line.clone().line_to(Vec2::new(last.x, height)).line_to(Vec2::new(first.x, height)).close();
                painting.fill(&area, tint.with_alpha(tint.a * fill));
            }
            painting.stroke(&line, tint, thickness);
        })
        .size(width, height),
    )
}

/// The chart as a [`Component`] named `Chart` (grid lines, area fill) or `Sparkline` (the line alone).
pub struct ChartComponent {
    /// Builds the plain sparkline instead of the gridded chart.
    pub sparkline: bool,
}

impl Component for ChartComponent {
    fn name(&self) -> &'static str {
        if self.sparkline { "Sparkline" } else { "Chart" }
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number_list("values"),
            PropSpec::number("min", || Prop::number(0.0)),
            PropSpec::number("max", || Prop::number(0.0)),
            PropSpec::number("width", || Prop::number(160.0)),
            PropSpec::number("height", || Prop::number(48.0)),
            PropSpec::number("thickness", || Prop::number(2.0)),
            PropSpec::number("fill", || Prop::number(0.2)),
            PropSpec::number("grid", || Prop::number(0.0)),
            PropSpec::themed("color", "primary"),
            PropSpec::themed("grid_color", "surface1"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let get = |name: &str| props.number(name).get_untracked();
        line_chart(
            scene,
            parent,
            LineChart {
                values: props.number_list("values"),
                color: props.color("color"),
                grid_color: props.color("grid_color"),
                min: props.number("min"),
                max: props.number("max"),
                width: get("width"),
                height: get("height"),
                thickness: get("thickness"),
                fill: if self.sparkline { 0.0 } else { get("fill") },
                grid: if self.sparkline { 0 } else { get("grid") as u32 },
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use acanthus_scene::Fonts;

    use super::*;
    use crate::component::Registry;

    #[test]
    fn bounds_fall_back_to_the_data() {
        assert_eq!(bounds(&[2.0, 6.0, 4.0], 0.0, 0.0), (2.0, 6.0));
        assert_eq!(bounds(&[3.0, 3.0], 0.0, 0.0), (2.5, 3.5));
        assert_eq!(bounds(&[], 0.0, 0.0), (0.0, 1.0));
        assert_eq!(bounds(&[2.0, 6.0], 0.0, 10.0), (0.0, 10.0));
    }

    #[test]
    fn plot_spreads_values_across_the_area_with_high_values_up() {
        let p = plot(&[0.0, 5.0, 10.0], Vec2::new(104.0, 54.0), 2.0, 0.0, 10.0);
        assert_eq!(p, vec![Vec2::new(2.0, 52.0), Vec2::new(52.0, 27.0), Vec2::new(102.0, 2.0)]);
        assert_eq!(plot(&[7.0], Vec2::new(100.0, 50.0), 0.0, 0.0, 0.0), vec![Vec2::new(50.0, 25.0)]);
        assert!(plot(&[], Vec2::new(10.0, 10.0), 0.0, 0.0, 0.0).is_empty());
        let clamped = plot(&[20.0], Vec2::new(10.0, 10.0), 0.0, 0.0, 10.0);
        assert_eq!(clamped[0].y, 0.0);
    }

    #[test]
    fn the_components_build_and_the_chart_sizes_itself() {
        let mut registry = Registry::new();
        registry.register(ChartComponent { sparkline: false });
        registry.register(ChartComponent { sparkline: true });
        let colors: Vec<(String, Prop)> =
            ["color", "grid_color"].map(|n| (n.to_owned(), Prop::color(Color::hex(0x778899)))).to_vec();
        let values = Signal::new(vec![1.0, 3.0, 2.0]);
        let mut given = colors.clone();
        given.push(("values".to_owned(), Prop::NumberList(values.clone())));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let chart = registry.build("Chart", &mut scene, root, given).unwrap();
        let spark = registry.build("Sparkline", &mut scene, root, colors).unwrap();
        scene.frame();
        values.set(vec![4.0, 1.0]);
        scene.frame();
        assert!(scene.geometry(chart).is_some_and(|g| g.w == 160.0 && g.h == 48.0));
        assert!(scene.geometry(spark).is_some_and(|g| g.w == 160.0));
    }
}
