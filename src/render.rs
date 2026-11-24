//! Software rasterizer: z-buffer, perspective-correct per-pixel Blinn-Phong, supersampled AA, ground grid.
use crate::geom::corner_normals;
use crate::math::V3;
use crate::scene::Scene;

pub struct Cam {
    pub pos: V3,
    pub target: V3,
    /// Vertical field of view, degrees.
    pub fov: f64,
}

pub struct Opts {
    pub w: usize,
    pub h: usize,
    /// sRGB 0..1.
    pub bg: [f64; 3],
    /// Grid line spacing; `None` disables the grid.
    pub grid: Option<f64>,
    /// Supersampling factor per axis.
    pub ss: usize,
}

pub const VIEWS: [&str; 7] = ["front", "back", "left", "right", "top", "bottom", "iso"];

pub fn center_radius(bounds: Option<(V3, V3)>) -> (V3, f64) {
    match bounds {
        Some((lo, hi)) => ((lo + hi) * 0.5, ((hi - lo).len() * 0.5).max(1e-3)),
        None => (V3::default(), 3.0),
    }
}

/// Grid spacing as a power of ten matched to scene size.
pub fn auto_spacing(radius: f64) -> f64 {
    10f64.powf((radius / 2.0).log10().round())
}

/// Camera that frames the scene's bounding sphere from a named direction.
pub fn frame(view: &str, bounds: Option<(V3, V3)>, fov: f64, aspect: f64) -> Result<Cam, String> {
    let dir = match view {
        "front" => V3(0., 0., 1.),
        "back" => V3(0., 0., -1.),
        "right" => V3(1., 0., 0.),
        "left" => V3(-1., 0., 0.),
        "top" => V3(0., 1., 0.),
        "bottom" => V3(0., -1., 0.),
        "iso" => V3(1., 0.75, 1.).norm(),
        v => return Err(format!("unknown view '{v}'; use one of {VIEWS:?} or \"all\"")),
    };
    let (c, r) = center_radius(bounds);
    let half = (fov.to_radians() / 2.0).tan();
    let half = half.min(half * aspect); // limiting axis
    let dist = 1.05 * r * (1.0 + half * half).sqrt() / half;
    Ok(Cam { pos: c + dir * dist, target: c, fov })
}

/// (right, up, forward). Looking straight up/down uses -Z/+Z as the up hint.
fn basis(c: &Cam) -> (V3, V3, V3) {
    let fwd = (c.target - c.pos).norm();
    let hint = if fwd.1.abs() > 0.999 { V3(0., 0., if fwd.1 < 0. { -1. } else { 1. }) } else { V3(0., 1., 0.) };
    let right = fwd.cross(hint).norm();
    (right, right.cross(fwd), fwd)
}

fn lin(c: [f64; 3]) -> [f64; 3] {
    c.map(|x| x.powf(2.2))
}

#[derive(Clone, Copy)]
struct Vx {
    p: V3,
    n: V3,
}

struct Mat {
    base: [f64; 3],
    spec: f64,
    emis: f64,
}

struct Ctx<'a> {
    cam: &'a Cam,
    right: V3,
    up: V3,
    fwd: V3,
    f: f64,
    aspect: f64,
    w: usize,
    h: usize,
    col: Vec<[f64; 3]>,
    zb: Vec<f64>,
    key: V3,
    fill: V3,
}

const NEAR: f64 = 1e-4;

