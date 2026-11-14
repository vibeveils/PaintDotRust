//! Several open documents (tabs), each with its own [`Editor`] (document,
//! history, selection, preview). Tool choice and tool settings (colours,
//! brush size, ...) belong to the application, as in Paint.NET, so they move
//! with the user when the active document changes.
//!
//! Commands handled here:
//! - `doc_add` `{then?: <cmd>, ...}`: adds a document and makes it active. With
//!   `then`, the rest of the request is run on the new document as that
//!   command (e.g. `then: "new"` or `"open"`); if it fails, the new document is
//!   dropped again and the previous one stays active.
//! - `doc_switch` `{doc}`, `doc_close` `{doc}` (closing the last document
//!   leaves a fresh blank one), `docs`.
//!
//! Everything else goes to the active document. Every reply carries `doc`
//! (the active id); `doc_*` replies also carry `docs: [{id, name, dirty}]`.

use crate::{Editor, Reply};
use openpdn_formats::json::Json;

pub struct Session {
    docs: Vec<(u32, Editor)>,
    active: usize,
    next_id: u32,
}

impl Default for Session {
    fn default() -> Self {
        Session { docs: vec![(1, Editor::default())], active: 0, next_id: 2 }
    }
}

fn err(req: &Json, msg: &str) -> Reply {
    let mut r = Reply::default();
    if let Some(id) = req.get("id") {
        r.set("id", id.clone());
    }
    r.set("ok", Json::Bool(false));
    r.set("error", Json::Str(msg.into()));
    r
}

impl Session {
    pub fn active(&self) -> &Editor {
        &self.docs[self.active].1
    }

    pub fn active_id(&self) -> u32 {
        self.docs[self.active].0
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.docs.iter().position(|d| d.0 == id)
    }

    /// Makes `to` active, carrying the tool and its settings over from `from`.
    fn activate(&mut self, from: Option<usize>, to: usize) {
        if let Some(from) = from.filter(|&f| f != to && f < self.docs.len()) {
            let settings = self.docs[from].1.tools.settings.clone();
            let kind = self.docs[from].1.tools.active_kind();
            let ed = &mut self.docs[to].1;
            if ed.tools.active_kind() != kind {
                ed.commit_pending(); // its pending line/text belongs to the old tool
            }
            ed.tools.settings = settings;
            ed.tools.set_active(kind);
        }
        self.active = to;
        let ed = &mut self.docs[to].1;
        ed.force_state = true;
        ed.force_full = true;
        ed.sent_settings = u64::MAX;
    }

    fn docs_json(&self) -> Json {
        Json::Arr(
            self.docs
                .iter()
                .map(|(id, e)| {
                    Json::Obj(vec![
                        ("id".into(), Json::Num(*id as f64)),
                        ("name".into(), Json::Str(e.name.clone())),
                        ("dirty".into(), Json::Bool(e.dirty || e.has_pending())),
                    ])
                })
                .collect(),
        )
    }

    /// Renders the active document as the reply to a `doc_*` command.
    fn render_active(&mut self, req: &Json) -> Reply {
        let mut r = req.clone();
        set_cmd(&mut r, "render");
        let mut reply = self.docs[self.active].1.handle(&r, &[]);
        reply.set("docs", self.docs_json());
        reply
    }

    pub fn handle(&mut self, req: &Json, bin: &[u8]) -> Reply {
        let cmd = req.get("cmd").and_then(Json::as_str).unwrap_or("");
        let doc_arg = req.get("doc").and_then(Json::as_f64).map(|v| v as u32);
        let mut reply = match cmd {
            "doc_add" => {
                let prev = self.active;
                let id = self.next_id;
                self.next_id += 1;
                self.docs.push((id, Editor::default()));
                let idx = self.docs.len() - 1;
                self.activate(Some(prev), idx);
                match req.get("then").and_then(Json::as_str) {
                    Some(then) => {
                        let mut r = req.clone();
                        set_cmd(&mut r, then);
                        let mut reply = self.docs[idx].1.handle(&r, bin);
                        if reply.json.iter().any(|(k, v)| k == "ok" && *v == Json::Bool(false)) {
                            // Undo the add; keep the error.
                            self.docs.pop();
                            self.activate(None, prev.min(self.docs.len() - 1));
                            let mut refreshed = self.render_active(req);
                            refreshed.json.retain(|(k, _)| k != "ok");
                            for (k, v) in reply.json.drain(..).filter(|(k, _)| k == "ok" || k == "error") {
                                refreshed.json.push((k, v));
                            }
                            refreshed
                        } else {
                            reply.set("docs", self.docs_json());
                            reply
                        }
                    }
                    None => self.render_active(req),
                }
            }
            "doc_switch" => match doc_arg.and_then(|d| self.index_of(d)) {
                Some(i) => {
                    let prev = self.active;
                    self.activate(Some(prev), i);
                    self.render_active(req)
                }
                None => err(req, "no such document"),
            },
            "doc_close" => match doc_arg.and_then(|d| self.index_of(d)) {
                Some(i) => {
                    let was_active = i == self.active;
                    let (_, closed) = self.docs.remove(i);
                    if self.docs.is_empty() {
                        let id = self.next_id;
                        self.next_id += 1;
                        let mut fresh = Editor::default();
                        fresh.tools.settings = closed.tools.settings.clone();
                        fresh.tools.set_active(closed.tools.active_kind());
                        self.docs.push((id, fresh));
                        self.activate(None, 0);
                    } else if was_active {
                        let to = i.min(self.docs.len() - 1);
                        self.docs[to].1.tools.settings = closed.tools.settings.clone();
                        self.docs[to].1.tools.set_active(closed.tools.active_kind());
                        self.activate(None, to);
                    } else if i < self.active {
                        self.active -= 1;
                    }
                    self.render_active(req)
                }
                None => err(req, "no such document"),
            },
            "docs" => {
                let mut r = Reply::default();
                if let Some(id) = req.get("id") {
                    r.set("id", id.clone());
                }
                r.set("ok", Json::Bool(true));
                r.set("docs", self.docs_json());
                r
            }
            _ => match doc_arg {
                // A request aimed at a document that is no longer active is refused
                // rather than applied to the wrong one.
                Some(d) if d != self.active_id() => err(req, "document is not active"),
                _ => self.docs[self.active].1.handle(req, bin),
            },
        };
        reply.set("doc", Json::Num(self.active_id() as f64));
        reply
    }
}

fn set_cmd(req: &mut Json, cmd: &str) {
    if let Json::Obj(o) = req {
        o.retain(|(k, _)| k != "cmd" && k != "then");
        o.push(("cmd".into(), Json::Str(cmd.into())));
    }
}
