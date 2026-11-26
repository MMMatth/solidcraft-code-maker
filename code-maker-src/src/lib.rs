wit_bindgen::generate!({ path: "wit", world: "command-plugin" });

use craft::plugin::commands;
use craft::plugin::types::{FieldFailure, PluginError};
use exports::craft::plugin::entry::Guest;
use rxing::datamatrix::encoder::SymbolShapeHint;
use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

mod tags;

/// Longest text accepted, in bytes. A version-40 code holds more, but the model would be huge.
const MAX_TEXT: usize = 500;
/// Plate thickness and quiet-zone width (in modules) of a code on a new plate.
const PLATE: f64 = 2.0;
const QR_MARGIN: f64 = 2.0;
/// White margin of an ArUco marker or a reversed AprilTag (whose picture ends on black), in modules.
const TAG_MARGIN: f64 = 1.0;
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
            "code" => make(&params),
            _ => Err(invalid("entry", format!("no entry `{entry}`"))),
        }
    }
}

fn make(params: &Value) -> Result<String, PluginError> {
    let kind = Kind::of(params)?;
    let grid = kind.grid(params)?;
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

    // The outlines are drawn one call each, and a run may make only 256 calls: refuse a code with too many before
    // anything is made.
    let loops = outlines(&grid);
    if loops.len() > MAX_LOOPS {
        return Err(invalid(&kind.text_field(), "too long to draw: use a shorter text or link"));
    }
    let n = grid.w;
    // `size` is the code's width; the plate adds a margin of `grid.margin_x` and `grid.margin_y` modules around it.
    let mut module = size / n as f64;
    // Too small to print: the least that prints well is used instead, and the reply says so.
    let notice = (module < MIN_MODULE).then(|| {
        module = MIN_MODULE;
        format!("the size was raised from {size:.0} mm to {:.1} mm, the least this code prints well at", module * n as f64)
    });
    // A bar code is one row of modules, drawn as bars of the height asked for; the others have square modules.
    let module_h = if grid.h == 1 { number(params, "bar_height", 15.0, 3.0, 200.0)? } else { module };
    let side = (n as f64 + 2.0 * grid.margin_x) * module;
    let side_y = grid.h as f64 * module_h + 2.0 * grid.margin_y * module;

    // Where the sketch lies and how its axes run: on a face, centred on the face's centre
    // (moved by `move_x`, `move_y` and `move_z`); otherwise on top of a new plate.
    // Names the design does not use yet, so a second code beside the first gets a number (`QR code 2`).
    let (label, plate_name, right_edge) = free_names(kind.label(), kind.plate(), face.is_none())?;
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
                        "corner": mv, "length": side, "width": side_y, "height": PLATE})
                .to_string(),
            )?;
            ([mv[0] + side / 2.0, mv[1] + side_y / 2.0, mv[2] + PLATE], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], plate_name.clone())
        }
    };

    // The code is one sketch of rectangles, then one extrusion: it can be edited or moved afterwards.
    commands::run(
        "sketch.create",
        &json!({"name": label, "plane": {"origin": origin, "x_dir": x_dir, "y_dir": y_dir}}).to_string(),
    )?;
    let (half, half_y) = (n as f64 * module / 2.0, grid.h as f64 * module_h / 2.0);
    // The sketch's regions alternate dark and light (a ring's hole is a region of its own), and the
    // app extrudes every region it is given. So the extrusion is told which ones are dark: one per
    // outer loop, by its curves; the loops inside it are its holes.
    let mut dark = Vec::new();
    for (points, outer) in &loops {
        // The grid's y runs down and the sketch's y axis points up.
        let pts: Vec<[f64; 2]> = points
            .iter()
            .map(|&(x, y)| [f64::from(x) * module - half, (grid.h as f64 - f64::from(y)) * module_h - half_y])
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
        (Some(hex), Some(before)) => {
            paint(&target, hex, before, &Frame { origin, x_dir, y_dir, half: (half, half_y) }, height, engrave)?
        }
        (Some(_), None) => (0, Some("the faces of the part could not be read, so the code was not coloured".to_string())),
        (None, _) => (0, None),
    };
    let warning = match (notice, warning) {
        (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        (a, b) => a.or(b),
    };
    Ok(json!({"kind": kind.name(), "modules": n, "rows": grid.h, "module_mm": module, "width_mm": module * n as f64, "side_mm": side, "side_y_mm": side_y, "outlines": loops.len(), "dark_modules": grid.dark_count(),
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
fn face_names(body: &str) -> Result<Option<HashSet<String>>, PluginError> {
    Ok(faces_of(body)?
        .map(|faces| faces.iter().filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_string)).collect()))
}

/// Where the code lies: the centre of its sketch, its axes, and half its width and height.
struct Frame {
    origin: [f64; 3],
    x_dir: [f64; 3],
    y_dir: [f64; 3],
    half: (f64, f64),
}

/// Colours the faces the code made: the top and sides of the raised modules, or the floor and walls
/// of the engraving. A face counts when the extrusion created it (its name is not in `before`), it is
/// flat, it lies inside the code's rectangle, and it is either parallel to the code at exactly the height
/// (or depth) of the code, or square to it within the code's height. That leaves out the part's own
/// faces, even when the code is sunk into the part. Returns how many faces were coloured and, when
/// there is nothing to colour, why.
fn paint(target: &str, hex: &str, before: &HashSet<String>, frame: &Frame, height: f64, engrave: bool) -> Result<(usize, Option<String>), PluginError> {
    let Frame { origin, x_dir, y_dir, half } = *frame;
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
        if name.is_empty() || before.contains(name) {
            continue;
        }
        let rel = [centre[0] - origin[0], centre[1] - origin[1], centre[2] - origin[2]];
        let depth = dot(rel, normal);
        let Some(face_normal) = unit(face_normal) else { continue };
        let along = dot(face_normal, normal);
        let wanted = if engrave { -height } else { height };
        let top = along.abs() > 0.999 && (depth - wanted).abs() <= EPS;
        let wall = along.abs() < 0.001
            && if engrave { depth < -EPS && depth > -height - EPS } else { depth > EPS && depth < height + EPS };
        if (top || wall) && dot(rel, x_dir).abs() <= half.0 + EPS && dot(rel, y_dir).abs() <= half.1 + EPS {
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

/// The names for this run: the `code` and, with a new plate, the `plate` (`QR code` and `QR plate`, say), each with
/// the first number (`QR code 2`, ...) that no feature or body of the design uses. A command that meets a name
/// already taken fails the whole run, so the free names are looked up first. With a new plate it also returns
/// the right-hand edge (largest X) of the bodies the design already holds, if any.
fn free_names(code: &str, plate: &str, with_plate: bool) -> Result<(String, String, Option<f64>), PluginError> {
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
        !taken.contains(&name(code, number).as_str())
            && (!with_plate || !taken.contains(&name(plate, number).as_str()))
    };
    let number = (1..=1000).find(|&n| free(n)).ok_or_else(|| invalid("name", "too many codes of this kind in the design"))?;
    Ok((name(code, number), name(plate, number), right_edge.filter(|_| with_plate)))
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
                let normal = unit(normal)?;
                let off = [point[0] - centre[0], point[1] - centre[1], point[2] - centre[2]];
                // Distance from the point to the face's plane, and from the point to the face's centre.
                dot(off, normal).abs().le(&POINT_TOLERANCE).then(|| (norm(off), f))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, f)| f)
            .ok_or_else(|| bad("the point is not on a flat face (a curved face cannot carry the code)"))?
    } else {
        return Err(bad("the pick needs a face index, a face name or a point on the face"));
    };
    let normal = vec3(face.get("plane_normal")).and_then(unit).ok_or_else(|| bad("the face is not flat"))?;
    let centre = vec3(face.get("centroid")).ok_or_else(|| bad("the face has no centre"))?;
    Ok(Face { body: body.to_string(), normal, centre })
}

