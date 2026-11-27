use glam::{Vec2, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    pub a: usize,
    pub b: usize,
    pub c: usize,
}

/// A point located in a [`Triangulation2D`].
///
/// Holds the containing triangle and the barycentric coordinates of the
/// queried point within it, as [`Triangulation2D::locate`] returns them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangulatedPoint2D {
    /// Index into [`Triangulation2D::triangles`].
    pub triangle: usize,
    /// Weight of vertex `a` (sums to 1 with `lambda_b` and `lambda_c`).
    pub lambda_a: f32,
    /// Weight of vertex `b`.
    pub lambda_b: f32,
    /// Weight of vertex `c`.
    pub lambda_c: f32,
}

/// A completed Delaunay triangulation over a fixed set of 2D points.
///
/// The indices in each [`Triangle`] refer to positions in
/// [`points`](Self::points).
pub struct Triangulation2D {
    points: Vec<Vec2>,
    triangles: Vec<Triangle>,
}

impl Triangulation2D {
    /// Runs Bowyer-Watson on `points` and freezes the result.
    pub fn build(points: Vec<Vec2>) -> Self {
        let triangles = triangulate(&points);
        Self { points, triangles }
    }

    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    pub fn triangles(&self) -> &[Triangle] {
        &self.triangles
    }

    /// Returns the index of the triangle containing `point`, or `None` if it
    /// falls outside the triangulation.
    pub fn triangle_containing(&self, point: Vec2) -> Option<usize> {
        self.triangles.iter().position(|tri| {
            let b = barycentric(
                self.points[tri.a],
                self.points[tri.b],
                self.points[tri.c],
                point,
            );
            b.cmpge(Vec3::ZERO).all()
        })
    }

    /// Returns the barycentric coordinates `(λa, λb, λc)` of `point` relative
    /// to the triangle at `triangle_index`.
    ///
    /// The coordinates sum to 1 by construction, and all lie in `[0, 1]` when
    /// the point is inside the triangle.
    ///
    /// # Panics
    ///
    /// Panics if `triangle_index` is out of bounds.
    pub fn barycentric_in(&self, triangle_index: usize, point: Vec2) -> Vec3 {
        let tri = &self.triangles[triangle_index];
        barycentric(
            self.points[tri.a],
            self.points[tri.b],
            self.points[tri.c],
            point,
        )
    }

    /// Like [`locate`](Self::locate), but always returns a result.
    ///
    /// When `point` falls outside the triangulation it is replaced by the
    /// geometrically nearest point on any triangle's boundary, so the returned
    /// weights sum to 1 and are all non-negative.
    pub fn locate_or_nearest(&self, point: Vec2) -> TriangulatedPoint2D {
        if let Some(loc) = self.locate(point) {
            return loc;
        }

        let mut best_triangle = 0;
        let mut best_weights = Vec3::ZERO;
        let mut best_dist_sq = f32::INFINITY;

        for (i, tri) in self.triangles.iter().enumerate() {
            let (a, b, c) = (self.points[tri.a], self.points[tri.b], self.points[tri.c]);
            let (weights, dist_sq) = nearest_on_triangle(a, b, c, point);
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_triangle = i;
                best_weights = weights;
            }
        }

        TriangulatedPoint2D {
            triangle: best_triangle,
            lambda_a: best_weights.x,
            lambda_b: best_weights.y,
            lambda_c: best_weights.z,
        }
    }

    /// Finds the triangle containing `point` and returns a [`TriangulatedPoint2D`]
    /// with its index and barycentric coordinates, or `None` if the point is
    /// outside the triangulation.
    ///
    /// Prefer this over calling [`triangle_containing`](Self::triangle_containing)
    /// and [`barycentric_in`](Self::barycentric_in) separately, because it
    /// avoids computing the coordinates twice.
    pub fn locate(&self, point: Vec2) -> Option<TriangulatedPoint2D> {
        for (i, tri) in self.triangles.iter().enumerate() {
            let b = barycentric(
                self.points[tri.a],
                self.points[tri.b],
                self.points[tri.c],
                point,
            );
            if b.cmpge(Vec3::ZERO).all() {
                return Some(TriangulatedPoint2D {
                    triangle: i,
                    lambda_a: b.x,
                    lambda_b: b.y,
                    lambda_c: b.z,
                });
            }
        }
        None
    }
}

