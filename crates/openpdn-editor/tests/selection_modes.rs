use openpdn_editor::Editor;
use openpdn_formats::json::Json;

fn req(e: &mut Editor, j: &str) -> Json {
    let r = e.handle(&Json::parse(j).unwrap(), &[]).to_json();
    assert_eq!(r.get("ok"), Some(&Json::Bool(true)), "{j} -> {r:?}");
    r
}
fn drag(e: &mut Editor, (x0, y0): (i32, i32), (x1, y1): (i32, i32), button: i32, mods: &str) {
    for (phase, x, y) in [("down", x0, y0), ("move", x1, y1), ("up", x1, y1)] {
        req(e, &format!(r#"{{"cmd":"pointer","phase":"{phase}","x":{x},"y":{y},"button":{button}{mods}}}"#));
    }
}
fn sel(e: &Editor) -> Vec<(i32, i32, u8)> {
    let s = e.document().selection().expect("selection");
    let mut v = vec![];
    for y in 0..e.document().height() {
        for x in 0..e.document().width() {
            let c = s.coverage_at(x, y);
            if c > 0 {
                v.push((x, y, c));
            }
        }
    }
    v
}
fn bounds(e: &Editor) -> openpdn_core::PixelRect {
    e.document().selection().unwrap().bounds()
}

#[test]
fn xor_via_ctrl_right_drag_and_toolbar() {
    let mut e = Editor::new(40, 30);
    req(&mut e, r#"{"cmd":"tool","name":"Rect Select"}"#);
    drag(&mut e, (0, 0), (9, 9), 0, "");
    drag(&mut e, (5, 5), (14, 14), 2, r#","ctrl":true"#);
    let s = sel(&e);
    let inside = |x, y| s.iter().any(|&(a, b, _)| a == x && b == y);
    assert!(inside(0, 0) && inside(14, 14) && !inside(7, 7), "xor keeps only the non-overlapping parts");
    assert_eq!(s.len(), 100 + 100 - 2 * 25);
    // Alt + right = intersect.
    drag(&mut e, (0, 0), (9, 9), 0, "");
    drag(&mut e, (5, 5), (14, 14), 2, r#","alt":true"#);
    assert_eq!(sel(&e).len(), 25);
    // Toolbar mode Xor with a plain drag.
    req(&mut e, r#"{"cmd":"setting","key":"combine","value":"Xor"}"#);
    drag(&mut e, (5, 5), (9, 9), 0, "");
    assert!(e.document().active_selection().is_none(), "xor of identical regions is empty");
}

#[test]
fn fixed_ratio_and_fixed_size() {
    let mut e = Editor::new(200, 100);
    req(&mut e, r#"{"cmd":"tool","name":"Rect Select"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"selectSize","value":{"mode":"FixedRatio","w":2,"h":1}}"#);
    drag(&mut e, (10, 10), (109, 89), 0, "");
    let b = bounds(&e);
    assert_eq!((b.x, b.y, b.width, b.height), (10, 10, 100, 50));
    // Dragging up-left anchors at the start point.
    drag(&mut e, (150, 80), (131, 41), 0, "");
    let b = bounds(&e);
    assert_eq!((b.right(), b.bottom(), b.width, b.height), (151, 81, 20, 10));
    // Fixed size: a single click makes the selection.
    req(&mut e, r#"{"cmd":"setting","key":"selectSize","value":{"mode":"FixedSize","w":30,"h":20}}"#);
    drag(&mut e, (5, 6), (5, 6), 0, "");
    let b = bounds(&e);
    assert_eq!((b.x, b.y, b.width, b.height), (5, 6, 30, 20));
    let r = req(&mut e, r#"{"cmd":"init"}"#);
    let st = r.get("state").unwrap().get("settings").unwrap().get("selectSize").unwrap();
    assert_eq!(st.get("mode").and_then(Json::as_str), Some("FixedSize"));
}

#[test]
fn global_magic_wand_and_fill() {
    let mut e = Editor::new(30, 10);
    // Two separate black squares on white.
    req(&mut e, r#"{"cmd":"colors","fg":"000000"}"#);
    req(&mut e, r#"{"cmd":"tool","name":"Rectangle"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shape","value":"Fill"}"#);
    req(&mut e, r#"{"cmd":"colors","bg":"000000"}"#);
    drag(&mut e, (1, 1), (4, 4), 0, "");
    drag(&mut e, (20, 2), (23, 5), 0, "");
    req(&mut e, r#"{"cmd":"tool","name":"Magic Wand"}"#);
    drag(&mut e, (2, 2), (2, 2), 0, "");
    assert_eq!(sel(&e).len(), 16, "contiguous picks one square");
    req(&mut e, r#"{"cmd":"setting","key":"flood","value":"Global"}"#);
    drag(&mut e, (2, 2), (2, 2), 0, "");
    assert_eq!(sel(&e).len(), 32, "global picks both squares");
    // Global paint bucket recolours both squares.
    req(&mut e, r#"{"cmd":"deselect"}"#);
    req(&mut e, r#"{"cmd":"colors","fg":"FF0000"}"#);
    req(&mut e, r#"{"cmd":"tool","name":"Fill"}"#);
    drag(&mut e, (2, 2), (2, 2), 0, "");
    let l = &e.document().layers[0].surface;
    assert_eq!((l.get(2, 2).r, l.get(21, 3).r, l.get(21, 3).g), (255, 255, 0));
}

#[test]
fn pixelated_selection_quality_hardens_edges() {
    let mut e = Editor::new(40, 40);
    req(&mut e, r#"{"cmd":"tool","name":"Ellipse Select"}"#);
    drag(&mut e, (3, 3), (33, 30), 0, "");
    let soft = sel(&e).iter().filter(|p| p.2 != 255).count();
    assert!(soft > 0, "antialiased ellipse has soft edges");
    req(&mut e, r#"{"cmd":"setting","key":"selectionAA","value":false}"#);
    assert!(sel(&e).iter().all(|p| p.2 == 255), "pixelated clipping is hard-edged");
    // Filling the selection respects it: no partially blended pixels.
    req(&mut e, r#"{"cmd":"fill_selection"}"#);
    let l = &e.document().layers[0].surface;
    assert!(l.pixels().iter().all(|p| p.r == 0 || p.r == 255));
    req(&mut e, r#"{"cmd":"setting","key":"selectionAA","value":true}"#);
    assert_eq!(sel(&e).iter().filter(|p| p.2 != 255).count(), soft, "switching back restores soft edges");
}

#[test]
fn tolerance_alpha_modes() {
    // New transparent layer (transparent black); paint its right half transparent red.
    let mut e = Editor::new(20, 10);
    req(&mut e, r#"{"cmd":"layer_add"}"#);
    req(&mut e, r#"{"cmd":"tool","name":"Rectangle"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"shape","value":"Fill"}"#);
    req(&mut e, r#"{"cmd":"setting","key":"alphaBlend","value":false}"#);
    req(&mut e, r#"{"cmd":"colors","fg":"FF000000","bg":"FF000000"}"#);
    drag(&mut e, (10, 0), (19, 9), 0, "");
    req(&mut e, r#"{"cmd":"setting","key":"tolerance","value":0}"#);
    req(&mut e, r#"{"cmd":"tool","name":"Magic Wand"}"#);
    drag(&mut e, (2, 2), (2, 2), 0, "");
    assert_eq!(sel(&e).len(), 200, "premultiplied: all fully transparent pixels match");
    req(&mut e, r#"{"cmd":"setting","key":"toleranceAlpha","value":"Straight"}"#);
    drag(&mut e, (2, 2), (2, 2), 0, "");
    assert_eq!(sel(&e).len(), 100, "straight: transparent red differs from transparent black");
    // Sampling the image instead of the layer sees the white background.
    req(&mut e, r#"{"cmd":"setting","key":"wandSampleImage","value":true}"#);
    drag(&mut e, (2, 2), (2, 2), 0, "");
    assert_eq!(sel(&e).len(), 200, "image sampling: the composite is white everywhere");
}