fn vec3(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    let f = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite());
    Some([f(0)?, f(1)?, f(2)?])
}

/// The vector scaled to length 1, or `None` when it has no direction.
fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let len = norm(a);
    (len.is_finite() && len > 1e-9).then(|| scale(a, 1.0 / len))
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

/// What the plugin draws: a QR code, an ArUco marker, an AprilTag or another code or barcode.
#[derive(Clone, Copy)]
enum Kind {
    Qr,
    Aruco,
    April,
    /// Another barcode, by its id in `BARCODES`.
    Other(&'static str),
}

/// The codes `rxing` writes besides the QR code: the id of the form, and the name of the code and of its plate.
const BARCODES: [(&str, &str, &str); 12] = [
    ("datamatrix", "Data Matrix", "Data Matrix plate"),
    ("aztec", "Aztec code", "Aztec plate"),
    ("pdf417", "PDF417 code", "PDF417 plate"),
    ("code128", "Code 128", "Code 128 plate"),
    ("code39", "Code 39", "Code 39 plate"),
    ("code93", "Code 93", "Code 93 plate"),
    ("ean13", "EAN-13", "EAN-13 plate"),
    ("ean8", "EAN-8", "EAN-8 plate"),
    ("upca", "UPC-A", "UPC-A plate"),
    ("upce", "UPC-E", "UPC-E plate"),
    ("itf", "ITF", "ITF plate"),
    ("codabar", "Codabar", "Codabar plate"),
];

impl Kind {
    fn of(params: &Value) -> Result<Kind, PluginError> {
        match params.get("kind").and_then(Value::as_str).unwrap_or("qr") {
            "qr" => Ok(Kind::Qr),
            "aruco" => Ok(Kind::Aruco),
            "apriltag" => Ok(Kind::April),
            other => match BARCODES.iter().find(|b| b.0 == other) {
                Some(b) => Ok(Kind::Other(b.0)),
                None => {
                    let all: Vec<&str> = ["qr", "aruco", "apriltag"].into_iter().chain(BARCODES.iter().map(|b| b.0)).collect();
                    Err(invalid("kind", format!("`{other}` is not a kind of code: use one of {}", all.join(", "))))
                }
            },
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Qr => "qr",
            Kind::Aruco => "aruco",
            Kind::April => "apriltag",
            Kind::Other(id) => id,
        }
    }

