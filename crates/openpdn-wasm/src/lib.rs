//! Browser entry point: exposes [`openpdn_editor::Editor`] to JavaScript.
//! The web UI's worker (`web/worker.js`) calls `handle` with a JSON request
//! and binary payload, and gets the JSON reply and concatenated payloads.

use openpdn_editor::Session;
use openpdn_formats::json::Json;
use wasm_bindgen::prelude::*;

#[cfg(feature = "threads")]
pub use wasm_bindgen_rayon::init_thread_pool;

#[wasm_bindgen]
pub struct WasmEditor {
    editor: Session,
}

#[wasm_bindgen]
pub struct HandleResult {
    json: String,
    bin: Vec<u8>,
}

#[wasm_bindgen]
impl HandleResult {
    #[wasm_bindgen(getter)]
    pub fn json(&self) -> String {
        self.json.clone()
    }

    /// Copied into a fresh `Uint8Array` on the JS side.
    #[wasm_bindgen(getter)]
    pub fn bin(&self) -> Vec<u8> {
        self.bin.clone()
    }
}

#[wasm_bindgen]
impl WasmEditor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmEditor {
        WasmEditor { editor: Session::default() }
    }

    pub fn handle(&mut self, request: &str, payload: &[u8]) -> HandleResult {
        let reply = match Json::parse(request) {
            Ok(req) => self.editor.handle(&req, payload),
            Err(e) => {
                let mut r = openpdn_editor::Reply::default();
                r.json.push(("ok".into(), Json::Bool(false)));
                r.json.push(("error".into(), Json::Str(format!("bad request: {e}"))));
                r
            }
        };
        HandleResult { json: reply.to_json().to_string_pretty(), bin: reply.binary() }
    }
}

impl Default for WasmEditor {
    fn default() -> Self {
        Self::new()
    }
}
