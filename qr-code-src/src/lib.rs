wit_bindgen::generate!({ path: "wit", world: "command-plugin" });

use craft::plugin::commands;
use craft::plugin::types::{FieldFailure, PluginError};
use exports::craft::plugin::entry::Guest;
use qrcodegen::{QrCode, QrCodeEcc};
use serde_json::{Value, json};

/// Longest text accepted, in bytes. A version-40 code holds more, but the model would be huge.
const MAX_TEXT: usize = 500;

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
    let ecc = match params.get("correction").and_then(Value::as_str).unwrap_or("M") {
        "L" => QrCodeEcc::Low,
        "M" => QrCodeEcc::Medium,
        "Q" => QrCodeEcc::Quartile,
        "H" => QrCodeEcc::High,
        other => return Err(invalid("correction", format!("`{other}` is not L, M, Q or H"))),
    };
    let module = number(params, "module_size", 1.0, 0.2, 20.0)?;
    let height = number(params, "height", 0.6, 0.1, 20.0)?;
    let plate = number(params, "plate_thickness", 2.0, 0.2, 50.0)?;
    let margin = number(params, "margin", 2.0, 0.0, 10.0)?.round();
    let engrave = match params.get("mode").and_then(Value::as_str).unwrap_or("relief") {
        "relief" => false,
        "engrave" => true,
        other => return Err(invalid("mode", format!("`{other}` is not relief or engrave"))),
    };
    if engrave && height >= plate {
        return Err(invalid("height", "an engraving must be shallower than the plate"));
    }

    let offset = number(params, "offset", 0.0, -1000.0, 1000.0)?;
    let face = match params.get("face").and_then(Value::as_array).and_then(|a| a.first()) {
        Some(pick) => Some(face_frame(pick)?),
        None => None,
    };

    let code = QrCode::encode_text(text, ecc).map_err(|_| invalid("text", "too long for a QR code"))?;
    let n = usize::try_from(code.size()).map_err(|_| invalid("text", "empty code"))?;
    let side = (n as f64 + 2.0 * margin) * module;

    // On a face the code has no plate of its own: it joins the picked body or is cut into it.
    let target = match &face {
        Some(f) => f.body.clone(),
        None => {
            commands::run(
                "solid.box",
                &json!({"name": "QR plate", "body_name": "QR plate",
                        "corner": [0, 0, 0], "length": side, "width": side, "height": plate})
                .to_string(),
            )?;
            "QR plate".to_string()
        }
    };

    let rects = merge(&code, n);
    let base = if face.is_some() { 0.0 } else { plate };
    let (z0, dz, op) = if engrave {
        (base - height, height + 0.5, "cut")
    } else {
        (base, height, "join")
    };
    // Without a face the code sits on the plate with its margin; on a face it is centred on the
    // face's centre, so the modules are laid out around the origin and moved afterwards.
    let (ox, oy) = match face {
        Some(_) => (-(n as f64) * module / 2.0, -(n as f64) * module / 2.0),
        None => (margin * module, margin * module),
    };
    // One body per rectangle, then a single combine: a join or cut per rectangle gets slower with
    // every one already in the plate.
    let mut tools = Vec::with_capacity(rects.len());
    for (i, (x, y, w, h)) in rects.iter().enumerate() {
        // Row 0 of a code is its top edge: flip it so the code reads correctly seen from above.
        let x0 = ox + *x as f64 * module;
        let y0 = oy + (n - y - h) as f64 * module;
        let name = format!("QR module {i}");
        let args = json!({
            "body_name": name,
            "corner": [x0, y0, z0],
            "length": *w as f64 * module, "width": *h as f64 * module, "height": dz,
        });
        commands::run("solid.box", &args.to_string())?;
        tools.push(name);
    }
    if let Some(f) = &face {
        place(f, &tools, offset)?;
    }
    commands::run(
        "solid.combine",
        &json!({"target": target, "tools": tools, "operation": op}).to_string(),
    )?;
    Ok(json!({"modules": n, "side_mm": side, "boxes": rects.len(), "mode": if engrave { "engrave" } else { "relief" }})
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

/// Turns the modules built around the origin (code in XY, height along Z) so Z becomes the face's
/// normal, with the top of the code toward world Z (toward world Y on a face that looks up or
/// down), then moves them onto the face's centre, `offset` mm along the normal.
fn place(face: &Face, tools: &[String], offset: f64) -> Result<(), PluginError> {
    let n = face.normal;
    // The code's up direction: world Z, or world Y when the face is nearly horizontal.
    let world_up = if n[2].abs() > 0.999 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let d = world_up[0] * n[0] + world_up[1] * n[1] + world_up[2] * n[2];
    let up = [world_up[0] - d * n[0], world_up[1] - d * n[1], world_up[2] - d * n[2]];
    let up = scale(up, 1.0 / norm(up));
    let right = cross(up, n);

    // The rotation whose columns are right, up, n, as an axis and an angle.
    let trace = right[0] + up[1] + n[2];
    let angle = ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos();
    let axis = if angle < 1e-9 {
        None
    } else if angle > std::f64::consts::PI - 1e-6 {
        // A half turn: the axis is the column of (R + I) / 2 that is largest.
        let cols = [
            [right[0] + 1.0, right[1], right[2]],
            [up[0], up[1] + 1.0, up[2]],
            [n[0], n[1], n[2] + 1.0],
        ];
        cols.into_iter().max_by(|a, b| norm(*a).total_cmp(&norm(*b)))
    } else {
        Some([up[2] - n[1], n[0] - right[2], right[1] - up[0]])
    };
    let axis = axis.filter(|a| norm(*a) > 1e-12).map(|a| scale(a, 1.0 / norm(a)));

    let at = [
        face.centre[0] + n[0] * offset,
        face.centre[1] + n[1] * offset,
        face.centre[2] + n[2] * offset,
    ];
    let mut args = json!({"bodies": tools, "translate": at});
    if let Some(axis) = axis {
        args["axis"] = json!(axis);
        args["angle"] = json!(angle.to_degrees());
        args["origin"] = json!([0, 0, 0]);
    }
    commands::run("solid.move", &args.to_string())?;
    Ok(())
}

/// Dark modules merged into as few rectangles as a greedy sweep finds: (x, y, width, height).
fn merge(code: &QrCode, n: usize) -> Vec<(usize, usize, usize, usize)> {
    let dark = |x: usize, y: usize| match (i32::try_from(x), i32::try_from(y)) {
        (Ok(x), Ok(y)) => code.get_module(x, y),
        _ => false,
    };
    let mut used = vec![false; n * n];
    let free = |used: &[bool], x: usize, y: usize| dark(x, y) && !used.get(y * n + x).copied().unwrap_or(true);
    let mut out = Vec::new();
    for y in 0..n {
        for x in 0..n {
            if !free(&used, x, y) {
                continue;
            }
            let mut w = 1;
            while x + w < n && free(&used, x + w, y) {
                w += 1;
            }
            let mut h = 1;
            while y + h < n && (x..x + w).all(|xx| free(&used, xx, y + h)) {
                h += 1;
            }
            for yy in y..y + h {
                for xx in x..x + w {
                    if let Some(u) = used.get_mut(yy * n + xx) {
                        *u = true;
                    }
                }
            }
            out.push((x, y, w, h));
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