    /// The row of the barcode table, for a barcode.
    fn barcode(self) -> Option<&'static (&'static str, &'static str, &'static str)> {
        match self {
            Kind::Other(id) => BARCODES.iter().find(|b| b.0 == id),
            _ => None,
        }
    }

    /// The base of the names of the feature and of the plate, numbered when the design has some already.
    fn label(self) -> &'static str {
        match self {
            Kind::Qr => "QR code",
            Kind::Aruco => "ArUco marker",
            Kind::April => "AprilTag",
            Kind::Other(_) => self.barcode().map_or("Code", |b| b.1),
        }
    }

    fn plate(self) -> &'static str {
        match self {
            Kind::Qr => "QR plate",
            Kind::Aruco => "ArUco plate",
            Kind::April => "AprilTag plate",
            Kind::Other(_) => self.barcode().map_or("Code plate", |b| b.2),
        }
    }

    /// The form field that holds what the code says, to name in an error.
    fn text_field(self) -> String {
        match self {
            Kind::Aruco | Kind::April => "id".to_string(),
            Kind::Qr => text_field("qr"),
            Kind::Other(id) => text_field(id),
        }
    }

    /// The grid of dark modules to draw: from the text, or from the family and number of a marker.
    fn grid(self, params: &Value) -> Result<Grid, PluginError> {
        match self {
            Kind::Qr => barcode_grid("qr", params),
            Kind::Aruco => aruco_grid(params),
            Kind::April => april_grid(params),
            Kind::Other(id) => barcode_grid(id, params),
        }
    }
}

