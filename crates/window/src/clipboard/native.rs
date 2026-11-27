use std::collections::VecDeque;

use arboard::{Clipboard, Error};

use super::{ClipboardBackend, ClipboardRead};

/// The operating system's clipboard, read synchronously and answered the next frame.
pub struct OsClipboard {
    clipboard: Clipboard,
    reads: VecDeque<ClipboardRead>,
}

impl OsClipboard {
    /// Opens the operating system's clipboard.
    ///
    /// # Errors
    ///
    /// Returns the platform's reason when the clipboard cannot be opened, as on
    /// a Linux session with no display server.
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
        let read = ClipboardRead::from_arboard(self.clipboard.get_text());
        self.reads.push_back(read);
    }

    fn poll_read(&mut self) -> Option<ClipboardRead> {
        self.reads.pop_front()
    }
}

impl ClipboardRead {
    fn from_arboard(result: Result<String, Error>) -> Self {
        match result {
            Err(Error::ContentNotAvailable | Error::ConversionFailure) => Self::NoText,
            result => Self::from_text(result.map_err(|error| error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Read {
        read: Result<String, Error>,
        expected: ClipboardRead,
    }

    #[test]
    fn a_clipboard_read_is_text_no_text_or_unavailable_with_the_reason() {
        let unavailable = |error: Error| Read {
            expected: ClipboardRead::Unavailable(error.to_string()),
            read: Err(error),
        };
        let cases = [
            Read {
                read: Ok("bunny".into()),
                expected: ClipboardRead::Text("bunny".into()),
            },
            Read {
                read: Ok(String::new()),
                expected: ClipboardRead::NoText,
            },
            Read {
                read: Err(Error::ContentNotAvailable),
                expected: ClipboardRead::NoText,
            },
            unavailable(Error::ClipboardOccupied),
        ];
        for case in cases {
            assert_eq!(ClipboardRead::from_arboard(case.read), case.expected);
        }
    }
}
