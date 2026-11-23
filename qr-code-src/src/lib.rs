wit_bindgen::generate!({ path: "wit", world: "command-plugin" });

use craft::plugin::commands;
use craft::plugin::types::{FieldFailure, PluginError};
use exports::craft::plugin::entry::Guest;
use qrcodegen::{QrCode, QrCodeEcc};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Longest text accepted, in bytes. A version-40 code holds more, but the model would be huge.
const MAX_TEXT: usize = 500;
/// Plate thickness and quiet-zone width (in modules) of a code on a new plate.
const PLATE: f64 = 2.0;
const MARGIN: f64 = 2.0;
/// Most outlines one run draws: the host allows 256 calls, and a few go to the other commands.
const MAX_LOOPS: usize = 240;
/// Smallest module that prints legibly with a common 0.4 mm nozzle.
const MIN_MODULE: f64 = 0.4;

struct Plugin;

impl Guest for Plugin {
    fn execute(entry: String, params: String) -> Result<String, PluginError> {
        let params: Value = serde_json::from_str(&params).map_err(|e| invalid("params", e))?;
        match entry.as_str() {
            "qr" => qr(&params),
            _ => Err(invalid("entry", format!("no entry `{entry}`"))),
        }
    }
}

fn qr(params: &Value) -> Result<String, PluginError> {
    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
    if text.is_empty() {
        return Err(invalid("text", "enter the text or address to encode"));
    }
    if text.len() > MAX_TEXT {
        return Err(invalid("text", format!("at most {MAX_TEXT} bytes")));
    }
    // Plain settings only: the error correction, the margin and the plate are fixed choices.
    let size = number(params, "size", 30.0, 5.0, 500.0)?;
    let height = number(params, "height", 0.6, 0.1, 20.0)?;
    let offset = number(params, "offset", 0.0, -1000.0, 1000.0)?;
    let engrave = match params.get("style").and_then(Value::as_str).unwrap_or("raised") {
        "raised" => false,
        "engraved" => true,
        other => return Err(invalid("style", format!("`{other}` is not raised or engraved"))),
    };
    let face = match params.get("face").and_then(Value::as_array).and_then(|a| a.first()) {
        Some(pick) => Some(face_frame(pick)?),
        None => None,
    };
    if engrave && face.is_none() && height >= PLATE {
        return Err(invalid("height", format!("an engraving on a new plate must be under {PLATE} mm deep")));
    }

    let code = QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|_| invalid("text", "too long for a QR code"))?;
    let n = usize::try_from(code.size()).map_err(|_| invalid("text", "empty code"))?;
    // `size` is the code's width; the plate adds a margin of MARGIN modules around it.
    let module = size / n as f64;
    if module < MIN_MODULE {
        return Err(invalid(
            "size",
            format!("too small for this text to print well: use at least {:.0} mm or a shorter text", (MIN_MODULE * n as f64).ceil()),
        ));
    }
    let margin = MARGIN;
    let plate = PLATE;
    let side = (n as f64 + 2.0 * margin) * module;

    // Where the sketch lies and how its axes run: on a face, centred on the face's centre
    // (`offset` along its normal); otherwise on top of a new plate.
    let (origin, x_dir, y_dir, target) = match &face {
        Some(f) => {
            let (right, up) = axes(f.normal);
            let o = [
                f.centre[0] + f.normal[0] * offset,
                f.centre[1] + f.normal[1] * offset,
                f.centre[2] + f.normal[2] * offset,
            ];
            (o, right, up, f.body.clone())
        }
        None => {
            commands::run(
                "solid.box",
                &json!({"name": "QR plate", "body_name": "QR plate",
                        "corner": [0, 0, 0], "length": side, "width": side, "height": plate})
                .to_string(),
            )?;
            ([side / 2.0, side / 2.0, plate], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], "QR plate".to_string())
        }
    };

    // The code is one sketch of rectangles, then one extrusion: it can be edited or moved afterwards.
    commands::run(
        "sketch.create",
        &json!({"name": "QR code", "plane": {"origin": origin, "x_dir": x_dir, "y_dir": y_dir}}).to_string(),
    )?;
    let loops = outlines(&code, n);
    // A plugin may make only 256 host calls in a run, and each loop is one of them.
    if loops.len() > MAX_LOOPS {
        return Err(invalid("text", "too long to draw: use a shorter text or link"));
    }
    let half = n as f64 * module / 2.0;
    for points in &loops {
        // The grid's y runs down and the sketch's y axis points up.
        let pts: Vec<[f64; 2]> = points
            .iter()
            .map(|&(x, y)| [f64::from(x) * module - half, (n as f64 - f64::from(y)) * module - half])
            .collect();
        commands::run("sketch.line", &json!({"points": pts, "closed": true, "connect": false}).to_string())?;
    }
    commands::run("sketch.finish", "{}")?;
    // Modules lifted off the face touch nothing, so they are kept in one body explicitly.
    commands::run(
        "solid.extrude",
        &json!({
            "sketch": "QR code", "name": "QR code", "distance": height,
            "operation": if engrave { "cut" } else { "join" },
            "direction": if engrave { "negative" } else { "positive" },
            "targets": [target], "touching": "one_body",
        })
        .to_string(),
    )?;
    Ok(json!({"modules": n, "side_mm": side, "outlines": loops.len(), "style": if engrave { "engraved" } else { "raised" }})
        .to_string())
}

/// A picked planar face: its body, outward normal and centre.
struct Face {
    body: String,
    normal: [f64; 3],
    centre: [f64; 3],
}

