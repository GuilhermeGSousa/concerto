use concerto_foundation::geometry::delauney::Triangulation2D;
use glam::Vec2;

const DUPLICATE_TOLERANCE: f32 = 1e-4;
const LINE_TOLERANCE: f32 = 1e-3;

pub(crate) struct AimWeights {
    shape: AimShape,
}

enum AimShape {
    Empty,
    Single(usize),
    Line(AimLine),
    Plane(AimPlane),
}

struct AimLine {
    origin: Vec2,
    direction: Vec2,
    stops: Vec<AimStop>,
}

struct AimStop {
    point: usize,
    along: f32,
}

struct AimPlane {
    triangulation: Triangulation2D,
    kept: Vec<usize>,
}

impl AimWeights {
    pub(crate) fn new(points: &[Vec2]) -> Self {
        let mut kept: Vec<usize> = Vec::new();
        for (index, point) in points.iter().enumerate() {
            let duplicate = kept
                .iter()
                .any(|&earlier| points[earlier].distance(*point) < DUPLICATE_TOLERANCE);
            if !duplicate {
                kept.push(index);
            }
        }
        let shape = match kept.len() {
            0 => AimShape::Empty,
            1 => AimShape::Single(kept[0]),
            _ => {
                let line = AimLine::through_farthest(points, &kept);
                if line.holds(points, &kept) {
                    AimShape::Line(line)
                } else {
                    let triangulation =
                        Triangulation2D::build(kept.iter().map(|&index| points[index]).collect());
                    if triangulation.triangles().is_empty() {
                        AimShape::Line(line)
                    } else {
                        AimShape::Plane(AimPlane {
                            triangulation,
                            kept,
                        })
                    }
                }
            }
        };
        Self { shape }
    }

    pub(crate) fn weigh(&self, sample: Vec2, weights: &mut [f32]) {
        weights.fill(0.0);
        match &self.shape {
            AimShape::Empty => {}
            AimShape::Single(point) => set(weights, *point, 1.0),
            AimShape::Line(line) => line.weigh(sample, weights),
            AimShape::Plane(plane) => plane.weigh(sample, weights),
        }
    }
}

impl AimLine {
    fn through_farthest(points: &[Vec2], kept: &[usize]) -> Self {
        let mut first = kept[0];
        let mut second = kept[1];
        let mut farthest = points[first].distance_squared(points[second]);
        for (position, &a) in kept.iter().enumerate() {
            for &b in &kept[position + 1..] {
                let distance = points[a].distance_squared(points[b]);
                if distance > farthest {
                    farthest = distance;
                    first = a;
                    second = b;
                }
            }
        }
        let origin = points[first];
        let direction = (points[second] - origin).normalize_or_zero();
        let mut stops: Vec<AimStop> = kept
            .iter()
            .map(|&point| AimStop {
                point,
                along: (points[point] - origin).dot(direction),
            })
            .collect();
        stops.sort_by(|a, b| a.along.total_cmp(&b.along));
        Self {
            origin,
            direction,
            stops,
        }
    }

    fn holds(&self, points: &[Vec2], kept: &[usize]) -> bool {
        kept.iter().all(|&point| {
            (points[point] - self.origin).perp_dot(self.direction).abs() <= LINE_TOLERANCE
        })
    }

    fn weigh(&self, sample: Vec2, weights: &mut [f32]) {
        let along = (sample - self.origin).dot(self.direction);
        let next = self.stops.partition_point(|stop| stop.along <= along);
        if next == 0 {
            set(weights, self.stops[0].point, 1.0);
        } else if next == self.stops.len() {
            set(weights, self.stops[next - 1].point, 1.0);
        } else {
            let lower = &self.stops[next - 1];
            let upper = &self.stops[next];
            let t = (along - lower.along) / (upper.along - lower.along);
            set(weights, lower.point, 1.0 - t);
            set(weights, upper.point, t);
        }
    }
}

