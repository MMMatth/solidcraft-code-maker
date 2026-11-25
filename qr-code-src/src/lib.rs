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
/// Most outlines one run draws. The host allows 256 calls and the other commands of a run (the design
/// listing, the plate or face lookup, the sketch, the extrusion, the faces and the colour) take 8. The
/// time is what limits first: each outline takes longer than the one before, and 207 take about 14 s
/// of the 30 s a command may run.
const MAX_LOOPS: usize = 200;
/// Space left between a new plate and the bodies already in the design.
const PLATE_GAP: f64 = 5.0;
/// Smallest module that prints legibly with a common 0.4 mm nozzle.
const MIN_MODULE: f64 = 0.4;
/// The named colours of the form, as `#rrggbb`. `none` leaves the code the colour of its body.
const COLORS: [(&str, &str); 7] = [
    ("blue", "#1e6fd9"),
    ("red", "#d93a2f"),
    ("green", "#2e9e4f"),
    ("orange", "#f08a1c"),
    ("yellow", "#f2c40f"),
    ("black", "#1a1a1a"),
    ("white", "#f5f5f5"),
];

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
    if params.get("offset").is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0)) {
        return Err(invalid("offset", "removed: use move_z (or move_x, move_y) to lift or sink the code"));
    }
    // A move along the world axes, for a face or a new plate; turning is only for a face.
    let mut mv = [
        number(params, "move_x", 0.0, -100_000.0, 100_000.0)?,
        number(params, "move_y", 0.0, -100_000.0, 100_000.0)?,
        number(params, "move_z", 0.0, -100_000.0, 100_000.0)?,
    ];
    let face_axes = match params.get("move_axes").and_then(Value::as_str).unwrap_or("world") {
        "world" => false,
        "face" => true,
        other => return Err(invalid("move_axes", format!("`{other}` is not world or face"))),
    };
    let turn = number(params, "rotation", 0.0, -180.0, 180.0)?.to_radians();
    let engrave = match params.get("style").and_then(Value::as_str).unwrap_or("raised") {
        "raised" => false,
        "engraved" => true,
        other => return Err(invalid("style", format!("`{other}` is not raised or engraved"))),
    };
    let color = look(params)?;
    let face = match params.get("face").and_then(Value::as_array).and_then(|a| a.first()) {
        Some(pick) => Some(face_frame(pick)?),
        None => None,
    };
    if face.is_none() && turn != 0.0 {
        return Err(invalid("rotation", "turning needs a face: pick one, or leave Rotate at 0"));
    }
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
    // (moved by `move_x`, `move_y` and `move_z`); otherwise on top of a new plate.
    // Names the design does not use yet, so a second code beside the first gets `QR code 2`.
    let (label, plate_name, right_edge) = free_names(face.is_none())?;
    // A new plate with no move asked goes beside what the design already holds, not on top of it.
    let beside = face.is_none() && mv == [0.0, 0.0, 0.0];
    if let (true, Some(edge)) = (beside, right_edge) {
        mv[0] = edge + PLATE_GAP;
    }
    let (origin, x_dir, y_dir, target) = match &face {
        Some(f) => {
            let (right, up) = axes(f.normal);
            // X, Y, Z are the world axes, or the face's: right, up and out of the face.
            let moved = if face_axes {
                [0, 1, 2].map(|i| right[i] * mv[0] + up[i] * mv[1] + f.normal[i] * mv[2])
            } else {
                mv
            };
            let o = [0, 1, 2].map(|i| f.centre[i] + moved[i]);
            // Turning counter-clockwise seen from outside the face: the axes stay a right-handed pair.
            let (sin, cos) = turn.sin_cos();
            let turned_right = [0, 1, 2].map(|i| cos * right[i] + sin * up[i]);
            let turned_up = [0, 1, 2].map(|i| cos * up[i] - sin * right[i]);
            (o, turned_right, turned_up, f.body.clone())
        }
        None => {
            commands::run(
                "solid.box",
                &json!({"name": plate_name, "body_name": plate_name,
                        "corner": mv, "length": side, "width": side, "height": plate})
                .to_string(),
            )?;
            ([mv[0] + side / 2.0, mv[1] + side / 2.0, mv[2] + plate], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], plate_name.clone())
        }
    };

    // The code is one sketch of rectangles, then one extrusion: it can be edited or moved afterwards.
    commands::run(
        "sketch.create",
        &json!({"name": label, "plane": {"origin": origin, "x_dir": x_dir, "y_dir": y_dir}}).to_string(),
    )?;
    let loops = outlines(&code, n);
    // A plugin may make only 256 host calls in a run, and each loop is one of them.
    if loops.len() > MAX_LOOPS {
        return Err(invalid("text", "too long to draw: use a shorter text or link"));
    }
    let half = n as f64 * module / 2.0;
    // The sketch's regions alternate dark and light (a ring's hole is a region of its own), and the
    // app extrudes every region it is given. So the extrusion is told which ones are dark: one per
    // outer loop, by its curves; the loops inside it are its holes.
    let mut dark = Vec::new();
    for (points, outer) in &loops {
        // The grid's y runs down and the sketch's y axis points up.
        let pts: Vec<[f64; 2]> = points
            .iter()
            .map(|&(x, y)| [f64::from(x) * module - half, (n as f64 - f64::from(y)) * module - half])
            .collect();
        let reply = commands::run("sketch.line", &json!({"points": pts, "closed": true, "connect": false}).to_string())?;
        if *outer {
            let reply: Value = serde_json::from_str(&reply).map_err(|e| invalid("sketch", e))?;
            let curves = reply
                .get("created")
                .and_then(|c| c.get("curves"))
                .cloned()
                .ok_or_else(|| invalid("sketch", "the sketch gave no curve ids"))?;
            dark.push(json!({"curves": curves}));
        }
    }
    commands::run("sketch.finish", "{}")?;
    // The faces the body has before the code: only the ones the extrusion adds get the colour.
    let faces_before = if color.is_some() { face_names(&target)? } else { None };
    // Modules lifted off the face touch nothing, so they are kept in one body explicitly.
    commands::run(
        "solid.extrude",
        &json!({
            "sketch": label, "name": label, "distance": height, "profiles": dark,
            "operation": if engrave { "cut" } else { "join" },
            "direction": if engrave { "negative" } else { "positive" },
            "targets": [target], "touching": "one_body",
        })
        .to_string(),
    )?;
    // The colour is a finishing touch: when the host's face listing is not what is expected, the code
    // is kept and the reply says why it has no colour.
    let (coloured, warning) = match (&color, &faces_before) {
        (Some(hex), Some(before)) => paint(&target, hex, before, origin, x_dir, y_dir, half, height, engrave)?,
        (Some(_), None) => (0, Some("the faces of the part could not be read, so the code was not coloured".to_string())),
        (None, _) => (0, None),
    };
    Ok(json!({"modules": n, "side_mm": side, "outlines": loops.len(), "dark_modules": dark_modules(&code, n),
              "style": if engrave { "engraved" } else { "raised" }, "coloured_faces": coloured, "warning": warning})
    .to_string())
}