// Barycentric coordinates of p with respect to triangle (a, b, c).
// Returns (λa, λb, λc) with λa + λb + λc = 1.  All three are in [0, 1] when p
// is inside the triangle; negative values indicate p is outside that edge.
fn barycentric(a: Vec2, b: Vec2, c: Vec2, p: Vec2) -> Vec3 {
    let denom = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    let la = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / denom;
    let lb = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / denom;
    Vec3::new(la, lb, 1.0 - la - lb)
}

// Nearest point on triangle (a, b, c) to p, expressed as barycentric coordinates,
// plus the squared distance from p to that point.
// When p is outside, projects onto each edge (clamping to segment endpoints) and
// returns the closest projection. This gives the true geometric nearest point,
// unlike clamping barycentric coords directly which is only an approximation.
fn nearest_on_triangle(a: Vec2, b: Vec2, c: Vec2, p: Vec2) -> (Vec3, f32) {
    let raw = barycentric(a, b, c, p);
    if raw.cmpge(Vec3::ZERO).all() {
        return (raw, 0.0);
    }

    let mut best = Vec2::ZERO;
    let mut best_dist_sq = f32::INFINITY;

    for (ea, eb) in [(a, b), (b, c), (c, a)] {
        let edge = eb - ea;
        let len_sq = edge.dot(edge);
        let t = if len_sq < 1e-10 {
            0.0
        } else {
            ((p - ea).dot(edge) / len_sq).clamp(0.0, 1.0)
        };
        let pt = ea + t * edge;
        let d = p.distance_squared(pt);
        if d < best_dist_sq {
            best_dist_sq = d;
            best = pt;
        }
    }

    (barycentric(a, b, c, best), best_dist_sq)
}

// Returns true if `d` is strictly inside the circumcircle of CCW-ordered triangle (a, b, c).
// Computed in f64 to avoid precision loss from squaring f32 coordinates.
fn in_circumcircle(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let ax = (a.x - d.x) as f64;
    let ay = (a.y - d.y) as f64;
    let bx = (b.x - d.x) as f64;
    let by = (b.y - d.y) as f64;
    let cx = (c.x - d.x) as f64;
    let cy = (c.y - d.y) as f64;

    let det = ax * (by * (cx * cx + cy * cy) - cy * (bx * bx + by * by))
        - ay * (bx * (cx * cx + cy * cy) - cx * (bx * bx + by * by))
        + (ax * ax + ay * ay) * (bx * cy - by * cx);

    det > 0.0
}

// Reorders triangle vertices so they wind counter-clockwise.
fn ccw(pts: &[Vec2], a: usize, b: usize, c: usize) -> [usize; 3] {
    let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
    let cross = (pb.x - pa.x) * (pc.y - pa.y) - (pb.y - pa.y) * (pc.x - pa.x);
    if cross >= 0.0 {
        [a, b, c]
    } else {
        [a, c, b]
    }
}

