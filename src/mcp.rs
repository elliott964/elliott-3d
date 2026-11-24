//! MCP server (JSON-RPC 2.0, newline-delimited over stdio) + tool implementations.
use crate::geom::{self, Mesh};
use crate::math::V3;
use crate::render::{self, Opts};
use crate::scene::{Object, Scene};
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "elliott-3d: headless 3D modeler. Y-up, right-handed, unitless. Primitives are 1x1x1 \
centered at the origin, so `scale` = dimensions (a 2x1x3 box: scale [2,1,3]); rotation is Euler XYZ degrees. \
Colors: \"#rrggbb\", a CSS-ish name, or [r,g,b] in 0..1. Build with add_* tools, then call `render` to look at \
the result (view \"all\" = front/right/top/iso). The ground grid lies on y=0 (red line = X axis, blue = Z axis).";

// ---------- protocol ----------

pub fn serve() {
    let mut scene = Scene::default();
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&mut scene, &msg),
            Err(e) => Some(rpc_err(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_err(id: Value, code: i32, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

fn handle(scene: &mut Scene, msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?.clone(); // no id = notification, never answered
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[0]);
            rpc_ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "elliott-3d", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let res = catch_unwind(AssertUnwindSafe(|| call(scene, name, &args)))
                .unwrap_or_else(|_| Err("internal error (panic) while running tool".into()));
            rpc_ok(
                id,
                match res {
                    Ok(content) => json!({"content": content, "isError": false}),
                    Err(e) => json!({"content": [text(e)], "isError": true}),
                },
            )
        }
        _ => rpc_err(id, -32601, &format!("method not found: {method}")),
    })
}

fn text(s: impl ToString) -> Value {
    json!({"type": "text", "text": s.to_string()})
}

// ---------- argument parsing ----------

struct Args<'a>(&'a Value);

impl<'a> Args<'a> {
    fn get(&self, k: &str) -> Option<&'a Value> {
        self.0.get(k).filter(|v| !v.is_null())
    }
    fn num(&self, k: &str) -> Result<Option<f64>, String> {
        self.get(k).map(|v| v.as_f64().ok_or(format!("'{k}' must be a number"))).transpose()
    }
    fn int(&self, k: &str) -> Result<Option<i64>, String> {
        Ok(self.num(k)?.map(|n| n as i64))
    }
    fn str(&self, k: &str) -> Result<Option<&'a str>, String> {
        self.get(k).map(|v| v.as_str().ok_or(format!("'{k}' must be a string"))).transpose()
    }
    fn bool(&self, k: &str) -> Result<Option<bool>, String> {
        self.get(k).map(|v| v.as_bool().ok_or(format!("'{k}' must be a boolean"))).transpose()
    }
    fn req_str(&self, k: &str) -> Result<&'a str, String> {
        self.str(k)?.ok_or(format!("'{k}' is required"))
    }
    /// `[x,y,z]`, or a single number when `uniform` (e.g. scale).
    fn v3(&self, k: &str, uniform: bool) -> Result<Option<V3>, String> {
        self.get(k).map(|v| parse_v3(v, k, uniform)).transpose()
    }
    fn color(&self, k: &str) -> Result<Option<[f64; 3]>, String> {
        self.get(k).map(parse_color).transpose().map_err(|e| format!("'{k}': {e}"))
    }
    /// Array of `[a,b]` pairs.
    fn pairs(&self, k: &str) -> Result<Vec<[f64; 2]>, String> {
        let arr = self.get(k).and_then(Value::as_array).ok_or(format!("'{k}' must be an array of [x,y] pairs"))?;
        arr.iter()
            .map(|p| match p.as_array().map(|a| a.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
                Some([Some(a), Some(b)]) => Ok([*a, *b]),
                _ => Err(format!("'{k}' entries must be [number, number]")),
            })
            .collect()
    }
}

fn parse_v3(v: &Value, k: &str, uniform: bool) -> Result<V3, String> {
    if uniform && let Some(n) = v.as_f64() {
        return Ok(V3(n, n, n));
    }
    match v.as_array().map(|a| a.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
        Some([Some(x), Some(y), Some(z)]) => Ok(V3(*x, *y, *z)),
        _ => Err(format!("'{k}' must be [x,y,z]{}", if uniform { " or a number" } else { "" })),
    }
}