/// A picture of dark and light modules, row by row from the top-left, and the width of the light margin
/// a plate needs around it, in modules.
struct Grid {
    w: usize,
    h: usize,
    cells: Vec<bool>,
    margin_x: f64,
    margin_y: f64,
}

impl Grid {
    /// Whether the module is dark; anything outside the grid is light.
    fn dark(&self, x: i32, y: i32) -> bool {
        match (usize::try_from(x), usize::try_from(y)) {
            (Ok(x), Ok(y)) if x < self.w && y < self.h => self.cells.get(y * self.w + x).copied().unwrap_or(false),
            _ => false,
        }
    }

    fn dark_count(&self) -> usize {
        self.cells.iter().filter(|&&d| d).count()
    }

    /// A square grid with the same margin on every side.
    fn square(n: usize, cells: Vec<bool>, margin: f64) -> Grid {
        Grid { w: n, h: n, cells, margin_x: margin, margin_y: margin }
    }
}

/// The form field of a code's text: its own (`ean13_text`) when it has strict rules, else `text`.
fn text_field(id: &str) -> String {
    if RULES.iter().any(|r| r.0 == id) { format!("{id}_text") } else { "text".to_string() }
}

/// The text to encode and the field it came from. A kind with rules of its own has its own field in the form
/// (`ean13_text`), so a mistake is shown on the right one; a script may also give `text` for any kind.
fn text_of(id: &str, params: &Value) -> Result<(String, String), PluginError> {
    let own = text_field(id);
    let given = |f: &str| params.get(f).and_then(Value::as_str).filter(|t| !t.is_empty());
    let (field, raw) = match (given(&own), given("text")) {
        (Some(t), _) => (own, t),
        (None, Some(t)) => ("text".to_string(), t),
        (None, None) => {
            let what = match RULES.iter().find(|r| r.0 == id) {
                Some(r) => format!("enter {}", r.2),
                None => "enter the text or link to encode".to_string(),
            };
            return Err(invalid(&own, what));
        }
    };
    if raw.len() > MAX_TEXT {
        return Err(invalid(&field, format!("at most {MAX_TEXT} bytes")));
    }
    let Some(rule) = RULES.iter().find(|r| r.0 == id) else { return Ok((field, raw.to_string())) };
    // Digits may be typed with spaces or hyphens (5 901234 123457): they are dropped.
    let text: String = if rule.3 { raw.chars().filter(|c| !matches!(c, ' ' | '-')).collect() } else { raw.to_string() };
    if let Some(reason) = (rule.4)(&text) {
        return Err(invalid(&field, format!("{reason} ({})", rule.1)));
    }
    Ok((field, text))
}

type Check = fn(&str) -> Option<String>;

/// Code 39 and Code 93: capital letters, digits, space and `- . $ / + %`.
fn capitals(t: &str) -> Option<String> {
    t.chars().find(|c| !(c.is_ascii_uppercase() || c.is_ascii_digit() || " -.$/+%".contains(*c))).map(|c| format!("`{c}` is not allowed"))
}

