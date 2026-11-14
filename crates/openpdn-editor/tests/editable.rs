use openpdn_editor::Editor;
use openpdn_formats::json::Json;

fn req(e: &mut Editor, j: &str) -> Json {
    req_bin(e, j, &[])
}
fn req_bin(e: &mut Editor, j: &str, bin: &[u8]) -> Json {
    let r = e.handle(&Json::parse(j).unwrap(), bin).to_json();
    assert_eq!(r.get("ok"), Some(&Json::Bool(true)), "{j} -> {r:?}");
    r
}
fn ptr(e: &mut Editor, phase: &str, x: i32, y: i32, button: i32) -> Json {
    req(e, &format!(r#"{{"cmd":"pointer","phase":"{phase}","x":{x},"y":{y},"button":{button},"handleRadius":3}}"#))
}
fn drag(e: &mut Editor, (x0, y0): (i32, i32), (x1, y1): (i32, i32)) {
    ptr(e, "down", x0, y0, 0);
    ptr(e, "move", (x0 + x1) / 2, (y0 + y1) / 2, 0);
    ptr(e, "up", x1, y1, 0);
}
fn px(e: &Editor, x: i32, y: i32) -> (u8, u8, u8, u8) {
    let c = e.document().layers[0].surface.get(x, y);
    (c.r, c.g, c.b, c.a)
}
fn history(e: &mut Editor) -> Vec<String> {
    let r = req(e, r#"{"cmd":"init"}"#);
    match r.get("state").unwrap().get("history").unwrap().get("items").unwrap() {
        Json::Arr(a) => a.iter().map(|v| v.as_str().unwrap().to_owned()).collect(),
        _ => panic!(),
    }
}
fn pending_points(e: &mut Editor) -> usize {
    let r = req(e, r#"{"cmd":"init"}"#);
    match r.get("state").unwrap().get("pending").unwrap().get("points") {
        Some(Json::Arr(a)) => a.len(),
        _ => 0,
    }
}
const WHITE: (u8, u8, u8, u8) = (255, 255, 255, 255);
const BLACK: (u8, u8, u8, u8) = (0, 0, 0, 255);

fn line_editor() -> Editor {
    let mut e = Editor::new(100, 60);
    req(&mut e, r#"{"cmd":"tool","name":"Line"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":3}"#);
    e
}

#[test]
fn line_stays_editable_until_committed() {
    let mut e = line_editor();
    let original = e.document().layers[0].surface.pixels().to_vec();
    drag(&mut e, (10, 30), (90, 30));
    assert_eq!(px(&e, 50, 30), BLACK, "drawn live");
    assert!(e.has_pending());
    assert!(history(&mut e).is_empty(), "no history step until committed");
    // Move it without leaving a trace.
    req(&mut e, r#"{"cmd":"shape_translate","phase":"start","dx":0,"dy":0}"#);
    req(&mut e, r#"{"cmd":"shape_translate","phase":"move","dx":0,"dy":15}"#);
    assert_eq!(px(&e, 50, 30), WHITE, "old position restored");
    assert_eq!(px(&e, 50, 45), BLACK);
    // Width changes re-render live.
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":9}"#);
    assert_eq!(px(&e, 50, 49), BLACK);
    // Escape cancels: back to the original pixels exactly.
    req(&mut e, r#"{"cmd":"shape_cancel"}"#);
    assert_eq!(e.document().layers[0].surface.pixels(), &original[..]);
    assert!(!e.has_pending());
    // Enter commits as one step.
    drag(&mut e, (10, 10), (90, 10));
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!(history(&mut e), vec!["Line/Curve"]);
    // Another image action commits too.
    drag(&mut e, (10, 50), (90, 50));
    req(&mut e, r#"{"cmd":"select_all"}"#);
    assert_eq!(history(&mut e), vec!["Line/Curve", "Line/Curve", "Select All"]);
    assert_eq!(px(&e, 50, 50), BLACK);
}

#[test]
fn bending_lines_and_curve_types() {
    let mut e = line_editor();
    drag(&mut e, (10, 40), (90, 40));
    // Dragging the middle of the line adds a bend point.
    ptr(&mut e, "down", 50, 40, 0);
    ptr(&mut e, "move", 50, 10, 0);
    ptr(&mut e, "up", 50, 10, 0);
    assert_eq!(pending_points(&mut e), 3);
    assert_eq!(px(&e, 50, 40), WHITE, "the straight part moved away");
    assert_eq!(px(&e, 50, 11), BLACK, "bent to the new point");
    assert_eq!(px(&e, 30, 25), BLACK, "straight segment from the start to the bend");
    // Spline: smooth curve through the same points.
    req(&mut e, r#"{"cmd":"setting","key":"curve","value":"Spline"}"#);
    assert_eq!(px(&e, 50, 11), BLACK);
    assert_eq!(px(&e, 30, 25), WHITE, "the spline bows out above the straight segment");
    // Right-click on the bend point removes it.
    ptr(&mut e, "down", 50, 10, 2);
    ptr(&mut e, "up", 50, 10, 2);
    assert_eq!(pending_points(&mut e), 2);
    assert_eq!(px(&e, 50, 40), BLACK);
    // Bézier: four points; dragging a control point bends the curve.
    req(&mut e, r#"{"cmd":"setting","key":"curve","value":"Bezier"}"#);
    assert_eq!(pending_points(&mut e), 4);
    ptr(&mut e, "down", 37, 40, 0); // first control point (1/3 along)
    ptr(&mut e, "move", 37, 0, 0);
    ptr(&mut e, "up", 37, 0, 0);
    assert_eq!(px(&e, 50, 40), WHITE);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!(history(&mut e), vec!["Line/Curve"]);
}

#[test]
fn clicking_away_starts_a_new_line() {
    let mut e = line_editor();
    drag(&mut e, (10, 10), (90, 10));
    drag(&mut e, (10, 50), (90, 50));
    assert_eq!(history(&mut e), vec!["Line/Curve"], "the first line was committed");
    assert_eq!((px(&e, 50, 10), px(&e, 50, 50)), (BLACK, BLACK));
    // A click without a drag commits and draws nothing new.
    ptr(&mut e, "down", 50, 30, 0);
    ptr(&mut e, "up", 50, 30, 0);
    assert!(!e.has_pending());
    assert_eq!(px(&e, 50, 30), WHITE);
}

#[test]
fn live_text_preview_is_non_destructive() {
    let mut e = Editor::new(40, 20);
    let original = e.document().layers[0].surface.pixels().to_vec();
    let red = [255u8, 0, 0, 255].repeat(4 * 3);
    req_bin(&mut e, r#"{"cmd":"text_preview","x":2,"y":2,"w":4,"h":3}"#, &red);
    assert_eq!(px(&e, 3, 3), (255, 0, 0, 255));
    req_bin(&mut e, r#"{"cmd":"text_preview","x":20,"y":10,"w":4,"h":3}"#, &red);
    assert_eq!(px(&e, 3, 3), WHITE, "moved: the old spot is restored");
    assert_eq!(px(&e, 21, 11), (255, 0, 0, 255));
    req(&mut e, r#"{"cmd":"text_cancel"}"#);
    assert_eq!(e.document().layers[0].surface.pixels(), &original[..]);
    req_bin(&mut e, r#"{"cmd":"text_preview","x":2,"y":2,"w":4,"h":3}"#, &red);
    req(&mut e, r#"{"cmd":"undo"}"#); // another action commits, then undoes it
    assert_eq!(e.document().layers[0].surface.pixels(), &original[..]);
    req_bin(&mut e, r#"{"cmd":"text_preview","x":2,"y":2,"w":4,"h":3}"#, &red);
    req(&mut e, r#"{"cmd":"text_commit"}"#);
    assert_eq!(history(&mut e), vec!["Text"]);
}

fn pending(e: &mut Editor) -> Json {
    req(e, r#"{"cmd":"init"}"#).get("state").unwrap().get("pending").unwrap().clone()
}
fn f(j: &Json, k: &str) -> f64 {
    j.get(k).and_then(Json::as_f64).unwrap()
}
fn black_count(e: &Editor) -> usize {
    e.document().layers[0].surface.pixels().iter().filter(|p| p.r < 128).count()
}

#[test]
fn shapes_stay_editable_move_resize_rotate() {
    let mut e = Editor::new(120, 100);
    req(&mut e, r#"{"cmd":"tool","name":"Rectangle"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":2}"#);
    let original = e.document().layers[0].surface.pixels().to_vec();
    drag(&mut e, (20, 20), (59, 49)); // 40×30 box at (20,20)
    let p = pending(&mut e);
    assert_eq!(p.get("kind").and_then(Json::as_str), Some("shape"));
    assert_eq!((f(&p, "cx"), f(&p, "cy"), f(&p, "w"), f(&p, "h")), (40.0, 35.0, 40.0, 30.0));
    assert_eq!(px(&e, 20, 20), BLACK);
    assert!(history(&mut e).is_empty());
    // Drag inside to move by (10, 5).
    drag(&mut e, (40, 35), (50, 40));
    let p = pending(&mut e);
    assert_eq!((f(&p, "cx"), f(&p, "cy")), (50.0, 40.0));
    assert_eq!(px(&e, 20, 20), WHITE, "moved without leaving a trace");
    assert_eq!(px(&e, 30, 25), BLACK);
    // Drag the bottom-right handle (at the corner, pixel 70,55) by (20, 10).
    drag(&mut e, (70, 55), (90, 65));
    let p = pending(&mut e);
    assert_eq!((f(&p, "cx"), f(&p, "cy"), f(&p, "w"), f(&p, "h")), (60.0, 45.0, 60.0, 40.0));
    assert_eq!(px(&e, 89, 64), BLACK);
    // Rotate 90° with the handle above the top edge (radius 3 → 12 px above).
    let top = (60, 25 - 12);
    drag(&mut e, top, (60 + 40, 45));
    let p = pending(&mut e);
    assert!((f(&p, "angle") - std::f64::consts::FRAC_PI_2).abs() < 0.05, "angle {}", f(&p, "angle"));
    assert_eq!(px(&e, 89, 64), WHITE, "rotated: the old corner is gone");
    assert_eq!(px(&e, 41, 74), BLACK, "now 40 wide, 60 tall, around (60, 45)");
    // Esc restores everything; Enter commits one step.
    req(&mut e, r#"{"cmd":"shape_cancel"}"#);
    assert_eq!(e.document().layers[0].surface.pixels(), &original[..]);
    drag(&mut e, (20, 20), (59, 49));
    req(&mut e, r#"{"cmd":"shape_nudge","dx":-1,"dy":0}"#);
    assert_eq!(px(&e, 19, 20), BLACK, "nudged one pixel left");
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!(history(&mut e), vec!["Rectangle"]);
}

#[test]
fn unrotated_editable_shapes_match_classic_rendering() {
    for tool in ["Rectangle", "Rounded Rect", "Ellipse"] {
        let render = |editable: bool| {
            let mut e = Editor::new(80, 60);
            req(&mut e, &format!(r#"{{"cmd":"tool","name":"{tool}"}}"#));
            req(&mut e, r#"{"cmd":"setting","key":"brush","value":5}"#);
            req(&mut e, r#"{"cmd":"setting","key":"shape","value":"FillOutline"}"#);
            if !editable {
                e.tools_mut().settings.editable_shapes = false;
            }
            drag(&mut e, (5, 7), (70, 50));
            req(&mut e, r#"{"cmd":"shape_commit"}"#);
            e.document().layers[0].surface.pixels().to_vec()
        };
        assert_eq!(render(true), render(false), "{tool}");
    }
}

#[test]
fn line_caps() {
    let draw = |cap_start: &str, cap_end: &str| {
        let mut e = line_editor();
        req(&mut e, r#"{"cmd":"setting","key":"brush","value":6}"#);
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"capStart","value":"{cap_start}"}}"#));
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"capEnd","value":"{cap_end}"}}"#));
        drag(&mut e, (20, 30), (80, 30));
        e
    };
    let flat = draw("Flat", "Flat");
    assert_eq!(px(&flat, 18, 30), WHITE, "flat: nothing beyond the end point");
    assert_eq!(px(&flat, 21, 30), BLACK);
    let square = draw("Square", "Square");
    assert_eq!(px(&square, 18, 32), BLACK, "square: extends half the width, corners square");
    let round = draw("Round", "Round");
    assert_eq!(px(&round, 18, 30), BLACK);
    assert_eq!(px(&round, 17, 33), WHITE, "round: corners cut");
    assert_eq!(px(&square, 17, 33).0, 127, "square: half-covered there");
    let arrow = draw("Flat", "Arrow");
    assert_eq!(px(&arrow, 64, 37), BLACK, "arrowhead wider than the line");
    assert_eq!(px(&arrow, 78, 30), BLACK, "tip at the end point");
    assert_eq!(px(&arrow, 83, 30), WHITE);
    let open = draw("Flat", "ArrowOpen");
    assert_eq!(px(&open, 63, 39), BLACK, "spoke");
    assert_eq!(px(&open, 84, 30), WHITE);
    assert_ne!(open.document().layers[0].surface.pixels(), arrow.document().layers[0].surface.pixels());
    let tail = draw("ArrowIn", "Flat");
    assert_eq!(px(&tail, 21, 37), BLACK, "inward arrow is widest at the end point");
    let diamond = draw("Diamond", "Circle");
    assert!(black_count(&diamond) > black_count(&flat) + 100);
    // Caps update live on a pending line.
    let mut e = draw("Flat", "Flat");
    req(&mut e, r#"{"cmd":"setting","key":"capEnd","value":"Arrow"}"#);
    assert_eq!(px(&e, 64, 37), BLACK);
}

#[test]
fn rotate_and_nudge_lines() {
    let mut e = line_editor();
    drag(&mut e, (20, 30), (80, 30));
    req(&mut e, r#"{"cmd":"shape_rotate","phase":"start","angle":0}"#);
    req(&mut e, &format!(r#"{{"cmd":"shape_rotate","phase":"move","angle":{}}}"#, std::f64::consts::FRAC_PI_2));
    assert_eq!(px(&e, 20, 30), WHITE);
    assert_eq!(px(&e, 50, 5), BLACK, "now vertical through the centre");
    assert_eq!(px(&e, 50, 55), BLACK);
    req(&mut e, r#"{"cmd":"shape_nudge","dx":3,"dy":0}"#);
    assert_eq!(px(&e, 53, 5), BLACK);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!(history(&mut e), vec!["Line/Curve"]);
}

/// Writes /tmp/caps.png with every cap pair and some rotated shapes (run with --ignored).
#[test]
#[ignore]
fn render_caps_sheet() {
    let mut e = Editor::new(420, 330);
    req(&mut e, r#"{"cmd":"tool","name":"Line"}"#);
    let caps = ["Flat", "Square", "Round", "Arrow", "ArrowOpen", "ArrowIn", "Diamond", "Circle"];
    for (i, c) in caps.iter().enumerate() {
        for (j, w) in [2, 8].iter().enumerate() {
            req(&mut e, &format!(r#"{{"cmd":"setting","key":"brush","value":{w}}}"#));
            req(&mut e, &format!(r#"{{"cmd":"setting","key":"capStart","value":"{c}"}}"#));
            req(&mut e, &format!(r#"{{"cmd":"setting","key":"capEnd","value":"{c}"}}"#));
            let y = 20 + i as i32 * 26;
            let x = 20 + j as i32 * 130;
            drag(&mut e, (x, y), (x + 100, y));
            req(&mut e, r#"{"cmd":"shape_commit"}"#);
        }
    }
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":4}"#);
    req(&mut e, r#"{"cmd":"setting","key":"curve","value":"Bezier"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"capStart","value":"Circle"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"capEnd","value":"Arrow"}"#);
    drag(&mut e, (300, 30), (400, 120));
    ptr(&mut e, "down", 333, 60, 0);
    ptr(&mut e, "move", 380, 20, 0);
    ptr(&mut e, "up", 380, 20, 0);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    for (k, tool) in ["Rectangle", "Rounded Rect", "Ellipse"].iter().enumerate() {
        req(&mut e, &format!(r#"{{"cmd":"tool","name":"{tool}"}}"#));
        req(&mut e, r#"{"cmd":"setting","key":"shape","value":"FillOutline"}"#);
        req(&mut e, r#"{"cmd":"colors","fg":"203080","bg":"F0C040"}"#);
        let (x, y) = (290 + (k as i32 % 2) * 60, 150 + (k as i32 / 2) * 90);
        drag(&mut e, (x, y), (x + 50, y + 30));
        req(&mut e, &format!(r#"{{"cmd":"shape_rotate","phase":"start","angle":0}}"#));
        req(&mut e, &format!(r#"{{"cmd":"shape_rotate","phase":"move","angle":{}}}"#, 0.5 + k as f64 * 0.3));
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
        req(&mut e, r#"{"cmd":"colors","fg":"000000","bg":"FFFFFF"}"#);
    }
    let png = openpdn_formats::png::encode(&e.document().layers[0].surface, &[], openpdn_formats::png::Compression::Fast);
    std::fs::write("/tmp/caps.png", png).unwrap();
}

const SHAPES: [&str; 33] = ["Rectangle", "RoundedRectangle", "Ellipse", "Diamond", "Triangle", "RightTriangle", "Trapezoid", "Parallelogram",
    "Pentagon", "Hexagon", "Octagon", "Polygon", "Star", "CustomPolygon", "Plus", "Cross", "Arrow", "DoubleArrow", "Chevron", "Heart",
    "Callout", "Moon", "Lightning", "Terminator", "Document", "PredefinedProcess", "ManualInput", "ManualOperation", "Preparation",
    "Delay", "OffPageConnector", "Database", "Merge"];

#[test]
fn every_shape_type_draws_inside_its_box() {
    for t in SHAPES {
        let mut e = Editor::new(80, 60);
        req(&mut e, r#"{"cmd":"tool","name":"Shape"}"#);
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"shapeType","value":"{t}"}}"#));
        req(&mut e, r#"{"cmd":"setting","key":"shape","value":"FillOutline"}"#);
        req(&mut e, r#"{"cmd":"setting","key":"brush","value":3}"#);
        req(&mut e, r#"{"cmd":"colors","fg":"000000","bg":"FF0000"}"#);
        drag(&mut e, (10, 10), (69, 49)); // box 10..70 × 10..50
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
        let l = &e.document().layers[0].surface;
        let mut painted = 0;
        for y in 0..60 {
            for x in 0..80 {
                let c = l.get(x, y);
                if (c.r, c.g, c.b) != (255, 255, 255) {
                    painted += 1;
                    assert!((10..70).contains(&x) && (10..50).contains(&y), "{t}: painted outside its box at {x},{y}");
                }
            }
        }
        assert!(painted > 300, "{t}: only {painted} pixels");
        assert_eq!(history(&mut e), vec!["Shape"], "{t}");
    }
}

#[test]
fn custom_polygon_corners_and_stepwise_undo() {
    let mut e = Editor::new(100, 100);
    req(&mut e, r#"{"cmd":"tool","name":"Shape"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shapeType","value":"CustomPolygon"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shapeSides","value":4}"#);
    drag(&mut e, (20, 20), (79, 79));
    let p = pending(&mut e);
    let corners = |p: &Json| match p.get("vertices") { Some(Json::Arr(a)) => a.len(), _ => 0 };
    assert_eq!(corners(&p), 4, "a diamond-shaped 4-gon");
    // Drag the top corner (50, 20) up to (50, 5): the box refits around it.
    drag(&mut e, (49, 20), (49, 5));
    let p = pending(&mut e);
    assert!(f(&p, "h") > 70.0, "taller now: {}", f(&p, "h"));
    // Drag an edge to add a corner.
    drag(&mut e, (35, 65), (10, 90));
    assert_eq!(corners(&pending(&mut e)), 5);
    // Ctrl+Z steps back one edit at a time, then removes the shape; Ctrl+Y redoes.
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert_eq!(corners(&pending(&mut e)), 4);
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert!((f(&pending(&mut e), "h") - 60.0).abs() < 0.5, "back to the original height");
    req(&mut e, r#"{"cmd":"redo"}"#);
    assert!(f(&pending(&mut e), "h") > 70.0);
    req(&mut e, r#"{"cmd":"undo"}"#);
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert_eq!(pending(&mut e), Json::Null, "undoing past the first edit removes the shape");
    assert!(history(&mut e).is_empty());
    assert_eq!(e.document().layers[0].surface.pixels().iter().filter(|c| c.r != 255).count(), 0);
}

#[test]
fn line_bends_undo_one_at_a_time() {
    let mut e = line_editor();
    drag(&mut e, (10, 40), (90, 40));
    ptr(&mut e, "down", 50, 40, 0);
    ptr(&mut e, "move", 50, 10, 0);
    ptr(&mut e, "up", 50, 10, 0);
    req(&mut e, r#"{"cmd":"shape_nudge","dx":5,"dy":0}"#);
    assert_eq!(pending_points(&mut e), 3);
    req(&mut e, r#"{"cmd":"undo"}"#); // the nudge
    assert_eq!(px(&e, 50, 11), BLACK);
    req(&mut e, r#"{"cmd":"undo"}"#); // the bend
    assert_eq!(pending_points(&mut e), 2);
    assert_eq!(px(&e, 50, 40), BLACK);
    req(&mut e, r#"{"cmd":"redo"}"#);
    assert_eq!(pending_points(&mut e), 3);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    req(&mut e, r#"{"cmd":"undo"}"#); // after committing, undo removes the whole line
    assert_eq!(px(&e, 50, 11), WHITE);
}

#[test]
fn dashes_and_fill_patterns() {
    let mut e = line_editor();
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":4}"#);
    req(&mut e, r#"{"cmd":"setting","key":"dash","value":"Dash"}"#);
    drag(&mut e, (10, 30), (90, 30));
    let row: Vec<bool> = (10..90).map(|x| px(&e, x, 30) == BLACK).collect();
    let runs = row.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(runs >= 6, "dashed: {runs} on/off changes");
    req(&mut e, r#"{"cmd":"setting","key":"dash","value":"Dot"}"#);
    assert!((10..90).any(|x| px(&e, x, 30) == WHITE) && px(&e, 10, 30) == BLACK);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    // Patterned shape fill: stripes of the fill colour, the image showing through.
    let mut e = Editor::new(60, 60);
    req(&mut e, r#"{"cmd":"tool","name":"Shape"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shape","value":"Fill"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"fillStyle","value":"Horizontal"}"#);
    req(&mut e, r#"{"cmd":"colors","fg":"000000","bg":"000000"}"#);
    drag(&mut e, (5, 5), (54, 54));
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!((px(&e, 30, 16), px(&e, 30, 19)), (BLACK, WHITE));
    // Patterned bucket fill.
    let mut e = Editor::new(20, 20);
    req(&mut e, r#"{"cmd":"tool","name":"Fill"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"fillStyle","value":"Vertical"}"#);
    req(&mut e, r#"{"cmd":"colors","fg":"000000"}"#);
    drag(&mut e, (5, 5), (5, 5));
    assert_eq!((px(&e, 8, 3), px(&e, 11, 3)), (BLACK, WHITE));
}

/// Writes /tmp/shapes.png with every shape type (run with --ignored).
#[test]
#[ignore]
fn render_shapes_sheet() {
    let mut e = Editor::new(560, 360);
    req(&mut e, r#"{"cmd":"tool","name":"Shape"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shape","value":"FillOutline"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":3}"#);
    req(&mut e, r#"{"cmd":"colors","fg":"203080","bg":"F0C040"}"#);
    for (i, t) in SHAPES.iter().enumerate() {
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"shapeType","value":"{t}"}}"#));
        let (x, y) = (10 + (i as i32 % 8) * 68, 10 + (i as i32 / 8) * 70);
        drag(&mut e, (x, y), (x + 59, y + 49));
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
    }
    req(&mut e, r#"{"cmd":"setting","key":"fillStyle","value":"DiagonalCross"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shapeType","value":"Star"}"#);
    drag(&mut e, (350, 290), (409, 349));
    req(&mut e, r#"{"cmd":"shape_rotate","phase":"start","angle":0}"#);
    req(&mut e, r#"{"cmd":"shape_rotate","phase":"move","angle":0.3}"#);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"fillStyle","value":"Solid"}"#);
    req(&mut e, r#"{"cmd":"tool","name":"Line"}"#);
    for (k, d) in ["Dash", "LongDash", "Dot", "DashDot", "DashDotDot"].iter().enumerate() {
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"dash","value":"{d}"}}"#));
        let y = 300 + k as i32 * 12;
        drag(&mut e, (430, y), (550, y));
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
    }
    let png = openpdn_formats::png::encode(&e.document().layers[0].surface, &[], openpdn_formats::png::Compression::Fast);
    std::fs::write("/tmp/shapes.png", png).unwrap();
}


#[test]
fn pattern_fill_does_not_change_the_outline() {
    let render = |style: &str| {
        let mut e = Editor::new(120, 100);
        req(&mut e, r#"{"cmd":"tool","name":"Shape"}"#);
        req(&mut e, r#"{"cmd":"setting","key":"shapeType","value":"Hexagon"}"#);
        req(&mut e, r#"{"cmd":"setting","key":"shape","value":"FillOutline"}"#);
        req(&mut e, r#"{"cmd":"setting","key":"brush","value":6}"#);
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"fillStyle","value":"{style}"}}"#));
        drag(&mut e, (10, 10), (109, 89));
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
        e.document().layers[0].surface.pixels().to_vec()
    };
    // Fill colour is white on a white image: the pattern must be invisible.
    assert_eq!(render("Solid"), render("DiagonalCross"));
}

/// Writes /tmp/thick.png: thick-stroked circles via Shape and Ellipse tools (run with --ignored).
#[test]
#[ignore]
fn render_thick_circles() {
    let mut e = Editor::new(420, 160);
    for (k, (tool, ty, mode, w)) in [("Shape", "Ellipse", "Outline", 30), ("Shape", "Ellipse", "FillOutline", 60), ("Shape", "RoundedRectangle", "Outline", 70), ("Shape", "Ellipse", "Outline", 120)].iter().enumerate() {
        req(&mut e, &format!(r#"{{"cmd":"tool","name":"{tool}"}}"#));
        if !ty.is_empty() {
            req(&mut e, &format!(r#"{{"cmd":"setting","key":"shapeType","value":"{ty}"}}"#));
        }
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"shape","value":"{mode}"}}"#));
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"brush","value":{w}}}"#));
        req(&mut e, r#"{"cmd":"colors","fg":"203080","bg":"F0C040"}"#);
        let x = 5 + k as i32 * 102;
        ptr(&mut e, "down", x, 20, 0);
        for i in 1..=10 { ptr(&mut e, "move", x + i * 9, 20 + i * 9, 0); }
        ptr(&mut e, "up", x + 95, 115, 0);
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
    }
    let png = openpdn_formats::png::encode(&e.document().layers[0].surface, &[], openpdn_formats::png::Compression::Fast);
    std::fs::write("/tmp/thick.png", png).unwrap();
}

const FILL_STYLES: [&str; 38] = ["Solid", "Horizontal", "Vertical", "ForwardDiagonal", "BackwardDiagonal", "LightHorizontal", "LightVertical",
    "DarkHorizontal", "DarkVertical", "DashedHorizontal", "Waves", "ZigZag", "Cross", "SmallGrid", "LargeGrid", "DiagonalCross",
    "DenseCrosshatch", "Diamonds", "Bricks", "Plaid", "Dots", "SparseDots", "LargeDots", "Checkerboard", "SmallCheckerboard",
    "LargeCheckerboard", "Dither6", "Dither12", "Dither25", "Dither37", "Percent50", "Dither62", "Dither75", "Dither87", "Dither94",
    "Noise25", "Noise50", "Noise75"];

/// Every fill style gives the same pixels whichever tool fills: Shape,
/// Freeform Shape and Paint Bucket, on the same area.
#[test]
fn fill_styles_are_consistent_across_tools() {
    let black = |e: &Editor| -> Vec<bool> { e.document().layers[0].surface.pixels().iter().map(|c| c.r == 0).collect() };
    let setup = |style: &str, tool: &str| {
        let mut e = Editor::new(64, 48);
        req(&mut e, &format!(r#"{{"cmd":"tool","name":"{tool}"}}"#));
        req(&mut e, r#"{"cmd":"setting","key":"shape","value":"Fill"}"#);
        req(&mut e, r#"{"cmd":"setting","key":"aa","value":false}"#);
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"fillStyle","value":"{style}"}}"#));
        req(&mut e, r#"{"cmd":"colors","fg":"000000","bg":"000000"}"#);
        e
    };
    for style in FILL_STYLES {
        // Shape tool: a 64×48 rectangle covering the image.
        let mut shape = setup(style, "Shape");
        drag(&mut shape, (0, 0), (63, 47));
        req(&mut shape, r#"{"cmd":"shape_commit"}"#);
        // Paint bucket on the white image.
        let mut bucket = setup(style, "Fill");
        drag(&mut bucket, (10, 10), (10, 10));
        // Freeform shape around the whole image.
        let mut free = setup(style, "Freeform Shape");
        ptr(&mut free, "down", 0, 0, 0);
        for (x, y) in [(63, 0), (63, 47), (0, 47)] {
            ptr(&mut free, "move", x, y, 0);
        }
        ptr(&mut free, "up", 0, 47, 0);
        let (a, b, c) = (black(&shape), black(&bucket), black(&free));
        let n = a.iter().filter(|&&v| v).count();
        assert!(n > 0, "{style}: nothing filled");
        // Compare away from the outer edge (polygon rasterisers differ on boundary pixels).
        let diff = |p: &[bool], q: &[bool]| (2..46).flat_map(|y| (2..62).map(move |x| y * 64 + x)).filter(|&i| p[i] != q[i]).count();
        assert_eq!(diff(&a, &b), 0, "{style}: shape vs bucket");
        assert_eq!(diff(&a, &c), 0, "{style}: shape vs freeform");
        if style != "Solid" {
            assert!(n < 64 * 48, "{style}: pattern should leave gaps");
        }
    }
}

#[test]
fn dither_coverage_matches_its_name() {
    for (style, pct) in [("Dither6", 6.25), ("Dither12", 12.5), ("Dither25", 25.0), ("Dither37", 37.5), ("Percent50", 50.0), ("Dither62", 62.5),
        ("Dither75", 75.0), ("Dither87", 87.5), ("Dither94", 93.75), ("Noise25", 25.0), ("Noise50", 50.0), ("Noise75", 75.0)] {
        let mut e = Editor::new(64, 64);
        let reply = e.handle(&Json::parse(&format!(r#"{{"cmd":"pattern_preview","style":"{style}","w":64,"h":64}}"#)).unwrap(), &[]);
        let mask = &reply.bins[0].1;
        let on = mask.iter().filter(|&&v| v == 255).count() as f64 * 100.0 / mask.len() as f64;
        assert!((on - pct).abs() < 3.0, "{style}: {on:.1}% on, expected {pct}");
    }
}