const NAMED: [(&str, &str); 14] = [
    ("red", "e03131"), ("green", "2f9e44"), ("blue", "1c7ed6"), ("yellow", "fcc419"), ("orange", "f76707"),
    ("purple", "9c36b5"), ("pink", "f783ac"), ("cyan", "15aabf"), ("brown", "8d5524"), ("white", "f1f3f5"),
    ("gray", "868e96"), ("grey", "868e96"), ("black", "212529"), ("gold", "e5b80b"),
];

fn parse_color(v: &Value) -> Result<[f64; 3], String> {
    match v {
        Value::String(s) => {
            let s = s.trim().to_lowercase();
            let hex = NAMED.iter().find(|(n, _)| *n == s).map(|(_, h)| h.to_string()).unwrap_or_else(|| s.trim_start_matches('#').to_string());
            let hex = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex };
            let n = (hex.len() == 6).then(|| u32::from_str_radix(&hex, 16).ok()).flatten();
            n.map(|n| [(n >> 16) as u8, (n >> 8) as u8, n as u8].map(|b| b as f64 / 255.0))
                .ok_or(format!("unrecognized color '{s}'; use #rrggbb, a basic name, or [r,g,b] in 0..1"))
        }
        Value::Array(_) => {
            let V3(r, g, b) = parse_v3(v, "color", false)?;
            if [r, g, b].iter().any(|c| !(0.0..=1.0).contains(c)) {
                return Err("[r,g,b] components must be in 0..1".into());
            }
            Ok([r, g, b])
        }
        _ => Err("must be a string or [r,g,b]".into()),
    }
}

fn hex(c: [f64; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

/// Validated, parsed object properties shared by add_* / set_object / duplicate_object.
struct Props {
    position: Option<V3>,
    rotation: Option<V3>,
    scale: Option<V3>,
    color: Option<[f64; 3]>,
    specular: Option<f64>,
    emissive: Option<f64>,
    crease: Option<f64>,
    parent: Option<Option<String>>,
}

impl Props {
    /// `name` is the object being configured (for parent-cycle detection).
    fn parse(scene: &Scene, name: &str, a: &Args) -> Result<Props, String> {
        let unit = |k: &str| -> Result<Option<f64>, String> {
            match a.num(k)? {
                Some(x) if !(0.0..=1.0).contains(&x) => Err(format!("'{k}' must be in 0..1")),
                x => Ok(x),
            }
        };
        let parent = match a.0.get("parent") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(p)) if p.is_empty() => Some(None),
            Some(Value::String(p)) => {
                if scene.find(p).is_none() {
                    return Err(format!("parent '{p}' does not exist"));
                }
                if scene.cycles(name, p) {
                    return Err(format!("parenting '{name}' under '{p}' would create a cycle"));
                }
                Some(Some(p.clone()))
            }
            Some(_) => return Err("'parent' must be a string or null".into()),
        };
        Ok(Props {
            position: a.v3("position", false)?,
            rotation: a.v3("rotation", false)?,
            scale: a.v3("scale", true)?,
            color: a.color("color")?,
            specular: unit("specular")?,
            emissive: unit("emissive")?,
            crease: a.num("crease")?.map(|c| c.clamp(0.0, 180.0)),
            parent,
        })
    }

    fn apply(self, o: &mut Object) {
        o.position = self.position.unwrap_or(o.position);
        o.rotation = self.rotation.unwrap_or(o.rotation);
        o.scale = self.scale.unwrap_or(o.scale);
        o.color = self.color.unwrap_or(o.color);
        o.specular = self.specular.unwrap_or(o.specular);
        o.emissive = self.emissive.unwrap_or(o.emissive);
        o.crease = self.crease.unwrap_or(o.crease);
        if let Some(p) = self.parent {
            o.parent = p;
        }
    }
}

// ---------- tools ----------

fn r4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4 + 0.0
}

fn jv(v: V3) -> Value {
    json!([r4(v.0), r4(v.1), r4(v.2)])
}

