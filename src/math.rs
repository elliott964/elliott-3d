use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V3(pub f64, pub f64, pub f64);

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3(self.0 + o.0, self.1 + o.1, self.2 + o.2)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3(self.0 - o.0, self.1 - o.1, self.2 - o.2)
    }
}
impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, k: f64) -> V3 {
        V3(self.0 * k, self.1 * k, self.2 * k)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3(-self.0, -self.1, -self.2)
    }
}

impl V3 {
    pub const ONE: V3 = V3(1.0, 1.0, 1.0);
    pub fn dot(self, o: V3) -> f64 {
        self.0 * o.0 + self.1 * o.1 + self.2 * o.2
    }
    pub fn cross(self, o: V3) -> V3 {
        V3(
            self.1 * o.2 - self.2 * o.1,
            self.2 * o.0 - self.0 * o.2,
            self.0 * o.1 - self.1 * o.0,
        )
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    /// Unit vector; zero vector stays zero.
    pub fn norm(self) -> V3 {
        let l = self.len();
        if l > 1e-12 { self * (1.0 / l) } else { V3::default() }
    }
    pub fn lerp(self, o: V3, t: f64) -> V3 {
        self + (o - self) * t
    }
    pub fn min(self, o: V3) -> V3 {
        V3(self.0.min(o.0), self.1.min(o.1), self.2.min(o.2))
    }
    pub fn max(self, o: V3) -> V3 {
        V3(self.0.max(o.0), self.1.max(o.1), self.2.max(o.2))
    }
}

type M3 = [[f64; 3]; 3];

fn mat_mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn mat_vec(m: &M3, v: V3) -> V3 {
    V3(
        m[0][0] * v.0 + m[0][1] * v.1 + m[0][2] * v.2,
        m[1][0] * v.0 + m[1][1] * v.1 + m[1][2] * v.2,
        m[2][0] * v.0 + m[2][1] * v.1 + m[2][2] * v.2,
    )
}

/// Affine transform: `p' = m * p + t`.
#[derive(Clone, Copy)]
pub struct Xf {
    m: M3,
    t: V3,
}

impl Xf {
    /// Translate * RotateZYX(euler degrees, X applied first) * Scale.
    pub fn trs(t: V3, rot_deg: V3, s: V3) -> Xf {
        let (sx, cx) = rot_deg.0.to_radians().sin_cos();
        let (sy, cy) = rot_deg.1.to_radians().sin_cos();
        let (sz, cz) = rot_deg.2.to_radians().sin_cos();
        let rx = [[1., 0., 0.], [0., cx, -sx], [0., sx, cx]];
        let ry = [[cy, 0., sy], [0., 1., 0.], [-sy, 0., cy]];
        let rz = [[cz, -sz, 0.], [sz, cz, 0.], [0., 0., 1.]];
        let mut m = mat_mul(&rz, &mat_mul(&ry, &rx));
        for row in &mut m {
            row[0] *= s.0;
            row[1] *= s.1;
            row[2] *= s.2;
        }
        Xf { m, t }
    }

    pub fn point(&self, v: V3) -> V3 {
        mat_vec(&self.m, v) + self.t
    }

    /// `self` is the parent, `child` the local transform.
    pub fn then(&self, child: &Xf) -> Xf {
        Xf { m: mat_mul(&self.m, &child.m), t: self.point(child.t) }
    }

    /// Normal transform (cofactor matrix = det * inverse-transpose), normalized.
    pub fn normal(&self, n: V3) -> V3 {
        let m = &self.m;
        let c = [
            [
                m[1][1] * m[2][2] - m[1][2] * m[2][1],
                -(m[1][0] * m[2][2] - m[1][2] * m[2][0]),
                m[1][0] * m[2][1] - m[1][1] * m[2][0],
            ],
            [
                -(m[0][1] * m[2][2] - m[0][2] * m[2][1]),
                m[0][0] * m[2][2] - m[0][2] * m[2][0],
                -(m[0][0] * m[2][1] - m[0][1] * m[2][0]),
            ],
            [
                m[0][1] * m[1][2] - m[0][2] * m[1][1],
                -(m[0][0] * m[1][2] - m[0][2] * m[1][0]),
                m[0][0] * m[1][1] - m[0][1] * m[1][0],
            ],
        ];
        mat_vec(&c, n).norm()
    }
}