/// The rules of the codes that are strict: the id, the rule in a few words, what to enter, whether it is only digits
/// (so spaces and hyphens are dropped) and the check that says what is wrong, if anything.
const RULES: [(&str, &str, &str, bool, Check); 9] = [
    ("code128", "plain ASCII text, like ABC-123456", "plain ASCII text", false, |t| {
        t.chars().find(|c| !c.is_ascii() || c.is_ascii_control()).map(|c| format!("`{c}` is not plain ASCII text"))
    }),
    ("ean13", "12 or 13 digits, like 5901234123457", "12 or 13 digits", true, |t| digits(t, &[12, 13]).or_else(|| gs1_check(t, 13))),
    ("ean8", "7 or 8 digits, like 96385074", "7 or 8 digits", true, |t| digits(t, &[7, 8]).or_else(|| gs1_check(t, 8))),
    ("upca", "11 or 12 digits, like 036000291452", "11 or 12 digits", true, |t| digits(t, &[11, 12]).or_else(|| gs1_check(t, 12))),
    ("upce", "7 or 8 digits starting with 0 or 1, like 01234565", "7 or 8 digits", true, |t| {
        digits(t, &[7, 8]).or_else(|| (!t.starts_with(['0', '1'])).then(|| "a UPC-E starts with 0 or 1".to_string()))
    }),
    ("itf", "an even number of digits, like 123456789012", "an even number of digits", true, |t| {
        digits(t, &[]).or_else(|| (t.len() % 2 != 0).then(|| format!("{} digits is odd: add a 0 in front", t.len())))
    }),
    ("code39", "capital letters, digits, space and - . $ / + %, like ABC-1234", "capital letters and digits", false, capitals),
    ("code93", "capital letters, digits, space and - . $ / + %, like ABC1234", "capital letters and digits", false, capitals),
    ("codabar", "digits and - $ : / . + between a start and a stop letter A to D, like A12345B", "digits between a start and a stop letter A to D", false, |t| {
        let ends = |c: Option<char>| c.is_some_and(|c| matches!(c, 'A'..='D'));
        if !(ends(t.chars().next()) && ends(t.chars().last()) && t.len() >= 3) {
            Some("it must start and end with a letter A to D".to_string())
        } else {
            t[1..t.len() - 1].chars().find(|c| !(c.is_ascii_digit() || "-$:/.+".contains(*c))).map(|c| format!("`{c}` is not allowed"))
        }
    }),
];

/// `Some(reason)` when the text is not made of digits only, or its length is not one of `lengths` (any, if empty).
fn digits(t: &str, lengths: &[usize]) -> Option<String> {
    if let Some(c) = t.chars().find(|c| !c.is_ascii_digit()) {
        return Some(format!("only digits are allowed, not `{c}`"));
    }
    (!lengths.is_empty() && !lengths.contains(&t.len())).then(|| format!("{} digits given", t.len()))
}

/// The GS1 check digit (EAN-13, EAN-8, UPC-A): weights 3 and 1 from the right, before the check digit. When the text
/// has the `full` length, its last digit is the check digit and must be right; the message says the right one.
/// A shorter text has no check digit yet, and `rxing` adds it.
fn gs1_check(t: &str, full: usize) -> Option<String> {
    if t.len() != full {
        return None;
    }
    let body: Vec<u32> = t.chars().filter_map(|c| c.to_digit(10)).collect();
    let (data, last) = body.split_at(full - 1);
    let sum: u32 = data.iter().rev().enumerate().map(|(i, d)| if i % 2 == 0 { d * 3 } else { *d }).sum();
    let right = (10 - sum % 10) % 10;
    let given = last.first().copied().unwrap_or(0);
    (given != right).then(|| format!("the check digit should be {right}, not {given}"))
}

/// The QR code's error correction as `rxing` names it: more of it reads through more damage, in a denser code.
fn correction(params: &Value) -> Result<&'static str, PluginError> {
    match params.get("qr_correction").and_then(Value::as_str).unwrap_or("medium") {
        "low" => Ok("L"),
        "medium" => Ok("M"),
        "quartile" => Ok("Q"),
        "high" => Ok("H"),
        other => Err(invalid("qr_correction", format!("`{other}` is not low, medium, quartile or high"))),
    }
}