fn describe(scene: &Scene, i: usize) -> Value {
    let o = &scene.objects[i];
    let mut d = json!({
        "name": o.name, "parent": o.parent, "verts": o.verts.len(), "tris": o.tris.len(),
        "position": jv(o.position), "rotation": jv(o.rotation), "scale": jv(o.scale),
        "color": hex(o.color), "specular": o.specular, "emissive": o.emissive, "crease": o.crease,
    });
    if let Some((lo, hi)) = scene.object_bounds(i) {
        d["world_bounds"] = json!({"min": jv(lo), "max": jv(hi), "size": jv(hi - lo)});
    }
    d
}

fn scene_bounds(scene: &Scene) -> Value {
    scene.bounds().map_or(Value::Null, |(lo, hi)| json!({"min": jv(lo), "max": jv(hi), "size": jv(hi - lo)}))
}

/// Add a new object built from `mesh`, honoring name/transform/material args.
fn insert(scene: &mut Scene, default_name: &str, mesh: Mesh, a: &Args) -> Result<Value, String> {
    let name = match a.str("name")? {
        Some("") => return Err("'name' must not be empty".into()),
        Some(n) if scene.find(n).is_some() => return Err(format!("object '{n}' already exists")),
        Some(n) => n.to_string(),
        None => scene.unique(default_name),
    };
    let props = Props::parse(scene, &name, a)?;
    let mut o = Object::new(name, mesh.v, mesh.t);
    props.apply(&mut o);
    scene.objects.push(o);
    Ok(describe(scene, scene.objects.len() - 1))
}

