//! Mesh generators + smooth-normal computation. All primitives fit a unit (1x1x1) box centered at the origin.
use crate::math::V3;
use std::collections::HashMap;
use std::f64::consts::PI;

#[derive(Default)]
pub struct Mesh {
    pub v: Vec<V3>,
    pub t: Vec<[u32; 3]>,
}

impl Mesh {
    /// Push a triangle unless it has (near) zero area.
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        let (p, q, r) = (self.v[a as usize], self.v[b as usize], self.v[c as usize]);
        if (q - p).cross(r - p).len() > 1e-12 {
            self.t.push([a, b, c]);
        }
    }

    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.tri(a, b, c);
        self.tri(a, c, d);
    }

    /// Signed volume; positive when a closed mesh is wound counter-clockwise from outside.
    pub fn volume(&self) -> f64 {
        self.t
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.v[i as usize]);
                a.dot(b.cross(c))
            })
            .sum::<f64>()
            / 6.0
    }

    fn outward(mut self) -> Self {
        if self.volume() < 0.0 {
            for t in &mut self.t {
                t.swap(1, 2);
            }
        }
        self
    }
}

/// Parametric (rows x cols) vertex grid, optionally wrapping in either direction.
fn grid(rows: usize, cols: usize, wrap_rows: bool, wrap_cols: bool, f: impl Fn(usize, usize) -> V3) -> Mesh {
    let mut m = Mesh::default();
    for i in 0..rows {
        for j in 0..cols {
            m.v.push(f(i, j));
        }
    }
    let id = |i: usize, j: usize| (i * cols + j) as u32;
    let ri = if wrap_rows { rows } else { rows - 1 };
    let cj = if wrap_cols { cols } else { cols - 1 };
    for i in 0..ri {
        for j in 0..cj {
            let (i1, j1) = ((i + 1) % rows, (j + 1) % cols);
            m.quad(id(i, j), id(i, j1), id(i1, j1), id(i1, j));
        }
    }
    m
}

fn face(m: &mut Mesh, n: V3, u: V3, v: V3) {
    let c = n * 0.5;
    let o = m.v.len() as u32;
    for (a, b) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
        m.v.push(c + u * (a * 0.5) + v * (b * 0.5));
    }
    m.quad(o, o + 1, o + 2, o + 3);
}

pub fn cube() -> Mesh {
    let (x, y, z) = (V3(1., 0., 0.), V3(0., 1., 0.), V3(0., 0., 1.));
    let mut m = Mesh::default();
    for (n, u, v) in [(x, y, z), (-x, z, y), (y, z, x), (-y, x, z), (z, x, y), (-z, y, x)] {
        face(&mut m, n, u, v);
    }
    m
}

/// 1x1 quad in the XZ plane, normal +Y.
pub fn plane() -> Mesh {
    let mut m = Mesh::default();
    face(&mut m, V3(0., 1., 0.), V3(0., 0., 1.), V3(1., 0., 0.));
    m
}

pub fn sphere(seg: usize) -> Mesh {
    let rings = (seg / 2).max(2);
    grid(rings + 1, seg, false, true, |i, j| {
        let (phi, th) = (PI * i as f64 / rings as f64, 2.0 * PI * j as f64 / seg as f64);
        V3(0.5 * phi.sin() * th.cos(), 0.5 * phi.cos(), 0.5 * phi.sin() * th.sin())
    })
    .outward()
}

fn frustum(seg: usize, r_top: f64) -> Mesh {
    let mut m = grid(2, seg, false, true, |i, j| {
        let th = 2.0 * PI * j as f64 / seg as f64;
        let (r, y) = if i == 0 { (0.5, -0.5) } else { (r_top, 0.5) };
        V3(r * th.cos(), y, r * th.sin())
    });
    let (bc, tc) = (m.v.len() as u32, m.v.len() as u32 + 1);
    m.v.push(V3(0., -0.5, 0.));
    m.v.push(V3(0., 0.5, 0.));
    for j in 0..seg {
        let (a, b) = (j as u32, ((j + 1) % seg) as u32);
        m.tri(bc, b, a);
        if r_top > 0.0 {
            m.tri(tc, seg as u32 + a, seg as u32 + b);
        }
    }
    m.outward()
}

pub fn cylinder(seg: usize) -> Mesh {
    frustum(seg, 0.5)
}

pub fn cone(seg: usize) -> Mesh {
    frustum(seg, 0.0)
}

/// Outer diameter 1; `ratio` = tube radius / ring radius.
pub fn torus(seg: usize, ratio: f64) -> Mesh {
    let big = 0.5 / (1.0 + ratio);
    let small = big * ratio;
    let minor = (seg / 2).max(3);
    grid(seg, minor, true, true, |i, j| {
        let (t, p) = (2.0 * PI * i as f64 / seg as f64, 2.0 * PI * j as f64 / minor as f64);
        let r = big + small * p.cos();
        V3(r * t.cos(), small * p.sin(), r * t.sin())
    })
    .outward()
}

/// Revolve a `[radius, y]` profile around the Y axis.
pub fn lathe(profile: &[[f64; 2]], seg: usize) -> Result<Mesh, String> {
    if profile.len() < 2 {
        return Err("lathe profile needs at least 2 points".into());
    }
    Ok(grid(profile.len(), seg, false, true, |i, j| {
        let th = 2.0 * PI * j as f64 / seg as f64;
        V3(profile[i][0] * th.cos(), profile[i][1], profile[i][0] * th.sin())
    }))
}

