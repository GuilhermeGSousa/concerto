use std::collections::VecDeque;

use arboard::{Clipboard, Error};

use super::{text_read, ClipboardBackend, ClipboardRead};

/// The operating system's clipboard, read synchronously and answered the next frame.
pub struct OsClipboard {
    clipboard: Clipboard,
    reads: VecDeque<ClipboardRead>,
}

impl OsClipboard {
    /// Opens the operating system's clipboard, or says why it cannot.
    pub fn open() -> Result<Self, String> {
        Ok(Self {
            clipboard: Clipboard::new().map_err(|error| error.to_string())?,
            reads: VecDeque::new(),
        })
    }
}

impl ClipboardBackend for OsClipboard {
    fn write(&mut self, text: &str) {
        if let Err(error) = self.clipboard.set_text(text) {
            log::warn!("Could not copy to the clipboard: {error}");
        }
    }

    fn request_read(&mut self) {
        let read = arboard_read(self.clipboard.get_text());
        self.reads.push_back(read);
    }

    fn poll_read(&mut self) -> Option<ClipboardRead> {
        self.reads.pop_front()
    }
}

fn arboard_read(result: Result<String, Error>) -> ClipboardRead {
    match result {
        Err(Error::ContentNotAvailable | Error::ConversionFailure) => ClipboardRead::NoText,
        result => text_read(result.map_err(|error| error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_on_the_clipboard_is_read_as_text() {
        assert_eq!(
            arboard_read(Ok("bunny".into())),
            ClipboardRead::Text("bunny".into())
        );
    }

    #[test]
    fn an_empty_or_non_text_clipboard_reads_as_no_text() {
        assert_eq!(arboard_read(Ok(String::new())), ClipboardRead::NoText);
        assert_eq!(
            arboard_read(Err(Error::ContentNotAvailable)),
            ClipboardRead::NoText
        );
        assert_eq!(
            arboard_read(Err(Error::ConversionFailure)),
            ClipboardRead::NoText
        );
    }

    #[test]
    fn access_errors_read_as_unavailable() {
        for error in [
            Error::ClipboardOccupied,
            Error::ClipboardNotSupported,
            Error::Unknown {
                description: "no display".into(),
            },
        ] {
            let reason = error.to_string();
            assert_eq!(arboard_read(Err(error)), ClipboardRead::Unavailable(reason));
        }
    }
}