/// Looks the picked face up: the pick names a body and a face index, `model.faces` gives its plane.
fn face_frame(pick: &Value) -> Result<Face, PluginError> {
    let bad = |why: &str| invalid("face", why.to_string());
    let body = pick.get("body").and_then(Value::as_str).ok_or_else(|| bad("pick a face of a body"))?;
    let index = pick.get("index").and_then(Value::as_u64).ok_or_else(|| bad("the pick has no face index"))?;
    let reply = commands::run("model.faces", &json!({"body": body}).to_string())?;
    let reply: Value = serde_json::from_str(&reply).map_err(|e| invalid("face", e))?;
    let face = reply
        .get("faces")
        .and_then(Value::as_array)
        .and_then(|faces| faces.iter().find(|f| f.get("index").and_then(Value::as_u64) == Some(index)))
        .ok_or_else(|| bad("no such face"))?;
    let normal = vec3(face.get("plane_normal")).ok_or_else(|| bad("the face is not flat"))?;
    let centre = vec3(face.get("centroid")).ok_or_else(|| bad("the face has no centre"))?;
    let len = norm(normal);
    if len.is_nan() || len <= 1e-9 {
        return Err(bad("the face is not flat"));
    }
    Ok(Face { body: body.to_string(), normal: scale(normal, 1.0 / len), centre })
}

fn vec3(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    let f = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite());
    Some([f(0)?, f(1)?, f(2)?])
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The code's axes on a face: up is world Z (world Y on a nearly horizontal face), so the code reads
/// upright; right is up x normal, so right x up is the normal and the code is not mirrored.
fn axes(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let world_up = if n[2].abs() > 0.999 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let d = world_up[0] * n[0] + world_up[1] * n[1] + world_up[2] * n[2];
    let up = [world_up[0] - d * n[0], world_up[1] - d * n[1], world_up[2] - d * n[2]];
    let up = scale(up, 1.0 / norm(up));
    (cross(up, n), up)
}

/// A point of the code's grid, in modules: x to the right, y down, (0, 0) its top-left corner.
type Pt = (i32, i32);

/// The outline of every dark region as closed loops of grid points, with no line between two
/// neighbouring dark modules and none where a loop runs straight on. Dark regions that only touch at
/// a corner are kept as separate loops.
fn outlines(code: &QrCode, n: usize) -> Vec<Vec<Pt>> {
    let size = i32::try_from(n).unwrap_or(0);
    let dark = |x: i32, y: i32| x >= 0 && y >= 0 && x < size && y < size && code.get_module(x, y);
    // Each dark module's edges run clockwise on screen (the dark side on the right of travel); an
    // edge shared with another dark module cancels out.
    let mut edges: Vec<(Pt, Pt)> = Vec::new();
    for y in 0..size {
        for x in 0..size {
            if !dark(x, y) {
                continue;
            }
            if !dark(x, y - 1) {
                edges.push(((x, y), (x + 1, y)));
            }
            if !dark(x + 1, y) {
                edges.push(((x + 1, y), (x + 1, y + 1)));
            }
            if !dark(x, y + 1) {
                edges.push(((x + 1, y + 1), (x, y + 1)));
            }
            if !dark(x - 1, y) {
                edges.push(((x, y + 1), (x, y)));
            }
        }
    }
    let mut leaving: BTreeMap<Pt, Vec<usize>> = BTreeMap::new();
    for (i, (from, _)) in edges.iter().enumerate() {
        leaving.entry(*from).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut loops = Vec::new();
    for first in 0..edges.len() {
        if used.get(first).copied().unwrap_or(true) {
            continue;
        }
        let mut points = Vec::new();
        let mut current = first;
        for _ in 0..=edges.len() {
            if let Some(u) = used.get_mut(current) {
                *u = true;
            }
            let Some(&(from, to)) = edges.get(current) else { break };
            points.push(from);
            // Where several edges leave a point, turn right-most: it keeps hugging the same region.
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let turn = |i: usize| {
                edges.get(i).map_or(i32::MIN, |(a, b)| dx * (b.1 - a.1) - dy * (b.0 - a.0))
            };
            let next = leaving
                .get(&to)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&i| i == first || !used.get(i).copied().unwrap_or(true))
                .max_by_key(|&i| turn(i));
            match next {
                Some(i) if i != first => current = i,
                _ => break,
            }
        }
        loops.push(straighten(points));
    }
    loops
}

/// A closed loop without the points where it runs straight on.
fn straighten(points: Vec<Pt>) -> Vec<Pt> {
    let len = points.len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let (Some(prev), Some(here), Some(next)) =
            (points.get((i + len - 1) % len), points.get(i), points.get((i + 1) % len))
        else {
            continue;
        };
        let cross = (here.0 - prev.0) * (next.1 - here.1) - (here.1 - prev.1) * (next.0 - here.0);
        if cross != 0 {
            out.push(*here);
        }
    }
    out
}

fn number(params: &Value, field: &str, default: f64, min: f64, max: f64) -> Result<f64, PluginError> {
    let v = match params.get(field) {
        None | Some(Value::Null) => default,
        Some(v) => v.as_f64().ok_or_else(|| invalid(field, "a number is expected"))?,
    };
    if v.is_finite() && v >= min && v <= max {
        Ok(v)
    } else {
        Err(invalid(field, format!("between {min} and {max}")))
    }
}

fn invalid(field: &str, reason: impl ToString) -> PluginError {
    PluginError::InvalidInput(FieldFailure { field: field.into(), reason: reason.to_string() })
}

export!(Plugin);