fn call(scene: &mut Scene, tool: &str, args: &Value) -> Result<Vec<Value>, String> {
    let a = &Args(args);
    let seg = |a: &Args| -> Result<usize, String> { Ok(a.int("segments")?.unwrap_or(32).clamp(3, 256) as usize) };
    let one = |v: Value| Ok(vec![text(serde_json::to_string_pretty(&v).unwrap())]);
    match tool {
        "scene_clear" => {
            scene.objects.clear();
            one(json!({"ok": true}))
        }
        "add_primitive" => {
            let kind = a.req_str("kind")?;
            let mesh = match kind {
                "cube" => geom::cube(),
                "plane" => geom::plane(),
                "sphere" => geom::sphere(seg(a)?),
                "cylinder" => geom::cylinder(seg(a)?),
                "cone" => geom::cone(seg(a)?),
                "torus" => geom::torus(seg(a)?, a.num("ratio")?.unwrap_or(0.35).clamp(0.02, 1.0)),
                k => return Err(format!("unknown kind '{k}'; use cube|sphere|cylinder|cone|plane|torus")),
            };
            one(insert(scene, kind, mesh, a)?)
        }
        "add_mesh" => {
            let verts = a.get("vertices").and_then(Value::as_array).ok_or("'vertices' must be an array of [x,y,z]")?;
            let verts = verts.iter().map(|v| parse_v3(v, "vertices[]", false)).collect::<Result<Vec<_>, _>>()?;
            let faces = a.get("faces").and_then(Value::as_array).ok_or("'faces' must be an array of index arrays")?;
            let mut m = Mesh { v: verts, t: vec![] };
            for (fi, f) in faces.iter().enumerate() {
                let idx = f
                    .as_array()
                    .map(|a| a.iter().map(|i| i.as_u64().filter(|&i| (i as usize) < m.v.len())).collect::<Option<Vec<_>>>())
                    .flatten()
                    .filter(|i| i.len() >= 3)
                    .ok_or(format!("faces[{fi}] must list >=3 valid vertex indices"))?;
                for k in 1..idx.len() - 1 {
                    m.t.push([idx[0] as u32, idx[k] as u32, idx[k + 1] as u32]); // convex fan
                }
            }
            let mut o = a.0.clone();
            o["crease"] = a.get("crease").cloned().unwrap_or(json!(0)); // raw meshes default to flat
            one(insert(scene, "mesh", m, &Args(&o))?)
        }
        "add_extrude" => {
            let depth = a.num("depth")?.ok_or("'depth' is required")?;
            one(insert(scene, "extrude", geom::extrude(&a.pairs("profile")?, depth)?, a)?)
        }
        "add_lathe" => one(insert(scene, "lathe", geom::lathe(&a.pairs("profile")?, seg(a)?)?, a)?),
        "add_group" => one(insert(scene, "group", Mesh::default(), a)?),
        "set_object" => {
            let i = scene.find(a.req_str("name")?).ok_or("no such object")?;
            let old = scene.objects[i].name.clone();
            let new = a.str("new_name")?.filter(|n| *n != old);
            if let Some(n) = new
                && (n.is_empty() || scene.find(n).is_some())
            {
                return Err(format!("cannot rename to '{n}' (empty or taken)"));
            }
            let props = Props::parse(scene, &old, a)?;
            props.apply(&mut scene.objects[i]);
            if let Some(n) = new {
                for o in &mut scene.objects {
                    if o.parent.as_deref() == Some(&old) {
                        o.parent = Some(n.to_string());
                    }
                }
                scene.objects[i].name = n.to_string();
            }
            one(describe(scene, i))
        }
        "delete_object" => {
            let i = scene.find(a.req_str("name")?).ok_or("no such object")?;
            let gone: Vec<String> = scene.subtree(i).iter().map(|&k| scene.objects[k].name.clone()).collect();
            scene.objects.retain(|o| !gone.contains(&o.name));
            one(json!({"deleted": gone}))
        }
        "duplicate_object" => {
            let src = scene.find(a.req_str("name")?).ok_or("no such object")?;
            let root = match a.str("new_name")? {
                Some("") => return Err("'new_name' must not be empty".into()),
                Some(n) if scene.find(n).is_some() => return Err(format!("object '{n}' already exists")),
                Some(n) => n.to_string(),
                None => scene.unique(&format!("{}_copy", scene.objects[src].name)),
            };
            let props = Props::parse(scene, &root, a)?;
            let mut renamed: Vec<(String, String)> = vec![];
            let mut names = vec![];
            for (n, &i) in scene.subtree(src).iter().enumerate() {
                let old = scene.objects[i].name.clone();
                let new = if n == 0 { root.clone() } else { scene.unique(&format!("{old}_copy")) };
                let mut o = scene.objects[i].clone();
                o.name = new.clone();
                if n > 0 {
                    o.parent = o.parent.and_then(|p| renamed.iter().find(|(old, _)| *old == p).map(|(_, n)| n.clone()));
                }
                renamed.push((old, new.clone()));
                names.push(new);
                scene.objects.push(o);
            }
            let last = scene.find(&root).unwrap();
            props.apply(&mut scene.objects[last]);
            one(json!({"created": names, "root": describe(scene, last)}))
        }
        "list_objects" => {
            let objs: Vec<_> = (0..scene.objects.len()).map(|i| describe(scene, i)).collect();
            one(json!({"count": objs.len(), "bounds": scene_bounds(scene), "objects": objs}))
        }
        "render" => render_tool(scene, a),
        "save_scene" => {
            let path = a.req_str("path")?;
            std::fs::write(path, serde_json::to_vec(scene).unwrap()).map_err(|e| format!("{path}: {e}"))?;
            one(json!({"saved": path, "objects": scene.objects.len()}))
        }
        "load_scene" => {
            let path = a.req_str("path")?;
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let loaded: Scene = serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?;
            loaded.validate()?;
            *scene = loaded;
            one(json!({"loaded": path, "objects": scene.objects.len()}))
        }
        "export_obj" => {
            let path = a.req_str("path")?;
            std::fs::write(path, scene.to_obj()).map_err(|e| format!("{path}: {e}"))?;
            one(json!({"exported": path}))
        }
        t => Err(format!("unknown tool '{t}'")),
    }
}