/// The colour chosen in the form as `#rrggbb`; `None` means no colour.
fn look(params: &Value) -> Result<Option<String>, PluginError> {
    match params.get("color").and_then(Value::as_str).unwrap_or("black") {
        "none" => Ok(None),
        name => COLORS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, hex)| Some((*hex).to_string()))
            .ok_or_else(|| invalid("color", format!("`{name}` is not a colour of the list"))),
    }
}

/// The faces of a body as the host lists them, or `None` when the reply is not the expected shape.
fn faces_of(body: &str) -> Result<Option<Vec<Value>>, PluginError> {
    let reply = commands::run("model.faces", &json!({"body": body}).to_string())?;
    Ok(serde_json::from_str::<Value>(&reply).ok().and_then(|r| r.get("faces").and_then(Value::as_array).cloned()))
}

/// The names of a body's faces, or `None` when they cannot be read.
fn face_names(body: &str) -> Result<Option<Vec<String>>, PluginError> {
    Ok(faces_of(body)?
        .map(|faces| faces.iter().filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_string)).collect()))
}

/// Colours the faces the code made: the top and sides of the raised modules, or the floor and walls
/// of the engraving. A face counts when the extrusion created it (its name is not in `before`), it is
/// flat, it lies inside the code's square, and it is either parallel to the code at exactly the height
/// (or depth) of the code, or square to it within the code's height. That leaves out the part's own
/// faces, even when the code is sunk into the part. Returns how many faces were coloured and, when
/// there is nothing to colour, why.
#[allow(clippy::too_many_arguments)]
fn paint(
    target: &str,
    hex: &str,
    before: &[String],
    origin: [f64; 3],
    x_dir: [f64; 3],
    y_dir: [f64; 3],
    half: f64,
    height: f64,
    engrave: bool,
) -> Result<(usize, Option<String>), PluginError> {
    const EPS: f64 = 1e-3;
    let Some(faces) = faces_of(target)? else {
        return Ok((0, Some("the faces of the part could not be read, so the code was not coloured".to_string())));
    };
    let normal = cross(x_dir, y_dir);
    let mut spots = Vec::new();
    for face in &faces {
        let name = face.get("name").and_then(Value::as_str).unwrap_or("");
        let (Some(centre), Some(point), Some(face_normal)) =
            (vec3(face.get("centroid")), vec3(face.get("point")), vec3(face.get("plane_normal")))
        else {
            continue;
        };
        if name.is_empty() || before.iter().any(|b| b == name) {
            continue;
        }
        let rel = [centre[0] - origin[0], centre[1] - origin[1], centre[2] - origin[2]];
        let depth = dot(rel, normal);
        let len = norm(face_normal);
        if len.is_nan() || len <= 1e-9 {
            continue;
        }
        let along = dot(face_normal, normal) / len;
        let wanted = if engrave { -height } else { height };
        let top = along.abs() > 0.999 && (depth - wanted).abs() <= EPS;
        let wall = along.abs() < 0.001
            && if engrave { depth < -EPS && depth > -height - EPS } else { depth > EPS && depth < height + EPS };
        if (top || wall) && dot(rel, x_dir).abs() <= half + EPS && dot(rel, y_dir).abs() <= half + EPS {
            spots.push(json!({"point": point, "body": target}));
        }
    }
    if spots.is_empty() {
        return Ok((0, Some("no face of the code was found to colour".to_string())));
    }
    let count = spots.len();
    commands::run("appearance.assign", &json!({"faces": spots, "color": hex}).to_string())?;
    Ok((count, None))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The names for this run: `QR code` and, with a new plate, `QR plate`, each with the first number
/// (`QR code 2`, ...) that no feature or body of the design uses. A command that meets a name already
/// taken fails the whole run, so the free names are looked up first. With a new plate it also returns
/// the right-hand edge (largest X) of the bodies the design already holds, if any.
fn free_names(with_plate: bool) -> Result<(String, String, Option<f64>), PluginError> {
    // Bodies are measured only to find where a new plate can go.
    let reply = commands::run(
        "document.inspect",
        &json!({"timeline": "summary", "bodies": true, "measure": with_plate}).to_string(),
    )?;
    let reply: Value = serde_json::from_str(&reply).map_err(|e| invalid("name", e))?;
    let mut taken: Vec<&str> = Vec::new();
    let mut right_edge: Option<f64> = None;
    // Every sketch and feature is in the timeline; the bodies are listed apart.
    for list in ["timeline", "bodies"] {
        let items = reply
            .get(list)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("name", format!("the design listing has no `{list}`")))?;
        for item in items {
            taken.extend(item.get("name").and_then(Value::as_str));
            if list == "bodies" {
                let max_x = item.get("bbox").and_then(|b| b.get("max")).and_then(|m| m.get(0)).and_then(Value::as_f64);
                right_edge = match (right_edge, max_x) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
            }
        }
    }
    let name = |base: &str, number: usize| if number == 1 { base.to_string() } else { format!("{base} {number}") };
    let free = |number: usize| {
        !taken.contains(&name("QR code", number).as_str())
            && (!with_plate || !taken.contains(&name("QR plate", number).as_str()))
    };
    let number = (1..=1000).find(|&n| free(n)).ok_or_else(|| invalid("name", "too many QR codes in the design"))?;
    Ok((name("QR code", number), name("QR plate", number), right_edge.filter(|_| with_plate)))
}