/// A code from `rxing` (the QR code too): its modules as the writer gives them, with no margin of its own for a 2D code.
fn barcode_grid(id: &str, params: &Value) -> Result<Grid, PluginError> {
    let (field, text) = text_of(id, params)?;
    let format = match id {
        "qr" => BarcodeFormat::QR_CODE,
        "datamatrix" => BarcodeFormat::DATA_MATRIX,
        "aztec" => BarcodeFormat::AZTEC,
        "pdf417" => BarcodeFormat::PDF_417,
        "code128" => BarcodeFormat::CODE_128,
        "code39" => BarcodeFormat::CODE_39,
        "code93" => BarcodeFormat::CODE_93,
        "ean13" => BarcodeFormat::EAN_13,
        "ean8" => BarcodeFormat::EAN_8,
        "upca" => BarcodeFormat::UPC_A,
        "upce" => BarcodeFormat::UPC_E,
        "itf" => BarcodeFormat::ITF,
        "codabar" => BarcodeFormat::CODABAR,
        other => return Err(invalid("kind", format!("no barcode `{other}`"))),
    };
    // A QR code is written with the error correction asked for (medium by default) and no margin of its own; the
    // others as they come. Width and height 0 ask for the smallest picture, one pixel per module.
    let hints = match id {
        "qr" => EncodeHints { ErrorCorrection: Some(correction(params)?.into()), Margin: Some("0".into()), ..Default::default() },
        // A square Data Matrix: `rxing` may pick a rectangular one, which is valid but rarer and harder to read.
        "datamatrix" => EncodeHints { DataMatrixShape: Some(SymbolShapeHint::FORCE_SQUARE), ..Default::default() },
        _ => EncodeHints::default(),
    };
    let matrix = MultiFormatWriter
        .encode_with_hints(&text, &format, 0, 0, &hints)
        .map_err(|e| invalid(field.as_str(), format!("this cannot be written as {id}: {e}")))?;
    let (w, h) = (matrix.getWidth() as usize, matrix.getHeight() as usize);
    if w == 0 || h == 0 {
        return Err(invalid(field.as_str(), "nothing to draw"));
    }
    let cells = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| matrix.get(x as u32, y as u32)).collect();
    // A bar code has its quiet zone in the picture already; a 2D code gets the margin of a QR code.
    Ok(if h == 1 {
        Grid { w, h, cells, margin_x: 1.0, margin_y: 2.0 }
    } else {
        Grid { w, h, cells, margin_x: QR_MARGIN, margin_y: QR_MARGIN }
    })
}

/// The family asked for (`4x4_50`, `36h11`), with the name of the kind in front tolerated (`aruco_4x4_50`).
fn dictionary(params: &Value, kind: &str, default: &str) -> String {
    // The form has one family list per kind: `aruco_dictionary` or `apriltag_dictionary`.
    let field = format!("{kind}_dictionary");
    let name = params.get(&field).and_then(Value::as_str).filter(|d| !d.is_empty()).unwrap_or(default);
    let name = name.to_ascii_lowercase();
    name.strip_prefix(&format!("{kind}_")).map_or_else(|| name.clone(), str::to_string)
}

/// The marker number asked for, which must be below `count`.
fn marker_id(params: &Value, count: usize) -> Result<usize, PluginError> {
    let id = number(params, "id", 0.0, 0.0, 1_000_000.0)?;
    if id.fract() != 0.0 {
        return Err(invalid("id", "a whole number is expected"));
    }
    // A whole number from 0 to 1,000,000, checked above.
    let id = id as usize;
    if id >= count {
        return Err(invalid("id", format!("this dictionary has {count} markers: use 0 to {}", count - 1)));
    }
    Ok(id)
}