fn render_tool(scene: &Scene, a: &Args) -> Result<Vec<Value>, String> {
    let w = a.int("width")?.unwrap_or(768).clamp(16, 2048) as usize;
    let h = a.int("height")?.unwrap_or(576).clamp(16, 2048) as usize;
    let fov = a.num("fov")?.unwrap_or(40.0).clamp(5.0, 120.0);
    let ss = a.int("samples")?.unwrap_or(2).clamp(1, 4) as usize;
    let bounds = scene.bounds();
    let (_, radius) = render::center_radius(bounds);
    let spacing = match (a.bool("grid")?.unwrap_or(true), a.num("grid_spacing")?) {
        (false, _) => None,
        (true, Some(s)) if s > 0.0 => Some(s),
        (true, Some(_)) => return Err("'grid_spacing' must be > 0".into()),
        (true, None) => Some(render::auto_spacing(radius)),
    };
    let opts = Opts { w, h, bg: a.color("background")?.unwrap_or([0.17, 0.19, 0.22]), grid: spacing, ss };
    let aspect = w as f64 / h as f64;

    let shots: Vec<(String, render::Cam)> = if let Some(c) = a.get("camera") {
        let c = Args(c);
        let pos = c.v3("position", false)?.ok_or("camera.position is required")?;
        let target = match c.v3("target", false)? {
            Some(t) => t,
            None => render::center_radius(bounds).0,
        };
        if (pos - target).len() < 1e-9 {
            return Err("camera.position and camera.target must differ".into());
        }
        vec![("custom".into(), render::Cam { pos, target, fov })]
    } else {
        let names: Vec<String> = match a.get("view") {
            None => vec!["iso".into()],
            Some(Value::String(s)) if s == "all" => ["front", "right", "top", "iso"].map(String::from).into(),
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(v)) => v.iter().map(|x| x.as_str().map(String::from).ok_or("'view' entries must be strings")).collect::<Result<_, _>>()?,
            Some(_) => return Err("'view' must be a string or array of strings".into()),
        };
        if names.is_empty() || names.len() > 8 {
            return Err("'view' needs 1..8 entries".into());
        }
        names
            .into_iter()
            .map(|n| render::frame(&n, bounds, fov, aspect).map(|c| (n, c)))
            .collect::<Result<_, _>>()?
    };

    let multi = shots.len() > 1;
    let (mut content, mut info) = (vec![], vec![]);
    for (name, cam) in &shots {
        let png = render::encode_png(w, h, &render::render(scene, cam, &opts))?;
        let mut saved = Value::Null;
        if let Some(path) = a.str("path")? {
            let p = std::path::Path::new(path);
            let path = if multi {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("render");
                p.with_file_name(format!("{stem}_{name}.png"))
            } else {
                p.to_path_buf()
            };
            std::fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
            saved = json!(path.display().to_string());
        }
        info.push(json!({
            "view": name, "saved": saved,
            "camera": {"position": jv(cam.pos), "target": jv(cam.target), "fov": cam.fov},
        }));
        content.push(json!({"type": "image", "data": render::base64(&png), "mimeType": "image/png"}));
    }
    let summary = json!({"size": [w, h], "grid_spacing": spacing, "scene_bounds": scene_bounds(scene), "shots": info});
    content.insert(0, text(serde_json::to_string(&summary).unwrap()));
    Ok(content)
}

// ---------- tool schemas ----------

fn s_v3(d: &str) -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": d})
}
fn s_num(d: &str) -> Value {
    json!({"type": "number", "description": d})
}
fn s_str(d: &str) -> Value {
    json!({"type": "string", "description": d})
}
fn s_pairs(d: &str) -> Value {
    json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}, "description": d})
}

/// Properties shared by everything that creates or edits an object.
fn common() -> Vec<(&'static str, Value)> {
    vec![
        ("name", s_str("Unique object name (auto-generated when omitted on creation).")),
        ("parent", json!({"type": ["string", "null"], "description": "Parent object name; transforms compose. null or \"\" detaches."})),
        ("position", s_v3("Local position. Default [0,0,0].")),
        ("rotation", s_v3("Local Euler XYZ rotation in degrees. Default [0,0,0].")),
        ("scale", json!({"type": ["number", "array"], "items": {"type": "number"}, "description": "[x,y,z] or a uniform number. For primitives this is the size. Default 1."})),
        ("color", json!({"type": ["string", "array"], "items": {"type": "number"}, "description": "\"#rrggbb\", name (red, blue, gold, ...), or [r,g,b] 0..1."})),
        ("specular", s_num("Highlight strength 0..1. Default 0.25.")),
        ("emissive", s_num("Self-lit fraction 0..1 (1 = flat unlit color). Default 0.")),
        ("crease", s_num("Smooth-shading angle limit in degrees; faces meeting at less than this blend smoothly, 0 = flat. Default 40 (0 for add_mesh).")),
    ]
}

