use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::model::ContentBlock;
use serde_json::json;
use tokio::sync::oneshot;

/// A failed tool call, answered as `isError` content `{"code", "message"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}

impl ToolError {
    /// An error with a stable `code` and a `message` that says what to do next.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn content(&self) -> ContentBlock {
        ContentBlock::text(json!({ "code": self.code, "message": self.message }).to_string())
    }
}

pub(crate) enum Answer {
    Content(Vec<ContentBlock>),
    Error(ToolError),
}

type Slot = Option<oneshot::Sender<Answer>>;

/// Where a tool call's answer goes; only the first answer is sent.
#[derive(Clone)]
pub struct Reply {
    slot: Arc<Mutex<Slot>>,
}

impl Reply {
    pub(crate) fn new() -> (Self, oneshot::Receiver<Answer>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                slot: Arc::new(Mutex::new(Some(sender))),
            },
            receiver,
        )
    }

    /// Answers with text.
    pub fn ok(&self, text: impl Into<String>) {
        self.send(Answer::Content(vec![ContentBlock::text(text.into())]));
    }

    /// Answers with a PNG image, followed by `extra_text` unless it is empty.
    pub fn ok_image(&self, png: &[u8], extra_text: impl Into<String>) {
        let mut content = vec![ContentBlock::image(STANDARD.encode(png), "image/png")];
        let extra_text = extra_text.into();
        if !extra_text.is_empty() {
            content.push(ContentBlock::text(extra_text));
        }
        self.send(Answer::Content(content));
    }

    /// Answers with an error.
    pub fn err(&self, error: ToolError) {
        self.send(Answer::Error(error));
    }

    /// Whether the call is still waiting for an answer.
    pub fn is_open(&self) -> bool {
        self.slot()
            .as_ref()
            .is_some_and(|sender| !sender.is_closed())
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, answer: Answer) {
        if let Some(sender) = self.slot().take() {
            let _ = sender.send(answer);
        }
    }
}
