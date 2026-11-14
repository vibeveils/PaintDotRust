//! Editable gradients, the Distort brush and the Perspective tool.

use openpdn_editor::Editor;
use openpdn_formats::json::Json;

fn req(e: &mut Editor, j: &str) -> Json {
    let r = e.handle(&Json::parse(j).unwrap(), &[]).to_json();
    assert_eq!(r.get("ok"), Some(&Json::Bool(true)), "{j} -> {r:?}");
    r
}
fn ptr(e: &mut Editor, phase: &str, x: i32, y: i32, button: i32) {
    req(e, &format!(r#"{{"cmd":"pointer","phase":"{phase}","x":{x},"y":{y},"button":{button},"handleRadius":3}}"#));
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
fn state(e: &mut Editor) -> Json {
    req(e, r#"{"cmd":"init"}"#).get("state").unwrap().clone()
}
fn history(e: &mut Editor) -> Vec<String> {
    match state(e).get("history").unwrap().get("items").unwrap() {
        Json::Arr(a) => a.iter().map(|v| v.as_str().unwrap().to_owned()).collect(),
        _ => panic!(),
    }
}
fn pending(e: &mut Editor) -> Json {
    state(e).get("pending").cloned().unwrap_or(Json::Null)
}
fn arr(j: &Json) -> &[Json] {
    match j {
        Json::Arr(a) => a,
        _ => panic!("not an array: {j:?}"),
    }
}
fn near(a: (u8, u8, u8, u8), b: (u8, u8, u8, u8), tol: i32) -> bool {
    let d = |x: u8, y: u8| (x as i32 - y as i32).abs() <= tol;
    d(a.0, b.0) && d(a.1, b.1) && d(a.2, b.2) && d(a.3, b.3)
}

fn gradient_editor() -> Editor {
    let mut e = Editor::new(100, 60);
    req(&mut e, r#"{"cmd":"tool","name":"Gradient"}"#);
    req(&mut e, r##"{"cmd":"colors","fg":"#000000","bg":"#FFFFFF"}"##);
    req(&mut e, r#"{"cmd":"setting","key":"gradientDither","value":false}"#);
    e
}

#[test]
fn gradient_stays_editable_with_colour_stops() {
    let mut e = gradient_editor();
    drag(&mut e, (10, 30), (90, 30));
    assert!(e.has_pending());
    assert!(history(&mut e).is_empty(), "no history until committed");
    let p = pending(&mut e);
    assert_eq!(p.get("kind").and_then(Json::as_str), Some("gradient"));
    assert_eq!(arr(p.get("stops").unwrap()).len(), 2);
    assert!(near(px(&e, 5, 30), (0, 0, 0, 255), 0));
    assert!(near(px(&e, 50, 30), (128, 128, 128, 255), 2));
    assert!(near(px(&e, 95, 30), (255, 255, 255, 255), 0));

    // Drag the end point in: the gradient is redrawn, not stacked.
    drag(&mut e, (90, 30), (50, 30));
    assert!(near(px(&e, 30, 30), (128, 128, 128, 255), 2));
    assert!(near(px(&e, 60, 30), (255, 255, 255, 255), 0));

    // Changing the type re-renders live.
    req(&mut e, r#"{"cmd":"setting","key":"gradient","value":"Radial"}"#);
    assert!(near(px(&e, 10, 30), (0, 0, 0, 255), 0), "radial: centre at the start");

    req(&mut e, r#"{"cmd":"setting","key":"gradient","value":"Linear"}"#);
    // Click on the line: a new stop (same colour, so nothing changes yet), selected.
    let before = px(&e, 30, 30);
    ptr(&mut e, "down", 30, 30, 0);
    ptr(&mut e, "up", 30, 30, 0);
    let p = pending(&mut e);
    assert_eq!(arr(p.get("stops").unwrap()).len(), 3);
    assert_eq!(p.get("selected").and_then(Json::as_f64), Some(1.0));
    assert_eq!(px(&e, 30, 30), before);
    // Picking a colour recolours the selected stop only.
    req(&mut e, r##"{"cmd":"colors","fg":"#FF0000"}"##);
    assert!(near(px(&e, 30, 30), (255, 0, 0, 255), 0), "{:?}", px(&e, 30, 30));
    assert!(near(px(&e, 5, 30), (0, 0, 0, 255), 0), "start stays black");
    // Reverse swaps the order.
    req(&mut e, r#"{"cmd":"gradient_reverse"}"#);
    assert!(near(px(&e, 5, 30), (255, 255, 255, 255), 0));
    // Undo steps back through the edits: un-reverse, then the colour.
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert!(near(px(&e, 5, 30), (0, 0, 0, 255), 0));
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert_eq!(px(&e, 30, 30), before);
    req(&mut e, r#"{"cmd":"redo"}"#);
    assert!(near(px(&e, 30, 30), (255, 0, 0, 255), 0));
    // Delete removes the selected stop.
    req(&mut e, r#"{"cmd":"pending_delete"}"#);
    assert_eq!(arr(pending(&mut e).get("stops").unwrap()).len(), 2);

    // With nothing selected, the ends follow the colours.
    req(&mut e, r##"{"cmd":"colors","fg":"#0000FF"}"##);
    assert!(near(px(&e, 5, 30), (0, 0, 255, 255), 0));

    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert!(!e.has_pending());
    assert_eq!(history(&mut e), vec!["Gradient"]);
}

#[test]
fn gradient_dither_breaks_up_bands_without_shifting_colour() {
    let run = |dither: bool| {
        let mut e = Editor::new(256, 32);
        req(&mut e, r#"{"cmd":"tool","name":"Gradient"}"#);
        req(&mut e, r##"{"cmd":"colors","fg":"#000000","bg":"#101010"}"##);
        req(&mut e, &format!(r#"{{"cmd":"setting","key":"gradientDither","value":{dither}}}"#));
        drag(&mut e, (0, 0), (255, 0));
        req(&mut e, r#"{"cmd":"shape_commit"}"#);
        e
    };
    let (plain, dith) = (run(false), run(true));
    // Plain: every column is one flat value (bands 16 px wide).
    let column_varies = |e: &Editor, x: i32| (0..32).any(|y| px(e, x, y).0 != px(e, x, 0).0);
    assert!(!(0..256).any(|x| column_varies(&plain, x)));
    assert!((0..256).filter(|&x| column_varies(&dith, x)).count() > 100, "dithered columns mix neighbouring levels");
    // The average over each 8×8 block stays the same (within rounding).
    for bx in (0..256).step_by(8) {
        let avg = |e: &Editor| (0..8).flat_map(|y| (bx..bx + 8).map(move |x| (x, y))).map(|(x, y)| px(e, x, y).0 as f32).sum::<f32>() / 64.0;
        assert!((avg(&plain) - avg(&dith)).abs() <= 1.0, "block {bx}: {} vs {}", avg(&plain), avg(&dith));
    }
}

#[test]
fn gradient_repeat_modes() {
    let mut e = gradient_editor();
    req(&mut e, r#"{"cmd":"setting","key":"gradientRepeat","value":"Repeat"}"#);
    drag(&mut e, (0, 30), (40, 30));
    assert!(px(&e, 60, 30).0 < 140 && px(&e, 60, 30).0 > 115, "half way through the second cycle: {:?}", px(&e, 60, 30));
    req(&mut e, r#"{"cmd":"setting","key":"gradientRepeat","value":"Reflect"}"#);
    assert!(px(&e, 70, 30).0 < 70, "coming back down: {:?}", px(&e, 70, 30));
}

fn square_editor() -> Editor {
    // White 100×60 with a black 20×20 square centred at (50, 30).
    let mut e = Editor::new(100, 60);
    req(&mut e, r#"{"cmd":"tool","name":"Rect Select"}"#);
    ptr(&mut e, "down", 40, 20, 0);
    ptr(&mut e, "move", 60, 40, 0);
    ptr(&mut e, "up", 60, 40, 0);
    req(&mut e, r##"{"cmd":"colors","fg":"#000000","bg":"#FFFFFF"}"##);
    req(&mut e, r#"{"cmd":"fill_selection"}"#);
    req(&mut e, r#"{"cmd":"deselect"}"#);
    e
}
fn dark_count(e: &Editor) -> usize {
    e.document().layers[0].surface.pixels().iter().filter(|c| c.r < 128).count()
}

#[test]
fn distort_pinch_and_bulge() {
    let mut e = square_editor();
    let base = dark_count(&e);
    assert_eq!(base, 441);
    req(&mut e, r#"{"cmd":"tool","name":"Distort"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":50}"#);
    req(&mut e, r#"{"cmd":"setting","key":"warpStrength","value":100}"#);
    req(&mut e, r#"{"cmd":"setting","key":"warpMode","value":"Pinch"}"#);
    ptr(&mut e, "down", 50, 30, 0);
    ptr(&mut e, "move", 50, 30, 0); // held still: keeps pinching
    ptr(&mut e, "up", 50, 30, 0);
    let pinched = dark_count(&e);
    assert!(pinched < base - 40, "pinch shrinks the square: {pinched}");
    assert_eq!(history(&mut e).last().map(String::as_str), Some("Distort"));
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert_eq!(dark_count(&e), base);
    req(&mut e, r#"{"cmd":"setting","key":"warpMode","value":"Bulge"}"#);
    ptr(&mut e, "down", 50, 30, 0);
    ptr(&mut e, "up", 50, 30, 0);
    assert!(dark_count(&e) > base + 40, "bulge grows it: {}", dark_count(&e));
    // Right button: the opposite (pinch).
    req(&mut e, r#"{"cmd":"undo"}"#);
    ptr(&mut e, "down", 50, 30, 2);
    ptr(&mut e, "up", 50, 30, 2);
    assert!(dark_count(&e) < base);
    // Far from everything: nothing changes outside the brush.
    assert_eq!(px(&e, 2, 2), (255, 255, 255, 255));
}

#[test]
fn distort_push_and_twirl() {
    let mut e = square_editor();
    req(&mut e, r#"{"cmd":"tool","name":"Distort"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"brush","value":30}"#);
    req(&mut e, r#"{"cmd":"setting","key":"warpStrength","value":100}"#);
    req(&mut e, r#"{"cmd":"setting","key":"warpMode","value":"Push"}"#);
    assert_eq!(px(&e, 64, 30).0, 255);
    ptr(&mut e, "down", 55, 30, 0);
    for x in 56..=68 {
        ptr(&mut e, "move", x, 30, 0);
    }
    ptr(&mut e, "up", 68, 30, 0);
    assert!(px(&e, 64, 30).0 < 128, "pixels dragged right: {:?}", px(&e, 64, 30));
    req(&mut e, r#"{"cmd":"undo"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"warpMode","value":"TwirlClockwise"}"#);
    let before = dark_count(&e);
    let before_px = e.document().layers[0].surface.pixels().to_vec();
    ptr(&mut e, "down", 50, 30, 0);
    ptr(&mut e, "up", 50, 30, 0);
    // Turning keeps roughly the same amount of black, but moves the corners.
    let after = dark_count(&e);
    assert!((after as i32 - before as i32).abs() < 60, "{before} → {after}");
    let changed = e.document().layers[0].surface.pixels().iter().zip(&before_px).filter(|(a, b)| a != b).count();
    assert!(changed > 50, "the square's edges turned: {changed} pixels changed");
}

#[test]
fn perspective_shows_handles_and_is_pending_until_committed() {
    let mut e = square_editor();
    let original = e.document().layers[0].surface.pixels().to_vec();
    req(&mut e, r#"{"cmd":"tool","name":"Perspective"}"#);
    let p = pending(&mut e);
    assert_eq!(p.get("kind").and_then(Json::as_str), Some("quad"), "handles straight away");
    let c = arr(p.get("corners").unwrap());
    assert_eq!(c[2], Json::Arr(vec![Json::Num(100.0), Json::Num(60.0)]));
    // Undo with nothing changed undoes the document history, not the handles.
    assert_eq!(history(&mut e), vec!["Rect Select", "Fill Selection", "Deselect"]);
    // Pull the top-left corner in: the area outside the new shape is empty.
    drag(&mut e, (0, 0), (30, 15));
    assert_eq!(px(&e, 3, 3).3, 0, "outside the quad: transparent");
    assert!(history(&mut e).len() == 3, "still pending");
    // Ctrl+Z steps the corner back.
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert_eq!(e.document().layers[0].surface.pixels(), &original[..]);
    req(&mut e, r#"{"cmd":"redo"}"#);
    assert_eq!(px(&e, 3, 3).3, 0);
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!(history(&mut e).last().map(String::as_str), Some("Perspective"));
    // Handles come back for the next transform.
    assert_eq!(pending(&mut e).get("kind").and_then(Json::as_str), Some("quad"));
    req(&mut e, r#"{"cmd":"undo"}"#);
    assert_eq!(e.document().layers[0].surface.pixels(), &original[..]);
}

#[test]
fn perspective_of_a_selection_skews_pixels_and_outline() {
    let mut e = square_editor();
    req(&mut e, r#"{"cmd":"tool","name":"Rect Select"}"#);
    ptr(&mut e, "down", 40, 20, 0);
    ptr(&mut e, "move", 60, 40, 0);
    ptr(&mut e, "up", 60, 40, 0);
    req(&mut e, r#"{"cmd":"tool","name":"Perspective"}"#);
    // Drag the top edge's middle right: a skew.
    drag(&mut e, (50, 20), (60, 20));
    assert_eq!(px(&e, 41, 21).3, 0, "top left corner moved away (lifted: transparent)");
    assert!(px(&e, 62, 21).0 < 128, "and over to the right");
    assert!(px(&e, 50, 38).0 < 128, "bottom stays put");
    let b = state(&mut e).get("selection").cloned();
    // Cancel restores everything, selection included.
    req(&mut e, r#"{"cmd":"shape_cancel"}"#);
    assert_eq!(dark_count(&e), 441);
    let _ = b;
    // Commit: one step with the selection.
    drag(&mut e, (50, 20), (60, 20));
    req(&mut e, r#"{"cmd":"shape_commit"}"#);
    assert_eq!(history(&mut e).last().map(String::as_str), Some("Perspective"));
    let bounds = e.document().selection().unwrap().bounds();
    assert!(bounds.right() > 62, "selection skewed too: {bounds:?}");
}