/// Extrude a simple (non self-intersecting, hole-free) XY polygon from z=0 to z=depth.
pub fn extrude(profile: &[[f64; 2]], depth: f64) -> Result<Mesh, String> {
    let mut p = profile.to_vec();
    if p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
    if p.len() < 3 {
        return Err("extrude profile needs at least 3 distinct points".into());
    }
    let n = p.len();
    let area: f64 = (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum();
    if area < 0.0 {
        p.reverse();
    }
    let mut m = Mesh::default();
    for z in [depth, 0.0] {
        for q in &p {
            m.v.push(V3(q[0], q[1], z));
        }
    }
    for [a, b, c] in triangulate(&p) {
        m.tri(a, b, c); // front cap, +Z
        m.tri(n as u32 + a, n as u32 + c, n as u32 + b); // back cap, -Z
    }
    for i in 0..n {
        let j = (i + 1) % n;
        m.quad((n + i) as u32, (n + j) as u32, j as u32, i as u32);
    }
    Ok(m.outward())
}

/// Ear clipping for a CCW polygon.
fn triangulate(p: &[[f64; 2]]) -> Vec<[u32; 3]> {
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let mut idx: Vec<usize> = (0..p.len()).collect();
    let mut out = vec![];
    while idx.len() > 3 {
        let m = idx.len();
        let ear = (0..m).find(|&k| {
            let (a, b, c) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            cross(p[a], p[b], p[c]) > 1e-12
                && !idx.iter().any(|&q| {
                    q != a
                        && q != b
                        && q != c
                        && cross(p[a], p[b], p[q]) >= -1e-12
                        && cross(p[b], p[c], p[q]) >= -1e-12
                        && cross(p[c], p[a], p[q]) >= -1e-12
                })
        });
        match ear {
            Some(k) => {
                out.push([idx[(k + m - 1) % m] as u32, idx[k] as u32, idx[(k + 1) % m] as u32]);
                idx.remove(k);
            }
            None => break, // degenerate input: fan the remainder
        }
    }
    for k in 1..idx.len().saturating_sub(1) {
        out.push([idx[0] as u32, idx[k] as u32, idx[k + 1] as u32]);
    }
    out
}

/// Per-triangle-corner normals. Faces meeting at the same position are averaged (area-weighted)
/// when their angle is within `crease_deg`; `crease_deg <= 0` gives flat shading.
pub fn corner_normals(v: &[V3], t: &[[u32; 3]], crease_deg: f64) -> Vec<[V3; 3]> {
    let raw: Vec<V3> = t
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| v[i as usize]);
            (b - a).cross(c - a)
        })
        .collect();
    let unit: Vec<V3> = raw.iter().map(|n| n.norm()).collect();
    if crease_deg <= 0.0 {
        return unit.iter().map(|&n| [n; 3]).collect();
    }
    let key = |p: V3| ((p.0 * 1e5).round() as i64, (p.1 * 1e5).round() as i64, (p.2 * 1e5).round() as i64);
    let mut groups: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (f, t) in t.iter().enumerate() {
        for &i in t {
            groups.entry(key(v[i as usize])).or_default().push(f);
        }
    }
    let cos = crease_deg.to_radians().cos();
    t.iter()
        .enumerate()
        .map(|(f, t)| {
            t.map(|i| {
                let mut sum = V3::default();
                for &g in &groups[&key(v[i as usize])] {
                    if unit[g].dot(unit[f]) >= cos {
                        sum = sum + raw[g];
                    }
                }
                let n = sum.norm();
                if n == V3::default() { unit[f] } else { n }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed, consistently oriented, outward-facing: every directed edge appears once with its reverse once.
    fn assert_closed_outward(name: &str, m: &Mesh) {
        assert!(m.volume() > 1e-6, "{name}: volume {}", m.volume());
        let key = |p: V3| ((p.0 * 1e5).round() as i64, (p.1 * 1e5).round() as i64, (p.2 * 1e5).round() as i64);
        let mut edges: HashMap<_, i32> = HashMap::new();
        for t in &m.t {
            for k in 0..3 {
                let (a, b) = (key(m.v[t[k] as usize]), key(m.v[t[(k + 1) % 3] as usize]));
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        for (&(a, b), &n) in &edges {
            assert_eq!(n, 1, "{name}: directed edge used {n}x");
            assert_eq!(edges.get(&(b, a)), Some(&1), "{name}: edge without opposite twin");
        }
    }

    #[test]
    fn primitives_are_closed_and_outward() {
        assert_closed_outward("cube", &cube());
        assert_closed_outward("sphere", &sphere(16));
        assert_closed_outward("cylinder", &cylinder(12));
        assert_closed_outward("cone", &cone(12));
        assert_closed_outward("torus", &torus(16, 0.35));
        let l_shape = [[0., 0.], [2., 0.], [2., 1.], [1., 1.], [1., 2.], [0., 2.]];
        let m = extrude(&l_shape, 0.5).unwrap();
        assert_closed_outward("extrude", &m);
        assert!((m.volume() - 1.5).abs() < 1e-9, "L-shape volume {}", m.volume());
        let cw: Vec<_> = l_shape.iter().rev().copied().collect();
        assert!((extrude(&cw, 0.5).unwrap().volume() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn crease_keeps_cube_edges_hard_but_smooths_sphere() {
        let c = cube();
        for n in corner_normals(&c.v, &c.t, 40.0).iter().flatten() {
            assert!([n.0, n.1, n.2].iter().any(|a| a.abs() > 0.999), "cube normal not axis-aligned: {n:?}");
        }
        let s = sphere(32);
        for (t, ns) in s.t.iter().zip(corner_normals(&s.v, &s.t, 40.0)) {
            for k in 0..3 {
                let radial = s.v[t[k] as usize].norm();
                assert!(radial.dot(ns[k]) > 0.99, "sphere normal off radial");
            }
        }
    }
}
