use std::{collections::VecDeque, sync::Mutex};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
use native::OsClipboard;
#[cfg(target_arch = "wasm32")]
use web::OsClipboard;

use concerto_ecs::{
    events::{event_writer::EventWriter, Event},
    resource::{ResMut, Resource},
};

/// What a clipboard backend found when asked to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardRead {
    /// The clipboard held this text.
    Text(String),
    /// The clipboard held nothing that reads as text.
    NoText,
    /// The clipboard could not be read, or the platform refused; the reason is logged.
    Unavailable(String),
}

fn text_read(result: Result<String, String>) -> ClipboardRead {
    match result {
        Ok(text) if text.is_empty() => ClipboardRead::NoText,
        Ok(text) => ClipboardRead::Text(text),
        Err(reason) => ClipboardRead::Unavailable(reason),
    }
}

/// Where [`WindowClipboard`] sends copies and asks for reads; a read may be answered on any later frame.
pub trait ClipboardBackend: Send + 'static {
    /// Puts `text` on the clipboard.
    fn write(&mut self, text: &str);
    /// Starts a read, answered through [`ClipboardBackend::poll_read`].
    fn request_read(&mut self);
    /// The next finished read, in the order they were requested.
    fn poll_read(&mut self) -> Option<ClipboardRead>;
}

/// A clipboard that lives only in this process and answers each read the next frame.
#[derive(Default)]
pub struct InProcessClipboard {
    text: String,
    reads: VecDeque<ClipboardRead>,
}

impl ClipboardBackend for InProcessClipboard {
    fn write(&mut self, text: &str) {
        self.text = text.to_owned();
    }

    fn request_read(&mut self) {
        self.reads.push_back(ClipboardRead::Text(self.text.clone()));
    }

    fn poll_read(&mut self) -> Option<ClipboardRead> {
        self.reads.pop_front()
    }
}

/// Text answering a [`WindowClipboard::request_paste`], empty when the clipboard held no text.
#[derive(Event, Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPasted(pub String);

/// The clipboard UI controls copy to and paste from: an in-process copy in front of a backend, in-process only by default.
#[derive(Resource)]
pub struct WindowClipboard {
    copy: String,
    backend: Mutex<Box<dyn ClipboardBackend>>,
    warned: bool,
}

impl Default for WindowClipboard {
    fn default() -> Self {
        Self::with_backend(InProcessClipboard::default())
    }
}

impl WindowClipboard {
    /// A clipboard that copies to and pastes from `backend`.
    pub fn with_backend(backend: impl ClipboardBackend) -> Self {
        Self {
            copy: String::new(),
            backend: Mutex::new(Box::new(backend)),
            warned: false,
        }
    }

    /// Replaces the backend, keeping the in-process copy.
    pub fn set_backend(&mut self, backend: impl ClipboardBackend) {
        self.backend = Mutex::new(Box::new(backend));
    }

    /// Copies to and pastes from the platform's clipboard, keeping the current backend with a warning when it cannot be opened.
    pub fn use_os_clipboard(&mut self) {
        match OsClipboard::open() {
            Ok(os) => self.set_backend(os),
            Err(reason) => {
                log::warn!("Clipboard unavailable, copies stay in this process: {reason}")
            }
        }
    }

    /// Stores `text` as the in-process copy and puts it on the backend's clipboard.
    pub fn write(&mut self, text: impl Into<String>) {
        self.copy = text.into();
        unlock(&mut self.backend).write(&self.copy);
    }

    /// Asks the backend for its text; the answer arrives as a [`ClipboardPasted`] on a later frame.
    pub fn request_paste(&mut self) {
        unlock(&mut self.backend).request_read();
    }

    fn next_answer(&mut self) -> Option<String> {
        let read = unlock(&mut self.backend).poll_read()?;
        Some(match read {
            ClipboardRead::Text(text) => text,
            ClipboardRead::NoText => String::new(),
            ClipboardRead::Unavailable(reason) => {
                if !self.warned {
                    self.warned = true;
                    log::warn!("Clipboard unavailable, pasting the in-process copy: {reason}");
                }
                self.copy.clone()
            }
        })
    }
}

fn unlock(backend: &mut Mutex<Box<dyn ClipboardBackend>>) -> &mut dyn ClipboardBackend {
    backend
        .get_mut()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
}

/// Sends a [`ClipboardPasted`] for each paste request the backend has answered.
pub fn answer_paste_requests(
    mut clipboard: ResMut<WindowClipboard>,
    mut pasted: EventWriter<ClipboardPasted>,
) {
    while let Some(text) = clipboard.next_answer() {
        pasted.write(ClipboardPasted(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Answers {
        next: ClipboardRead,
        pending: Option<ClipboardRead>,
    }

    impl Answers {
        fn with(next: ClipboardRead) -> Self {
            Self {
                next,
                pending: None,
            }
        }
    }

    impl ClipboardBackend for Answers {
        fn write(&mut self, _: &str) {}

        fn request_read(&mut self) {
            self.pending = Some(self.next.clone());
        }

        fn poll_read(&mut self) -> Option<ClipboardRead> {
            self.pending.take()
        }
    }

    fn answers(clipboard: &mut WindowClipboard) -> Vec<String> {
        std::iter::from_fn(|| clipboard.next_answer()).collect()
    }

    #[test]
    fn a_write_is_pasted_back_from_the_in_process_clipboard() {
        let mut clipboard = WindowClipboard::default();
        clipboard.write("bunny");
        assert!(answers(&mut clipboard).is_empty());

        clipboard.request_paste();
        assert_eq!(answers(&mut clipboard), ["bunny"]);
        assert!(answers(&mut clipboard).is_empty());
    }

    #[test]
    fn each_paste_request_is_answered_once_in_order() {
        let mut clipboard = WindowClipboard::default();
        clipboard.write("one");
        clipboard.request_paste();
        clipboard.write("two");
        clipboard.request_paste();
        assert_eq!(answers(&mut clipboard), ["one", "two"]);
    }

    #[test]
    fn a_failed_read_answers_with_the_in_process_copy() {
        let mut clipboard = WindowClipboard::with_backend(Answers::with(
            ClipboardRead::Unavailable("access denied".into()),
        ));
        clipboard.write("kept here");
        clipboard.request_paste();
        assert_eq!(answers(&mut clipboard), ["kept here"]);
        clipboard.request_paste();
        assert_eq!(answers(&mut clipboard), ["kept here"]);
    }

    #[test]
    fn a_clipboard_without_text_answers_empty() {
        let mut clipboard = WindowClipboard::with_backend(Answers::with(ClipboardRead::NoText));
        clipboard.write("kept here");
        clipboard.request_paste();
        assert_eq!(answers(&mut clipboard), [""]);
    }

    #[test]
    fn the_backends_text_wins_over_the_in_process_copy() {
        let mut clipboard =
            WindowClipboard::with_backend(Answers::with(ClipboardRead::Text("from the OS".into())));
        clipboard.write("kept here");
        clipboard.request_paste();
        assert_eq!(answers(&mut clipboard), ["from the OS"]);
    }

    #[test]
    fn read_text_maps_to_text_no_text_or_unavailable() {
        assert_eq!(
            text_read(Ok("bunny".into())),
            ClipboardRead::Text("bunny".into())
        );
        assert_eq!(text_read(Ok(String::new())), ClipboardRead::NoText);
        assert_eq!(
            text_read(Err("NotAllowedError".into())),
            ClipboardRead::Unavailable("NotAllowedError".into())
        );
    }
}