fn tool(name: &str, desc: &str, props: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let props: Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    json!({"name": name, "description": desc, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

fn with_common(mut p: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    p.extend(common());
    p
}

fn tools() -> Value {
    let seg = ("segments", s_num("Radial segments, 3..256. Default 32."));
    let name_only = |d: &str| vec![("name", s_str(d))];
    let path = |d: &str| vec![("path", s_str(d))];
    json!([
        tool("scene_clear", "Delete every object.", vec![], &[]),
        tool(
            "add_primitive",
            "Add a unit-sized primitive (fits a 1x1x1 box at the origin); set `scale` for dimensions. torus: outer diameter 1.",
            with_common(vec![
                ("kind", json!({"type": "string", "enum": ["cube", "sphere", "cylinder", "cone", "plane", "torus"]})),
                seg.clone(),
                ("ratio", s_num("torus only: tube radius / ring radius, 0.02..1. Default 0.35.")),
            ]),
            &["kind"]
        ),
        tool(
            "add_mesh",
            "Add a raw mesh. Faces are index lists (3+ indices, convex polygons are fan-triangulated). Shaded flat by default.",
            with_common(vec![
                ("vertices", json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3}})),
                ("faces", json!({"type": "array", "items": {"type": "array", "items": {"type": "integer"}, "minItems": 3}})),
            ]),
            &["vertices", "faces"]
        ),
        tool(
            "add_extrude",
            "Extrude a simple XY polygon (no holes, any winding, concave OK) from z=0 to z=depth.",
            with_common(vec![("profile", s_pairs("Polygon corners [[x,y],...], >=3.")), ("depth", s_num("Extrusion length along +Z."))]),
            &["profile", "depth"]
        ),
        tool(
            "add_lathe",
            "Revolve a [radius, y] profile around the Y axis (vases, wheels, bottles). Open profile = open surface; end profiles at radius 0 to close.",
            with_common(vec![("profile", s_pairs("Points [[radius,y],...], >=2.")), seg]),
            &["profile"]
        ),
        tool("add_group", "Add an empty transform node to parent other objects under.", with_common(vec![]), &[]),
        tool(
            "set_object",
            "Update any subset of an object's transform, material, parent or name.",
            with_common(vec![("new_name", s_str("Rename (children follow)."))])
                .into_iter()
                .map(|(k, v)| if k == "name" { ("name", s_str("Existing object to modify.")) } else { (k, v) })
                .collect(),
            &["name"]
        ),
        tool("delete_object", "Delete an object and its descendants.", name_only("Object name."), &["name"]),
        tool(
            "duplicate_object",
            "Copy an object and its descendants (names get a _copy suffix). Any transform/material args override the copy's root.",
            with_common(vec![("new_name", s_str("Name for the copy of the root."))])
                .into_iter()
                .map(|(k, v)| if k == "name" { ("name", s_str("Object to copy.")) } else { (k, v) })
                .collect(),
            &["name"]
        ),
        tool("list_objects", "List all objects with transforms, materials and world-space bounds, plus scene bounds.", vec![], &[]),
        tool(
            "render",
            "Take a photo. Returns PNG image(s) plus camera info. Default: one iso view framing the whole scene.",
            vec![
                ("view", json!({"type": ["string", "array"], "items": {"type": "string"}, "description": "front|back|left|right|top|bottom|iso, a list of those, or \"all\" (front,right,top,iso). Auto-framed on the scene. Default iso."})),
                ("camera", json!({"type": "object", "description": "Custom camera (overrides view).", "properties": {"position": s_v3("Eye."), "target": s_v3("Look-at point. Default scene center.")}, "required": ["position"]})),
                ("fov", s_num("Vertical field of view, degrees. Default 40.")),
                ("width", s_num("Pixels, 16..2048. Default 768.")),
                ("height", s_num("Pixels, 16..2048. Default 576.")),
                ("background", json!({"type": ["string", "array"], "items": {"type": "number"}, "description": "Color. Default dark slate."})),
                ("grid", json!({"type": "boolean", "description": "Ground grid on y=0. Default true."})),
                ("grid_spacing", s_num("Override grid spacing (default: power of 10 matched to scene size).")),
                ("samples", s_num("Supersampling per axis 1..4 (antialiasing). Default 2.")),
                ("path", s_str("Also save the PNG here (multi-view: _<view> is appended to the file stem).")),
            ],
            &[]
        ),
        tool("save_scene", "Write the scene to a JSON file.", path("File path."), &["path"]),
        tool("load_scene", "Replace the scene with one from a JSON file.", path("File path."), &["path"]),
        tool("export_obj", "Export world-space geometry as Wavefront OBJ (no materials).", path("File path."), &["path"]),
    ])
}
