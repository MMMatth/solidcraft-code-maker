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

    let code = QrCode::encode_text(text, ecc).map_err(|_| invalid("text", "too long for a QR code"))?;
    let n = usize::try_from(code.size()).map_err(|_| invalid("text", "empty code"))?;
    let side = (n as f64 + 2.0 * margin) * module;

    commands::run(
        "solid.box",
        &json!({"name": "QR plate", "body_name": "QR plate",
                "corner": [0, 0, 0], "length": side, "width": side, "height": plate})
        .to_string(),
    )?;

    let rects = merge(&code, n);
    let (z0, dz, op) = if engrave {
        (plate - height, height + 0.5, "cut")
    } else {
        (plate, height, "join")
    };
    // One body per rectangle, then a single combine: a join or cut per rectangle gets slower with
    // every one already in the plate.
    let mut tools = Vec::with_capacity(rects.len());
    for (i, (x, y, w, h)) in rects.iter().enumerate() {
        // Row 0 of a code is its top edge: flip it so the code reads correctly seen from above.
        let x0 = (margin + *x as f64) * module;
        let y0 = (margin + (n - y - h) as f64) * module;
        let name = format!("QR module {i}");
        let args = json!({
            "body_name": name,
            "corner": [x0, y0, z0],
            "length": *w as f64 * module, "width": *h as f64 * module, "height": dz,
        });
        commands::run("solid.box", &args.to_string())?;
        tools.push(name);
    }
    commands::run(
        "solid.combine",
        &json!({"target": "QR plate", "tools": tools, "operation": op}).to_string(),
    )?;
    Ok(json!({"modules": n, "side_mm": side, "boxes": rects.len(), "mode": if engrave { "engrave" } else { "relief" }})
        .to_string())
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