impl Ctx<'_> {
    fn depth(&self, p: V3) -> f64 {
        (p - self.cam.pos).dot(self.fwd)
    }

    /// World point -> (pixel x, pixel y, view depth).
    fn proj(&self, p: V3) -> (f64, f64, f64) {
        let d = p - self.cam.pos;
        let (x, y, z) = (d.dot(self.right), d.dot(self.up), d.dot(self.fwd));
        (
            (self.f * x / (z * self.aspect) * 0.5 + 0.5) * self.w as f64,
            (0.5 - 0.5 * self.f * y / z) * self.h as f64,
            z,
        )
    }

    fn clip(&self, v: [Vx; 3]) -> Vec<Vx> {
        let mut out = Vec::with_capacity(4);
        for i in 0..3 {
            let (a, b) = (v[i], v[(i + 1) % 3]);
            let (za, zb) = (self.depth(a.p), self.depth(b.p));
            if za >= NEAR {
                out.push(a);
            }
            if (za >= NEAR) != (zb >= NEAR) {
                let t = (NEAR - za) / (zb - za);
                out.push(Vx { p: a.p.lerp(b.p, t), n: a.n.lerp(b.n, t) });
            }
        }
        out
    }

    fn shade(&self, p: V3, n: V3, m: &Mat) -> [f64; 3] {
        let v = (self.cam.pos - p).norm();
        let mut n = n.norm();
        if n.dot(v) < 0.0 {
            n = -n; // double-sided
        }
        let amb = 0.2 + 0.18 * (n.1 * 0.5 + 0.5);
        let lit = amb + 0.75 * n.dot(self.key).max(0.0) + 0.25 * n.dot(self.fill).max(0.0);
        let spec = n.dot((self.key + v).norm()).max(0.0).powf(48.0) * m.spec;
        std::array::from_fn(|k| (m.base[k] * lit + spec) * (1.0 - m.emis) + m.base[k] * m.emis)
    }

    fn tri(&mut self, a: &Vx, b: &Vx, c: &Vx, m: &Mat) {
        let ((ax, ay, az), (bx, by, bz), (cx, cy, cz)) = (self.proj(a.p), self.proj(b.p), self.proj(c.p));
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area.abs() < 1e-12 {
            return;
        }
        let x0 = ax.min(bx).min(cx).floor().max(0.0);
        let x1 = ax.max(bx).max(cx).ceil().min(self.w as f64 - 1.0);
        let y0 = ay.min(by).min(cy).floor().max(0.0);
        let y1 = ay.max(by).max(cy).ceil().min(self.h as f64 - 1.0);
        if x0 > x1 || y0 > y1 {
            return;
        }
        let (iza, izb, izc) = (1.0 / az, 1.0 / bz, 1.0 / cz);
        for y in y0 as usize..=y1 as usize {
            for x in x0 as usize..=x1 as usize {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((bx - px) * (cy - py) - (by - py) * (cx - px)) / area;
                let w1 = ((cx - px) * (ay - py) - (cy - py) * (ax - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let z = 1.0 / (w0 * iza + w1 * izb + w2 * izc);
                let i = y * self.w + x;
                if z >= self.zb[i] {
                    continue;
                }
                let (q0, q1, q2) = (w0 * iza * z, w1 * izb * z, w2 * izc * z);
                self.zb[i] = z;
                self.col[i] = self.shade(a.p * q0 + b.p * q1 + c.p * q2, a.n * q0 + b.n * q1 + c.n * q2, m);
            }
        }
    }

    /// Anti-aliased lines on the y=0 plane, drawn only where nothing nearer was rasterized.
    fn ground_grid(&mut self, s: f64) {
        let pu = 2.0 / (self.f * self.h as f64); // view-plane units per sample
        let (gray, red, blue) = ([0.5; 3], [0.9, 0.2, 0.2], [0.25, 0.4, 0.95]);
        let blend = |c: &mut [f64; 3], to: [f64; 3], a: f64| {
            for k in 0..3 {
                c[k] += (to[k] - c[k]) * a;
            }
        };
        for y in 0..self.h {
            for x in 0..self.w {
                let nx = (x as f64 + 0.5) / self.w as f64 * 2.0 - 1.0;
                let ny = 1.0 - (y as f64 + 0.5) / self.h as f64 * 2.0;
                let d = self.fwd + self.right * (nx * self.aspect / self.f) + self.up * (ny / self.f);
                if d.1.abs() < 1e-9 {
                    continue;
                }
                let t = -self.cam.pos.1 / d.1; // d has unit view-depth, so t is view depth
                let i = y * self.w + x;
                if t <= 0.0 || t >= self.zb[i] {
                    continue;
                }
                let (hx, hz) = (self.cam.pos.0 + d.0 * t, self.cam.pos.2 + d.2 * t);
                let fp = t * pu * d.len() / d.1.abs(); // ground footprint of one sample
                let fade = (1.0 - fp / (0.5 * s)).clamp(0.0, 1.0);
                if fade <= 0.0 {
                    continue;
                }
                let hw = fp.max(0.012 * s);
                let line = |c: f64| (1.0 - ((c / s - (c / s).round()).abs() * s) / hw).clamp(0.0, 1.0);
                let axis = |c: f64| (1.0 - c.abs() / (hw * 1.6)).clamp(0.0, 1.0);
                let px = &mut self.col[i];
                blend(px, gray, line(hx).max(line(hz)) * 0.3 * fade);
                blend(px, red, axis(hz) * 0.8 * fade);
                blend(px, blue, axis(hx) * 0.8 * fade);
            }
        }
    }
}

/// Render to tightly packed RGB8.
pub fn render(scene: &Scene, cam: &Cam, o: &Opts) -> Vec<u8> {
    let (w, h) = (o.w * o.ss, o.h * o.ss);
    let (right, up, fwd) = basis(cam);
    let mut cx = Ctx {
        cam,
        right,
        up,
        fwd,
        f: 1.0 / (cam.fov.to_radians() / 2.0).tan(),
        aspect: w as f64 / h as f64,
        w,
        h,
        col: vec![lin(o.bg); w * h],
        zb: vec![f64::INFINITY; w * h],
        // Lights ride with the camera: key from upper-left, fill from lower-right.
        key: (right * -0.5 + up * 0.7 - fwd * 0.6).norm(),
        fill: (right * 0.7 - up * 0.2 - fwd * 0.5).norm(),
    };
    for (i, ob) in scene.objects.iter().enumerate().filter(|(_, o)| !o.tris.is_empty()) {
        let xf = scene.world(i);
        let normals = corner_normals(&ob.verts, &ob.tris, ob.crease);
        let wp: Vec<V3> = ob.verts.iter().map(|&v| xf.point(v)).collect();
        let m = Mat { base: lin(ob.color), spec: ob.specular, emis: ob.emissive };
        for (t, n) in ob.tris.iter().zip(&normals) {
            let vx: [Vx; 3] = std::array::from_fn(|k| Vx { p: wp[t[k] as usize], n: xf.normal(n[k]) });
            let poly = cx.clip(vx);
            for k in 1..poly.len().saturating_sub(1) {
                cx.tri(&poly[0], &poly[k], &poly[k + 1], &m);
            }
        }
    }
    if let Some(s) = o.grid {
        cx.ground_grid(s);
    }
    let ss = o.ss;
    let inv = 1.0 / (ss * ss) as f64;
    let mut out = Vec::with_capacity(o.w * o.h * 3);
    for y in 0..o.h {
        for x in 0..o.w {
            let mut sum = [0.0; 3];
            for dy in 0..ss {
                for dx in 0..ss {
                    let c = cx.col[(y * ss + dy) * w + x * ss + dx];
                    for k in 0..3 {
                        sum[k] += c[k];
                    }
                }
            }
            out.extend(sum.map(|s| ((s * inv).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8));
        }
    }
    out
}

pub fn encode_png(w: usize, h: usize, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgb).map_err(|e| e.to_string())?;
    wr.finish().map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                s.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}
