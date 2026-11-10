use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;

/// A map of one flat tile that is read from nowhere, in a server with no installation.
fn server(allow_edit: bool) -> Server {
    let _map = crate::doc::one_map();
    let nowhere = std::env::temp_dir().join("openmapper-mcp-no-omsi");
    let roots = crate::install::Roots { omsi: nowhere.clone(), content: nowhere };
    let mut doc = Document {
        global: Default::default(),
        _publisher_mounts: std::sync::Arc::new(Default::default()),
        _publisher_protected_source: false,
        tiles: Default::default(),
        order: Vec::new(),
        dir: PathBuf::new(),
        source: None,
        global_dirty: false,
        fresh: false,
        global_rev: 0,
        timetable: Default::default(),
        traffic: Default::default(),
        descriptions: Default::default(),
        groups: Vec::new(),
    };
    doc.add_tile((0, 0));
    let mut s = Server::new(roots, allow_edit);
    s.wait_lib();
    s.put(doc);
    s
}

fn talk(s: &mut Server, line: &str) -> Value {
    s.handle(line).expect("an answer")
}

/// What a tool call gave, as data.
fn call(s: &mut Server, name: &str, args: Value) -> Result<Value, String> {
    s.call(name, &args).map(super::read::out_json)
}

#[test]
fn speaks_the_protocol_and_is_read_only_unless_told() {
    let mut s = server(false);
    let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#);
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert!(r["result"]["instructions"].as_str().unwrap().contains("render"));
    assert!(s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"map_info") && names.contains(&"render") && names.contains(&"catalogue"));
    assert!(!names.contains(&"save") && !names.contains(&"draw_road"), "no editing tools when read-only");
    // a point converts, given short as [x, y] or by tile
    let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"convert","arguments":{"tile":[1,2],"local":[10,20]}}}"#);
    assert_eq!(r["result"]["isError"], false);
    let text: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let ts = crate::doc::tile_size();
    assert_eq!(text["global"]["x"], json!(number(ts + 10.0)));
    assert_eq!(text["tile"], json!([1, 2]));
    let v = call(&mut s, "convert", json!([5.0, 7.0])).unwrap();
    assert_eq!(v["tile"], json!([0, 0]));
    // editing is refused, also inside a batch
    let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"save","arguments":{}}}"#);
    assert_eq!(r["result"]["isError"], true);
    let v = call(&mut s, "batch", json!({ "calls": [{ "tool": "map_info" }, { "tool": "terrain_brush", "arguments": { "mode": "raise", "path": [[10, 10]] } }] })).unwrap();
    assert_eq!(v["done"], 1);
    assert_eq!(v["failed"], 1);
    let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":5,"method":"nothing"}"#);
    assert_eq!(r["error"]["code"], -32601);
}

#[test]
fn a_publisher_protected_map_is_not_exposed_through_any_tool() {
    let mut s = server(true);
    s.ed.doc.as_mut().unwrap()._publisher_protected_source = true;
    let names: Vec<String> = s.tools().iter().filter_map(|t| t["name"].as_str().map(String::from)).collect();
    for name in names.iter().filter(|n| !["open_map", "new_map", "catalogue", "list_maps"].contains(&n.as_str())) {
        assert_eq!(s.call(name, &json!({})).err().as_deref(), Some(PROTECTED), "protected source escaped through {name}");
    }
    assert!(names.iter().any(|n| n == "render") && names.iter().any(|n| n == "batch"));
}

