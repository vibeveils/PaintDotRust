use openpdn_editor::{Reply, Session};
use openpdn_formats::json::Json;

fn call(s: &mut Session, req: &str, bin: &[u8]) -> Json {
    let r: Reply = s.handle(&Json::parse(req).unwrap(), bin);
    r.to_json()
}
fn get<'a>(j: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(j, |j, k| j.get(k).unwrap_or_else(|| panic!("missing {k} in {j:?}")))
}
fn docs(j: &Json) -> usize {
    match get(j, &["docs"]) { Json::Arr(a) => a.len(), _ => panic!() }
}

#[test]
fn tabs_add_switch_close() {
    let mut s = Session::default();
    call(&mut s, r#"{"cmd":"init","id":1}"#, &[]);
    let r = call(&mut s, r#"{"cmd":"doc_add","then":"new","w":100,"h":50,"id":2}"#, &[]);
    assert_eq!(get(&r, &["ok"]), &Json::Bool(true));
    assert_eq!(get(&r, &["state", "width"]).as_f64(), Some(100.0));
    assert_eq!(docs(&r), 2);
    let second = get(&r, &["doc"]).as_f64().unwrap();
    // Colours set on the second document carry over to the first.
    call(&mut s, r#"{"cmd":"colors","fg":"FF112233"}"#, &[]);
    let r = call(&mut s, r#"{"cmd":"doc_switch","doc":1}"#, &[]);
    assert_eq!(get(&r, &["state", "width"]).as_f64(), Some(800.0));
    let fg = get(&r, &["state", "settings", "fg"]).as_str().unwrap().to_ascii_lowercase();
    assert!(fg.contains("112233"), "fg {fg}");
    // A request tagged for an inactive document is refused.
    let r = call(&mut s, &format!(r#"{{"cmd":"flip_h","doc":{second}}}"#), &[]);
    assert_eq!(get(&r, &["ok"]), &Json::Bool(false));
    // A failed open in a new tab leaves the tabs as they were.
    let r = call(&mut s, r#"{"cmd":"doc_add","then":"open","name":"x.jpg"}"#, b"not an image");
    assert_eq!(get(&r, &["ok"]), &Json::Bool(false));
    assert_eq!(docs(&r), 2);
    assert_eq!(get(&r, &["doc"]).as_f64(), Some(1.0));
    // Closing the active tab activates a neighbour; closing the last leaves a blank one.
    let r = call(&mut s, r#"{"cmd":"doc_close","doc":1}"#, &[]);
    assert_eq!(docs(&r), 1);
    assert_eq!(get(&r, &["doc"]).as_f64(), Some(second));
    assert_eq!(get(&r, &["state", "width"]).as_f64(), Some(100.0));
    let r = call(&mut s, &format!(r#"{{"cmd":"doc_close","doc":{second}}}"#), &[]);
    assert_eq!(docs(&r), 1);
    assert_eq!(get(&r, &["state", "width"]).as_f64(), Some(800.0));
}