/// An ArUco marker: its bits inside a black border one module wide. A 0 bit is black.
fn aruco_grid(params: &Value) -> Result<Grid, PluginError> {
    let name = dictionary(params, "aruco", "4x4_50");
    let dict = tags::ARUCO.iter().find(|d| d.name == name).ok_or_else(|| {
        let all: Vec<&str> = tags::ARUCO.iter().map(|d| d.name).collect();
        invalid("aruco_dictionary", format!("`{name}` is not an ArUco dictionary: use one of {}", all.join(", ")))
    })?;
    let id = marker_id(params, dict.count)?;
    let bits = dict.size * dict.size;
    let per_marker = bits.div_ceil(8);
    let marker = dict
        .bytes
        .get(id * per_marker..(id + 1) * per_marker)
        .ok_or_else(|| invalid("id", "the dictionary data is too short"))?;
    // Whole bytes hold their bits from the top; the last, partial byte holds the leftover bits on its right.
    let whole = bits / 8 * 8;
    let bit = |k: usize| {
        let (byte, shift) = if k < whole { (k / 8, 7 - k % 8) } else { (bits / 8, bits - 1 - k) };
        marker.get(byte).is_some_and(|b| (b >> shift) & 1 == 1)
    };
    let n = dict.size + 2;
    let mut cells = vec![true; n * n];
    for row in 0..dict.size {
        for col in 0..dict.size {
            if let Some(cell) = cells.get_mut((row + 1) * n + col + 1) {
                *cell = !bit(row * dict.size + col);
            }
        }
    }
    Ok(Grid::square(n, cells, TAG_MARGIN))
}

/// An AprilTag, as the library's own `apriltag_to_image` draws it: black, a one-cell white ring, and
/// white where a bit of the code is set.
fn april_grid(params: &Value) -> Result<Grid, PluginError> {
    let name = dictionary(params, "apriltag", "36h11");
    let fam = tags::APRIL.iter().find(|f| f.name == name).ok_or_else(|| {
        let all: Vec<&str> = tags::APRIL.iter().map(|f| f.name).collect();
        invalid("apriltag_dictionary", format!("`{name}` is not an AprilTag family: use one of {}", all.join(", ")))
    })?;
    let id = marker_id(params, fam.codes.len())?;
    let code = fam.codes.get(id).copied().ok_or_else(|| invalid("id", "the family data is too short"))?;
    let total = fam.total;
    let mut white = vec![false; usize::try_from(total * total).unwrap_or(0)];
    let mut set = |x: i32, y: i32| {
        if let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y))
            && let Some(cell) = white.get_mut(y * total as usize + x)
        {
            *cell = true;
        }
    };
    let ring = fam.width_at_border + if fam.reversed { 0 } else { 2 };
    let start = (total - ring) / 2;
    for i in 0..ring - 1 {
        set(start + i, start);
        set(total - 1 - start, start + i);
        set(start + i + 1, total - 1 - start);
        set(start, start + 1 + i);
    }
    let origin = (total - fam.width_at_border) / 2;
    for (i, &(bx, by)) in fam.bits.iter().enumerate().take(fam.nbits) {
        if (code >> (fam.nbits - i - 1)) & 1 == 1 {
            set(i32::from(bx) + origin, i32::from(by) + origin);
        }
    }
    // A tag whose picture ends on black needs a white margin of its own; the others end on their ring.
    let margin = if fam.reversed { TAG_MARGIN } else { 0.0 };
    Ok(Grid::square(usize::try_from(total).unwrap_or(0), white.into_iter().map(|w| !w).collect(), margin))
}

/// A point of the code's grid, in modules: x to the right, y down, (0, 0) its top-left corner.
type Pt = (i32, i32);

