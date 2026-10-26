use std::sync::mpsc::{channel, Receiver, Sender};

use js_sys::wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::{text_read, ClipboardBackend, ClipboardRead};

/// The browser's `navigator.clipboard`, whose reads resolve on a later frame.
pub struct OsClipboard {
    sender: Sender<ClipboardRead>,
    reads: Receiver<ClipboardRead>,
}

impl OsClipboard {
    /// Checks that the page can reach `navigator.clipboard`, or says why it cannot.
    pub fn open() -> Result<Self, String> {
        navigator_clipboard()?;
        let (sender, reads) = channel();
        Ok(Self { sender, reads })
    }
}

impl ClipboardBackend for OsClipboard {
    fn write(&mut self, text: &str) {
        let Ok(clipboard) = navigator_clipboard() else {
            return;
        };
        let written = JsFuture::from(clipboard.write_text(text));
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = written.await {
                log::warn!("Could not copy to the clipboard: {}", describe(&error));
            }
        });
    }

    fn request_read(&mut self) {
        let sender = self.sender.clone();
        let clipboard = match navigator_clipboard() {
            Ok(clipboard) => clipboard,
            Err(reason) => {
                let _ = sender.send(ClipboardRead::Unavailable(reason));
                return;
            }
        };
        let read = JsFuture::from(clipboard.read_text());
        wasm_bindgen_futures::spawn_local(async move {
            let result = read
                .await
                .map(|text| text.as_string().unwrap_or_default())
                .map_err(|error| describe(&error));
            let _ = sender.send(text_read(result));
        });
    }

    fn poll_read(&mut self) -> Option<ClipboardRead> {
        self.reads.try_recv().ok()
    }
}

fn navigator_clipboard() -> Result<web_sys::Clipboard, String> {
    let window = web_sys::window().ok_or("no browser window")?;
    let clipboard = window.navigator().clipboard();
    if clipboard.is_undefined() {
        return Err("navigator.clipboard is missing; the page needs a secure context".into());
    }
    Ok(clipboard)
}

fn describe(error: &JsValue) -> String {
    match error.dyn_ref::<js_sys::Error>() {
        Some(error) => format!("{}: {}", error.name(), error.message()),
        None => format!("{error:?}"),
    }
}
