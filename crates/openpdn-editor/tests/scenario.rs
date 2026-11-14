use openpdn_editor::Editor;
use openpdn_formats::json::Json;

fn call(e: &mut Editor, json: &str, bin: &[u8]) -> (Json, Vec<u8>) {
    let req = Json::parse(json).unwrap();
    let r = e.handle(&req, bin);
    let j = r.to_json();
    assert_eq!(j.get("ok"), Some(&Json::Bool(true)), "{json} -> {:?}", j.get("error"));
    (j, r.binary())
}

#[test]
fn edit_session() {
    let mut e = Editor::new(64, 48);
    let (j, bin) = call(&mut e, r#"{"cmd":"init","id":1}"#, &[]);
    assert!(j.get("fx").unwrap().as_array().unwrap().len() > 40);
    assert_eq!(bin.len(), 64 * 48 * 4, "initial full tile");

    call(&mut e, r#"{"cmd":"tool","name":"Brush"}"#, &[]);
    call(&mut e, r##"{"cmd":"colors","fg":"#FF0000"}"##, &[]);
    call(&mut e, r#"{"cmd":"pointer","phase":"down","x":5,"y":5,"button":0}"#, &[]);
    let (j, bin) = call(&mut e, r#"{"cmd":"pointer","phase":"move","x":40,"y":30}"#, &[]);
    let t = j.get("tile").unwrap();
    let (w, h) = (t.get("w").unwrap().as_i64().unwrap(), t.get("h").unwrap().as_i64().unwrap());
    assert_eq!(bin.len() as i64, w * h * 4, "move sends just the dirty region");
    assert!(w < 64 || h < 48);
    let (j, _) = call(&mut e, r#"{"cmd":"pointer","phase":"up","x":40,"y":30}"#, &[]);
    let hist = j.get("state").unwrap().get("history").unwrap();
    assert_eq!(hist.get("applied").unwrap().as_i64(), Some(1));
    assert_eq!(e.document().layers[0].surface.get(20, 16).r, 255);

    // Effect preview then commit, then undo both edits.
    call(&mut e, r#"{"cmd":"select_all"}"#, &[]);
    call(&mut e, r#"{"cmd":"fx","phase":"begin","fx":"gaussian-blur","values":[1]}"#, &[]);
    call(&mut e, r#"{"cmd":"fx","phase":"preview","values":[4]}"#, &[]);
    let (j, _) = call(&mut e, r#"{"cmd":"fx","phase":"commit"}"#, &[]);
    let items = j.get("state").unwrap().get("history").unwrap().get("items").unwrap().as_array().unwrap().to_vec();
    assert_eq!(items.last().unwrap().as_str(), Some("Gaussian Blur"));
    call(&mut e, r#"{"cmd":"undo"}"#, &[]);
    call(&mut e, r#"{"cmd":"undo"}"#, &[]);
    call(&mut e, r#"{"cmd":"undo"}"#, &[]);
    assert_eq!(e.document().layers[0].surface.get(20, 16), openpdn_core::Bgra::WHITE);

    // Cancelled preview restores pixels.
    call(&mut e, r#"{"cmd":"fx","phase":"begin","fx":"invert-colors"}"#, &[]); // direct: applies at once
    assert_eq!(e.document().layers[0].surface.get(0, 0), openpdn_core::Bgra::BLACK);
    call(&mut e, r#"{"cmd":"fx","phase":"begin","fx":"emboss","values":[10]}"#, &[]);
    call(&mut e, r#"{"cmd":"fx","phase":"cancel"}"#, &[]);
    assert_eq!(e.document().layers[0].surface.get(0, 0), openpdn_core::Bgra::BLACK);

    // Copy / paste into a new layer, then save and reopen as .openpdn.
    call(&mut e, r#"{"cmd":"select_all"}"#, &[]);
    let (j, rgba) = call(&mut e, r#"{"cmd":"copy"}"#, &[]);
    assert_eq!(j.get("w").unwrap().as_i64(), Some(64));
    call(&mut e, r#"{"cmd":"paste","mode":"new_layer","w":64,"h":48}"#, &rgba[..64 * 48 * 4]);
    assert_eq!(e.document().layers.len(), 2);
    let (_, file) = call(&mut e, r#"{"cmd":"save","format":"openpdn"}"#, &[]);
    let mut e2 = Editor::new(1, 1);
    call(&mut e2, r#"{"cmd":"open","name":"x.openpdn"}"#, &file[..file.len() - 64 * 48 * 4.min(0)]);
    assert_eq!(e2.document().layers.len(), 2);
}
