mod geom;
mod math;
mod mcp;
mod render;
mod scene;

const HELP: &str = "elliott-3d - tiny headless 3D modeler for MCP

USAGE:
  elliott-3d                              serve MCP over stdio (default)
  elliott-3d render <scene.json> <out.png> [view]
                                          render a saved scene (view: front|back|left|right|top|bottom|iso)
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => mcp::serve(),
        Some("render") if args.len() >= 3 => {
            if let Err(e) = render_file(&args[1], &args[2], args.get(3).map_or("iso", String::as_str)) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some("--version") => println!("elliott-3d {}", env!("CARGO_PKG_VERSION")),
        _ => print!("{HELP}"),
    }
}

fn render_file(scene_path: &str, out: &str, view: &str) -> Result<(), String> {
    let scene: scene::Scene =
        serde_json::from_slice(&std::fs::read(scene_path).map_err(|e| format!("{scene_path}: {e}"))?).map_err(|e| format!("{scene_path}: {e}"))?;
    scene.validate()?;
    let (w, h) = (768, 576);
    let bounds = scene.bounds();
    let cam = render::frame(view, bounds, 40.0, w as f64 / h as f64)?;
    let spacing = render::auto_spacing(render::center_radius(bounds).1);
    let opts = render::Opts { w, h, bg: [0.17, 0.19, 0.22], grid: Some(spacing), ss: 2 };
    let png = render::encode_png(w, h, &render::render(&scene, &cam, &opts))?;
    std::fs::write(out, png).map_err(|e| format!("{out}: {e}"))
}
