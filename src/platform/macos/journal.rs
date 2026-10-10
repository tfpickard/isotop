//! The macOS unified log is not read yet.

use std::io;
use std::process::Child;

use crate::platform::JournalParser;

pub fn journal() -> io::Result<(Child, JournalParser)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "journal not available yet",
    ))
}
