use crate::math::{V3, Xf};
use serde::{Deserialize, Serialize};
use std::fmt::Write;

fn one() -> V3 {
    V3::ONE
}
fn default_color() -> [f64; 3] {
    [0.78, 0.8, 0.83]
}
fn default_specular() -> f64 {
    0.25
}
fn default_crease() -> f64 {
    40.0
}

/// A named node: optional mesh + local TRS transform + material. Empty meshes act as groups.
#[derive(Clone, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub verts: Vec<V3>,
    #[serde(default)]
    pub tris: Vec<[u32; 3]>,
    #[serde(default)]
    pub position: V3,
    /// Euler XYZ, degrees.
    #[serde(default)]
    pub rotation: V3,
    #[serde(default = "one")]
    pub scale: V3,
    /// sRGB 0..1.
    #[serde(default = "default_color")]
    pub color: [f64; 3],
    #[serde(default = "default_specular")]
    pub specular: f64,
    #[serde(default)]
    pub emissive: f64,
    /// Max angle (deg) between faces that still get smooth-shaded together; 0 = flat.
    #[serde(default = "default_crease")]
    pub crease: f64,
}

impl Object {
    pub fn new(name: String, verts: Vec<V3>, tris: Vec<[u32; 3]>) -> Object {
        Object {
            name,
            parent: None,
            verts,
            tris,
            position: V3::default(),
            rotation: V3::default(),
            scale: V3::ONE,
            color: default_color(),
            specular: default_specular(),
            emissive: 0.0,
            crease: default_crease(),
        }
    }

    fn local(&self) -> Xf {
        Xf::trs(self.position, self.rotation, self.scale)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Scene {
    pub objects: Vec<Object>,
}

impl Scene {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.objects.iter().position(|o| o.name == name)
    }

    /// `base` if free, else `base_2`, `base_3`, ...
    pub fn unique(&self, base: &str) -> String {
        if self.find(base).is_none() {
            return base.to_string();
        }
        (2..).map(|k| format!("{base}_{k}")).find(|n| self.find(n).is_none()).unwrap()
    }

    pub fn world(&self, i: usize) -> Xf {
        let mut cur = &self.objects[i];
        let mut xf = cur.local();
        for _ in 0..64 {
            let Some(pi) = cur.parent.as_deref().and_then(|p| self.find(p)) else { break };
            cur = &self.objects[pi];
            xf = cur.local().then(&xf);
        }
        xf
    }

    /// Root followed by all descendants (parents always precede children).
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        let mut k = 0;
        while k < out.len() {
            let name = &self.objects[out[k]].name;
            for (i, o) in self.objects.iter().enumerate() {
                if o.parent.as_deref() == Some(name) && !out.contains(&i) {
                    out.push(i);
                }
            }
            k += 1;
        }
        out
    }

    /// Would parenting `child` under `parent` create a cycle?
    pub fn cycles(&self, child: &str, parent: &str) -> bool {
        let mut cur = Some(parent);
        for _ in 0..=self.objects.len() {
            match cur {
                Some(c) if c == child => return true,
                Some(c) => cur = self.find(c).and_then(|i| self.objects[i].parent.as_deref()),
                None => return false,
            }
        }
        true
    }

    pub fn object_bounds(&self, i: usize) -> Option<(V3, V3)> {
        let xf = self.world(i);
        let mut it = self.objects[i].verts.iter().map(|&v| xf.point(v));
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    pub fn bounds(&self) -> Option<(V3, V3)> {
        (0..self.objects.len())
            .filter_map(|i| self.object_bounds(i))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    pub fn validate(&self) -> Result<(), String> {
        for (i, o) in self.objects.iter().enumerate() {
            if self.objects[..i].iter().any(|p| p.name == o.name) {
                return Err(format!("duplicate object name '{}'", o.name));
            }
            if o.tris.iter().flatten().any(|&k| k as usize >= o.verts.len()) {
                return Err(format!("object '{}' has a triangle index out of range", o.name));
            }
            if let Some(p) = &o.parent
                && (self.find(p).is_none() || self.cycles(&o.name, p))
            {
                return Err(format!("object '{}' has invalid parent '{p}'", o.name));
            }
        }
        Ok(())
    }

    /// Wavefront OBJ, world space, one `o` group per object (no materials).
    pub fn to_obj(&self) -> String {
        let mut s = String::from("# elliott-3d\n");
        let mut base = 1;
        for (i, o) in self.objects.iter().enumerate().filter(|(_, o)| !o.tris.is_empty()) {
            let xf = self.world(i);
            let _ = writeln!(s, "o {}", o.name.replace(char::is_whitespace, "_"));
            for &v in &o.verts {
                let p = xf.point(v);
                let _ = writeln!(s, "v {} {} {}", p.0, p.1, p.2);
            }
            for t in &o.tris {
                let _ = writeln!(s, "f {} {} {}", base + t[0], base + t[1], base + t[2]);
            }
            base += o.verts.len() as u32;
        }
        s
    }
}