fn triangulate(points: &[Vec2]) -> Vec<Triangle> {
    if points.len() < 3 {
        return vec![];
    }

    let n = points.len();

    let min_x = points.iter().fold(f32::INFINITY, |acc, p| acc.min(p.x));
    let max_x = points.iter().fold(f32::NEG_INFINITY, |acc, p| acc.max(p.x));
    let min_y = points.iter().fold(f32::INFINITY, |acc, p| acc.min(p.y));
    let max_y = points.iter().fold(f32::NEG_INFINITY, |acc, p| acc.max(p.y));

    let delta = ((max_x - min_x).max(max_y - min_y)).max(1.0) * 10.0;
    let mid = Vec2::new((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);

    // Append super-triangle vertices at indices n, n+1, n+2.
    let mut pts: Vec<Vec2> = points.to_vec();
    pts.push(Vec2::new(mid.x - 2.0 * delta, mid.y - delta));
    pts.push(Vec2::new(mid.x, mid.y + 2.0 * delta));
    pts.push(Vec2::new(mid.x + 2.0 * delta, mid.y - delta));

    let mut triangles: Vec<[usize; 3]> = vec![ccw(&pts, n, n + 1, n + 2)];

    for i in 0..n {
        let p = pts[i];

        // Collect indices of triangles whose circumcircle contains p.
        let mut bad: Vec<usize> = vec![];
        for (ti, &tri) in triangles.iter().enumerate() {
            if in_circumcircle(pts[tri[0]], pts[tri[1]], pts[tri[2]], p) {
                bad.push(ti);
            }
        }

        // Collect cavity boundary: edges not shared between two bad triangles.
        let mut boundary: Vec<(usize, usize)> = vec![];
        for &bi in &bad {
            let tri = triangles[bi];
            for &(ea, eb) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let shared = bad.iter().any(|&bj| {
                    if bj == bi {
                        return false;
                    }
                    let o = triangles[bj];
                    (o[0] == ea || o[1] == ea || o[2] == ea)
                        && (o[0] == eb || o[1] == eb || o[2] == eb)
                });
                if !shared {
                    boundary.push((ea, eb));
                }
            }
        }

        // Remove bad triangles in descending index order so swap_remove stays valid.
        bad.sort_unstable();
        for &bi in bad.iter().rev() {
            triangles.swap_remove(bi);
        }

        // Re-triangulate the cavity.
        for (ea, eb) in boundary {
            triangles.push(ccw(&pts, ea, eb, i));
        }
    }

    // Discard any triangle that touches a super-triangle vertex.
    triangles.retain(|tri| tri[0] < n && tri[1] < n && tri[2] < n);

    triangles
        .iter()
        .map(|&[a, b, c]| Triangle { a, b, c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    struct Count {
        points: Vec<Vec2>,
        triangles: usize,
    }

    #[test]
    fn a_triangulation_has_two_n_minus_hull_minus_two_triangles() {
        let cases = [
            Count {
                points: vec![Vec2::ZERO, Vec2::ONE],
                triangles: 0,
            },
            Count {
                points: square(),
                triangles: 4,
            },
        ];
        for case in cases {
            let count = case.points.len();
            let triangulation = Triangulation2D::build(case.points);
            assert_eq!(triangulation.triangles().len(), case.triangles);
            for triangle in triangulation.triangles() {
                let mut corners = [triangle.a, triangle.b, triangle.c];
                corners.sort_unstable();
                assert!(corners[0] < corners[1] && corners[1] < corners[2] && corners[2] < count);
            }
        }
    }

    fn square() -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(1.0, 1.0),
        ]
    }

    fn rebuilt(triangulation: &Triangulation2D, located: &TriangulatedPoint2D) -> Vec2 {
        let triangle = &triangulation.triangles()[located.triangle];
        let points = triangulation.points();
        points[triangle.a] * located.lambda_a
            + points[triangle.b] * located.lambda_b
            + points[triangle.c] * located.lambda_c
    }

    fn assert_weights(located: &TriangulatedPoint2D) -> Result<(), TestCaseError> {
        let weights = [located.lambda_a, located.lambda_b, located.lambda_c];
        prop_assert!(
            weights.iter().all(|weight| *weight >= -1.0e-5),
            "{weights:?}"
        );
        prop_assert!(
            (weights.iter().sum::<f32>() - 1.0).abs() < 1.0e-5,
            "{weights:?}"
        );
        Ok(())
    }

    fn inside() -> impl Strategy<Value = Vec2> {
        prop_oneof![
            proptest::sample::select(square()),
            (0.0f32..=2.0).prop_flat_map(|x| (0.0f32..=2.0).prop_map(move |y| Vec2::new(x, y))),
        ]
    }

    fn outside() -> impl Strategy<Value = Vec2> {
        (-3.0f32..5.0)
            .prop_flat_map(|x| (-3.0f32..5.0).prop_map(move |y| Vec2::new(x, y)))
            .prop_filter("outside the square", |point| {
                point.x < -0.01 || point.x > 2.01 || point.y < -0.01 || point.y > 2.01
            })
    }

    proptest! {
        #[test]
        fn a_point_inside_locates_to_weights_that_rebuild_it(point in inside()) {
            let triangulation = Triangulation2D::build(square());
            let located = triangulation.locate(point).expect("inside");
            assert_weights(&located)?;
            prop_assert!(rebuilt(&triangulation, &located).distance(point) < 1.0e-4);
            let barycentric = triangulation.barycentric_in(located.triangle, point);
            prop_assert!(
                barycentric.abs_diff_eq(Vec3::new(located.lambda_a, located.lambda_b, located.lambda_c), 1.0e-6)
            );
            let nearest = triangulation.locate_or_nearest(point);
            prop_assert!(rebuilt(&triangulation, &nearest).distance(point) < 1.0e-4);
        }

        #[test]
        fn a_point_outside_is_not_located_and_its_nearest_is_on_the_hull(point in outside()) {
            let triangulation = Triangulation2D::build(square());
            prop_assert!(triangulation.locate(point).is_none());
            let nearest = triangulation.locate_or_nearest(point);
            assert_weights(&nearest)?;
            let on_hull = point.clamp(Vec2::ZERO, Vec2::splat(2.0));
            prop_assert!(rebuilt(&triangulation, &nearest).distance(on_hull) < 1.0e-4);
        }
    }
}