impl AimPlane {
    fn weigh(&self, sample: Vec2, weights: &mut [f32]) {
        let located = self.triangulation.locate_or_nearest(sample);
        let triangle = self.triangulation.triangles()[located.triangle];
        set(weights, self.kept[triangle.a], located.lambda_a);
        set(weights, self.kept[triangle.b], located.lambda_b);
        set(weights, self.kept[triangle.c], located.lambda_c);
    }
}

fn set(weights: &mut [f32], index: usize, weight: f32) {
    if let Some(slot) = weights.get_mut(index) {
        *slot = weight;
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;

    use super::AimWeights;

    fn weigh(points: &[Vec2], sample: Vec2) -> Vec<f32> {
        let mut weights = vec![f32::NAN; points.len()];
        AimWeights::new(points).weigh(sample, &mut weights);
        weights
    }

    fn assert_weights(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (a - e).abs() < 1e-5,
                "weight {index}: {actual:?} != {expected:?}"
            );
        }
    }

    fn assert_pitch_line(points: &[Vec2], low: usize, middle: usize, high: usize) {
        let mut halfway_up = vec![0.0; points.len()];
        halfway_up[middle] = 0.5;
        halfway_up[high] = 0.5;
        assert_weights(&weigh(points, Vec2::new(0.0, 0.25)), &halfway_up);

        let mut quarter_down = vec![0.0; points.len()];
        quarter_down[middle] = 0.5;
        quarter_down[low] = 0.5;
        assert_weights(&weigh(points, Vec2::new(0.3, -0.25)), &quarter_down);

        let mut top = vec![0.0; points.len()];
        top[high] = 1.0;
        assert_weights(&weigh(points, Vec2::new(0.0, 1.5)), &top);

        let mut bottom = vec![0.0; points.len()];
        bottom[low] = 1.0;
        assert_weights(&weigh(points, Vec2::new(-0.2, -2.0)), &bottom);

        for step in -20..=20 {
            let sample = Vec2::new(step as f32 * 0.07, step as f32 * 0.05);
            let weights = weigh(points, sample);
            let total: f32 = weights.iter().sum();
            assert!((total - 1.0).abs() < 1e-5, "{sample}: {weights:?}");
            assert!(weights.iter().all(|weight| *weight >= 0.0), "{weights:?}");
        }
        weigh(points, Vec2::NAN);
    }

    #[test]
    fn a_pitch_only_set_on_a_line_interpolates_by_pitch_and_clamps_at_the_ends() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.5),
            Vec2::new(0.0, -0.5),
        ];

        assert_pitch_line(&points, 2, 0, 1);
    }

    #[test]
    fn a_point_a_thousandth_of_a_radian_off_the_line_still_counts_as_on_it() {
        let points = [
            Vec2::new(0.0, -0.5),
            Vec2::new(1e-3, 0.0),
            Vec2::new(0.0, 0.5),
        ];

        assert_pitch_line(&points, 0, 1, 2);
    }

    #[test]
    fn a_duplicated_point_is_dropped_and_keeps_a_zero_weight() {
        let points = [
            Vec2::new(0.0, -0.5),
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.5),
            Vec2::new(0.0, 0.0),
            Vec2::new(5e-5, 0.0),
        ];

        assert_pitch_line(&points, 0, 1, 2);
        let weights = weigh(&points, Vec2::new(0.0, 0.0));
        assert_eq!(weights[3], 0.0);
        assert_eq!(weights[4], 0.0);
    }

    #[test]
    fn a_sample_inside_a_triangle_is_weighed_by_its_barycentric_coordinates() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
        ];

        assert_weights(&weigh(&points, Vec2::new(0.25, 0.25)), &[0.5, 0.25, 0.25]);
        assert_weights(&weigh(&points, Vec2::new(0.1, 0.6)), &[0.3, 0.1, 0.6]);
    }

    #[test]
    fn a_single_point_takes_the_whole_weight_wherever_the_sample_is() {
        let points = [Vec2::new(0.2, -0.1)];

        assert_weights(&weigh(&points, Vec2::new(0.2, -0.1)), &[1.0]);
        assert_weights(&weigh(&points, Vec2::new(-3.0, 2.0)), &[1.0]);
    }
}