#[test]
fn the_brush_raises_the_ground_and_undo_takes_it_back() {
    let mut s = server(true);
    let ts = crate::doc::tile_size();
    let c = json!([ts * 0.5, ts * 0.5]);
    let flat = s.ground(glam::DVec2::splat(ts * 0.5));
    let v = call(&mut s, "terrain_brush", json!({ "mode": "raise", "path": [c], "radius": 40, "seconds": 2 })).unwrap();
    let raised = s.ground(glam::DVec2::splat(ts * 0.5));
    assert!(raised > flat + 0.1, "raised from {flat} to {raised}: {v}");
    assert_eq!(v["unsaved"], true);
    call(&mut s, "undo", json!({})).unwrap();
    assert!((s.ground(glam::DVec2::splat(ts * 0.5)) - flat).abs() < 1e-3, "undone");
    call(&mut s, "redo", json!({})).unwrap();
    assert!(s.ground(glam::DVec2::splat(ts * 0.5)) > flat + 0.1, "redone");
    // a batch of edits is one step to undo
    call(&mut s, "undo", json!({})).unwrap();
    let v = call(&mut s, "batch", json!({ "calls": [
        { "tool": "terrain_brush", "arguments": { "mode": "raise", "path": [c], "radius": 30 } },
        { "tool": "terrain_brush", "arguments": { "mode": "raise", "path": [[ts * 0.25, ts * 0.25]], "radius": 30 } }
    ] })).unwrap();
    assert_eq!(v["done"], 2, "{v}");
    assert!(s.ground(glam::DVec2::splat(ts * 0.25)) > flat + 0.1);
    call(&mut s, "undo", json!({})).unwrap();
    assert!((s.ground(glam::DVec2::splat(ts * 0.5)) - flat).abs() < 1e-3 && (s.ground(glam::DVec2::splat(ts * 0.25)) - flat).abs() < 1e-3, "both undone at once");
}

#[test]
fn undoing_a_batch_leaves_the_steps_before_it() {
    let mut s = server(true);
    let ts = crate::doc::tile_size();
    let here = glam::DVec2::splat(ts * 0.5);
    let flat = s.ground(here);
    call(&mut s, "terrain_brush", json!({ "mode": "raise", "path": [[ts * 0.5, ts * 0.5]], "radius": 30 })).unwrap();
    let raised = s.ground(here);
    // (`tiles` merges its own steps inside the batch's)
    let v = call(&mut s, "batch", json!({ "calls": [
        { "tool": "tiles", "arguments": { "add": [[1, 0], [2, 0]] } },
        { "tool": "terrain_brush", "arguments": { "mode": "raise", "path": [[ts * 0.5, ts * 0.5]], "radius": 30 } }
    ] })).unwrap();
    assert_eq!(v["done"], 2, "{v}");
    call(&mut s, "undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().tiles.len(), 1, "the batch's tiles undone");
    assert!((s.ground(here) - raised).abs() < 1e-3 && raised > flat + 0.1, "the brush stroke before the batch stays");
}

#[test]
fn render_gives_a_picture_of_the_map() {
    let mut s = server(false);
    match s.call("render", &json!({ "size": 300 })) {
        Ok(Out::Image { png, meta }) => {
            assert_eq!(&png[..4], b"\x89PNG");
            assert_eq!(meta["pixels"], json!([300, 300]));
        }
        Ok(Out::Json(v)) => panic!("no picture: {v}"),
        Err(e) => panic!("{e}"),
    }
    let r = talk(&mut s, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"render","arguments":{"center":[100,100],"radius":50,"size":256}}}"#);
    assert_eq!(r["result"]["content"][0]["type"], "image");
    assert_eq!(r["result"]["content"][0]["mimeType"], "image/png");
}

#[test]
fn tiles_are_added_next_to_the_map_and_the_terrain_grid_reads_them() {
    let mut s = server(true);
    let v = call(&mut s, "tiles", json!({ "add": [[2, 0], [1, 0], [5, 5]] })).unwrap();
    assert_eq!(v["tile_count"], 3, "{v}");
    assert!(v["problems"].to_string().contains("5, 5"));
    let ts = crate::doc::tile_size();
    let v = call(&mut s, "terrain", json!({ "min": [0, 0], "max": [ts * 3.0 - 1.0, 10], "step": ts })).unwrap();
    assert_eq!(v["columns"], 3);
    assert!(v["rows_from_south"][0].as_array().unwrap().iter().all(|h| !h.is_null()));
}

#[test]
fn base64_is_the_standard_one() {
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(base64(b""), "");
}