/// A picked planar face: its body, outward normal and centre.
struct Face {
    body: String,
    normal: [f64; 3],
    centre: [f64; 3],
}

/// How far from a face's plane a picked point may be, in mm: clients round their numbers.
const POINT_TOLERANCE: f64 = 0.01;

/// Looks the picked face up. A pick names a body and the face by its `index` (what the dialog sends),
/// its `name`, or a `point` on it (what an MCP client usually sends); `model.faces` gives its plane.
/// With a point, the flat faces whose plane holds the point are the candidates, and the one whose
/// centre is nearest wins.
fn face_frame(pick: &Value) -> Result<Face, PluginError> {
    let bad = |why: &str| invalid("face", why.to_string());
    let body = pick.get("body").and_then(Value::as_str).ok_or_else(|| bad("pick a face of a body"))?;
    let reply = commands::run("model.faces", &json!({"body": body}).to_string())?;
    let reply: Value = serde_json::from_str(&reply).map_err(|e| invalid("face", e))?;
    let faces = reply.get("faces").and_then(Value::as_array).ok_or_else(|| bad("the body has no faces"))?;
    let face = if let Some(index) = pick.get("index").and_then(Value::as_u64) {
        faces.iter().find(|f| f.get("index").and_then(Value::as_u64) == Some(index)).ok_or_else(|| bad("no such face"))?
    } else if let Some(name) = pick.get("name").and_then(Value::as_str) {
        faces.iter().find(|f| f.get("name").and_then(Value::as_str) == Some(name)).ok_or_else(|| bad("no face has that name"))?
    } else if let Some(point) = vec3(pick.get("point")) {
        faces
            .iter()
            .filter_map(|f| {
                let (normal, centre) = (vec3(f.get("plane_normal"))?, vec3(f.get("centroid"))?);
                let len = norm(normal);
                if len.is_nan() || len <= 1e-9 {
                    return None;
                }
                let off = [point[0] - centre[0], point[1] - centre[1], point[2] - centre[2]];
                // Distance from the point to the face's plane, and from the point to the face's centre.
                (dot(off, normal) / len).abs().le(&POINT_TOLERANCE).then(|| (norm(off), f))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, f)| f)
            .ok_or_else(|| bad("the point is not on a flat face (a curved face cannot carry the code)"))?
    } else {
        return Err(bad("the pick needs a face index, a face name or a point on the face"));
    };
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
fn outlines(code: &QrCode, n: usize) -> Vec<(Vec<Pt>, bool)> {
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
        let points = straighten(points);
        loops.push((points.clone(), is_outer(&points)));
    }
    loops
}

/// How many modules of the code are dark.
fn dark_modules(code: &QrCode, n: usize) -> usize {
    let size = i32::try_from(n).unwrap_or(0);
    (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).filter(|&(x, y)| code.get_module(x, y)).count()
}

/// Whether a loop encloses dark modules (an outer edge) rather than a light hole. The loops run with
/// the dark side on the right, which is clockwise on screen, where y runs down: a positive area.
fn is_outer(points: &[Pt]) -> bool {
    let len = points.len();
    let twice_area: i64 = (0..len)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % len)?);
            Some(i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1))
        })
        .sum();
    twice_area > 0
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