/// The outline of every dark region as closed loops of grid points, with no line between two
/// neighbouring dark modules and none where a loop runs straight on. Dark regions that only touch at
/// a corner are kept as separate loops.
fn outlines(grid: &Grid) -> Vec<(Vec<Pt>, bool)> {
    let (width, height) = (i32::try_from(grid.w).unwrap_or(0), i32::try_from(grid.h).unwrap_or(0));
    let dark = |x: i32, y: i32| grid.dark(x, y);
    // Each dark module's edges run clockwise on screen (the dark side on the right of travel); an
    // edge shared with another dark module cancels out.
    let mut edges: Vec<(Pt, Pt)> = Vec::new();
    for y in 0..height {
        for x in 0..width {
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
        let outer = is_outer(&points);
        loops.push((points, outer));
    }
    loops
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

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(kind: Kind, dictionary: &str, id: usize) -> Grid {
        let params = json!({format!("{}_dictionary", kind.name()): dictionary, "id": id});
        kind.grid(&params).unwrap_or_else(|e| panic!("{dictionary} {id}: {e:?}"))
    }

    /// Writes every grid as lines of `#` (dark) and `.` for a script to decode. Run with
    /// `DUMP_GRIDS=/path cargo test --lib dump_grids -- --ignored`.
    #[test]
    #[ignore]
    fn dump_grids() {
        let mut out = String::new();
        for d in &tags::ARUCO {
            for id in [0, 1, d.count / 2, d.count - 1] {
                let g = grid(Kind::Aruco, d.name, id);
                out += &format!("aruco {} {id} {} {}\n", d.name, g.w, g.margin_x);
                for y in 0..g.w as i32 {
                    out += &(0..g.w as i32).map(|x| if g.dark(x, y) { '#' } else { '.' }).collect::<String>();
                    out += "\n";
                }
            }
        }
        for f in &tags::APRIL {
            for id in [0, 1, f.codes.len() / 2, f.codes.len() - 1] {
                let g = grid(Kind::April, f.name, id);
                out += &format!("april {} {id} {} {}\n", f.name, g.w, g.margin_x);
                for y in 0..g.w as i32 {
                    out += &(0..g.w as i32).map(|x| if g.dark(x, y) { '#' } else { '.' }).collect::<String>();
                    out += "\n";
                }
            }
        }
        std::fs::write(std::env::var("DUMP_GRIDS").unwrap(), out).unwrap();
    }

    /// Writes a grid for each barcode, with the text it says, for `scripts/verify-barcodes.py`. Run with
    /// `DUMP_BARCODES=/path cargo test --lib dump_barcodes -- --ignored`.
    #[test]
    #[ignore]
    fn dump_barcodes() {
        let samples = [
            ("qr", "https://example.com"), ("qr", "https://lintrouvable.fr"), ("qr", "HELLO"), ("qr", "HI"), ("qr", "Bonjour le monde, ceci est un texte un peu plus long pour changer de version du code QR 1234567890"), ("datamatrix", "HELLO-123"), ("datamatrix", "Code Maker"), ("datamatrix", "https://lintrouvable.fr"), ("aztec", "HELLO-123"), ("pdf417", "HELLO-123"),
            ("code128", "ABC-123456"), ("code39", "ABC-1234"), ("code93", "ABC1234"), ("ean13", "5901234123457"),
            ("ean8", "96385074"), ("upca", "036000291452"), ("upce", "01234565"), ("itf", "123456789012"), ("codabar", "A12345B"),
        ];
        let mut out = String::new();
        for (id, text) in samples {
            let g = barcode_grid(id, &json!({"text": text})).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            out += &format!("{id} {} {} {text}\n", g.w, g.h);
            for y in 0..g.h as i32 {
                out += &(0..g.w as i32).map(|x| if g.dark(x, y) { '#' } else { '.' }).collect::<String>();
                out += "\n";
            }
        }
        for level in ["low", "quartile", "high"] {
            let g = barcode_grid("qr", &json!({"text": "https://example.com", "qr_correction": level})).unwrap();
            out += &format!("qr {} {} https://example.com\n", g.w, g.h);
            for y in 0..g.h as i32 {
                out += &(0..g.w as i32).map(|x| if g.dark(x, y) { '#' } else { '.' }).collect::<String>();
                out += "\n";
            }
        }
        std::fs::write(std::env::var("DUMP_BARCODES").unwrap(), out).unwrap();
    }
}
