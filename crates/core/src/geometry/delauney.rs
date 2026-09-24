use glam::{Vec2, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    pub a: usize,
    pub b: usize,
    pub c: usize,
}

/// Result of [`Triangulation2D::locate`]: the containing triangle and the
/// barycentric coordinates of the queried point within it.
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
/// Indices in `triangles` refer to positions in `points`.
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
    /// to `triangles[triangle_index]`. Coordinates are in `[0, 1]` when the
    /// point is inside the triangle and sum to 1 by construction.
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
    /// When `point` falls outside the triangulation it is projected onto the
    /// nearest triangle boundary by clamping negative barycentric coordinates
    /// to zero and renormalizing, so the returned weights always sum to 1 and
    /// are all non-negative.
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
    /// Prefer this over calling `triangle_containing` + `barycentric_in`
    /// separately because it avoids computing the coordinates twice.
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

    fn sorted(t: &Triangle) -> [usize; 3] {
        let mut idx = [t.a, t.b, t.c];
        idx.sort_unstable();
        idx
    }

    #[test]
    fn single_triangle() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        assert_eq!(t.triangles.len(), 1);
        assert_eq!(sorted(&t.triangles[0]), [0, 1, 2]);
    }

    #[test]
    fn square_gives_two_triangles() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        assert_eq!(t.triangles.len(), 2);
    }

    #[test]
    fn five_points_with_interior() {
        // Square with a center point: convex hull = 4, n = 5
        // Euler formula: T = 2n - h - 2 = 10 - 4 - 2 = 4
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(1.0, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        assert_eq!(t.triangles.len(), 4);
        for tri in &t.triangles {
            assert!(tri.a < 5 && tri.b < 5 && tri.c < 5);
        }
    }

    #[test]
    fn fewer_than_three_points_is_empty() {
        let t = Triangulation2D::build(vec![Vec2::ZERO, Vec2::ONE]);
        assert!(t.triangles.is_empty());
    }

    #[test]
    fn locate_center_of_triangle() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        let centroid = Vec2::new(0.5, 1.0 / 3.0);
        let loc = t.locate(centroid).expect("centroid should be inside");
        assert_eq!(loc.triangle, 0);
        // At the centroid all barycentric coordinates equal 1/3.
        let eps = 1e-5;
        assert!((loc.lambda_a - 1.0 / 3.0).abs() < eps);
        assert!((loc.lambda_b - 1.0 / 3.0).abs() < eps);
        assert!((loc.lambda_c - 1.0 / 3.0).abs() < eps);
    }

    #[test]
    fn locate_at_vertex() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        // Point exactly at vertex a should give λa = 1, λb = 0, λc = 0.
        let tri = &t.triangles[0];
        let va = t.points[tri.a];
        let loc = t.locate(va).expect("vertex should be inside");
        let eps = 1e-5;
        assert!((loc.lambda_a - 1.0).abs() < eps);
        assert!(loc.lambda_b.abs() < eps);
        assert!(loc.lambda_c.abs() < eps);
    }

    #[test]
    fn locate_or_nearest_projects_to_boundary() {
        // Diamond: (0,1) (1,0) (0,-1) (-1,0). Point (1,1) is outside.
        let pts = vec![
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(-1.0, 0.0),
        ];
        let t = Triangulation2D::build(pts.clone());
        let loc = t.locate_or_nearest(Vec2::new(1.0, 1.0));

        // Weights must be valid.
        assert!(loc.lambda_a >= 0.0 && loc.lambda_b >= 0.0 && loc.lambda_c >= 0.0);
        assert!((loc.lambda_a + loc.lambda_b + loc.lambda_c - 1.0).abs() < 1e-5);

        // The reconstructed point is the nearest point on the hull to (1,1),
        // which is the midpoint of the (0,1)-(1,0) edge: (0.5, 0.5).
        let tri = &t.triangles[loc.triangle];
        let reconstructed =
            pts[tri.a] * loc.lambda_a + pts[tri.b] * loc.lambda_b + pts[tri.c] * loc.lambda_c;
        assert!((reconstructed - Vec2::new(0.5, 0.5)).length() < 1e-5);
    }

    #[test]
    fn locate_or_nearest_exact_when_inside() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        let centroid = Vec2::new(0.5, 1.0 / 3.0);
        let via_locate = t.locate(centroid).unwrap();
        let via_nearest = t.locate_or_nearest(centroid);
        assert_eq!(via_locate.triangle, via_nearest.triangle);
        assert!((via_locate.lambda_a - via_nearest.lambda_a).abs() < 1e-6);
        assert!((via_locate.lambda_b - via_nearest.lambda_b).abs() < 1e-6);
        assert!((via_locate.lambda_c - via_nearest.lambda_c).abs() < 1e-6);
    }

    #[test]
    fn locate_outside_returns_none() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.5, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        assert!(t.locate(Vec2::new(5.0, 5.0)).is_none());
    }

    #[test]
    fn barycentric_coords_sum_to_one() {
        let pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(1.0, 1.0),
        ];
        let t = Triangulation2D::build(pts);
        let probe = Vec2::new(0.5, 0.5);
        if let Some(loc) = t.locate(probe) {
            assert!(
                (loc.lambda_a + loc.lambda_b + loc.lambda_c - 1.0).abs() < 1e-5,
                "coords must sum to 1"
            );
            // Cross-check via barycentric_in
            let b = t.barycentric_in(loc.triangle, probe);
            assert!((loc.lambda_a - b.x).abs() < 1e-6);
            assert!((loc.lambda_b - b.y).abs() < 1e-6);
            assert!((loc.lambda_c - b.z).abs() < 1e-6);
        } else {
            panic!("point should be inside the triangulation");
        }
    }
}
