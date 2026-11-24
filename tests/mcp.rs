use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

struct Server {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Server {
    fn start() -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_elliott-3d"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Server { child, out, next: 1 }
    }

    fn send(&mut self, v: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], id);
        v
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name": name, "arguments": args}))["result"].clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn decode_png(b64: &str) -> (png::OutputInfo, Vec<u8>) {
    // minimal base64 decoder
    let mut bits = 0u32;
    let mut n = 0;
    let mut bytes = vec![];
    for c in b64.bytes().filter(|&c| c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("bad base64"),
        } as u32;
        bits = bits << 6 | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            bytes.push((bits >> n) as u8);
        }
    }
    let mut r = png::Decoder::new(bytes.as_slice()).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    (info, buf)
}

#[test]
fn model_then_photograph() {
    let mut s = Server::start();
    let init = s.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "elliott-3d");
    s.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})); // must not be answered

    let tools = s.rpc("tools/list", json!({}));
    assert!(tools["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "render"));

    let r = s.tool("add_primitive", json!({"kind": "cube", "name": "box", "scale": [2, 1, 1], "color": "red", "position": [0, 0.5, 0]}));
    assert_eq!(r["isError"], false);
    let info: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(info["world_bounds"]["size"], json!([2.0, 1.0, 1.0]));

    // duplicate names and bad input are tool errors, not protocol errors
    assert_eq!(s.tool("add_primitive", json!({"kind": "cube", "name": "box"}))["isError"], true);
    assert_eq!(s.tool("add_primitive", json!({"kind": "blob"}))["isError"], true);

    // a child follows its parent
    s.tool("add_primitive", json!({"kind": "sphere", "name": "ball", "parent": "box", "position": [0, 1, 0], "scale": 0.5}));
    s.tool("set_object", json!({"name": "box", "position": [10, 0.5, 0]}));
    let list = s.tool("list_objects", json!({}));
    let list: Value = serde_json::from_str(list["content"][0]["text"].as_str().unwrap()).unwrap();
    let ball = list["objects"].as_array().unwrap().iter().find(|o| o["name"] == "ball").unwrap();
    assert_eq!(ball["world_bounds"]["min"][0], json!(10.0 - 0.5 * 2.0 * 0.5)); // parent scale x=2 stretches child
    s.tool("set_object", json!({"name": "box", "position": [0, 0.5, 0]}));

    let shot = s.tool("render", json!({"view": ["front", "top"], "width": 160, "height": 120, "background": "#000000", "grid": false}));
    assert_eq!(shot["isError"], false);
    let imgs: Vec<_> = shot["content"].as_array().unwrap().iter().filter(|c| c["type"] == "image").collect();
    assert_eq!(imgs.len(), 2);
    let (info, px) = decode_png(imgs[0]["data"].as_str().unwrap());
    assert_eq!((info.width, info.height), (160, 120));
    assert_eq!(&px[0..3], &[0, 0, 0], "corner is background");
    let c = ((60 * 160) + 80) * 3;
    assert!(px[c] > 80 && px[c] > px[c + 1] * 2, "center pixel should be red: {:?}", &px[c..c + 3]);
}
