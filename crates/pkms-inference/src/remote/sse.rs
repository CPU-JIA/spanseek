use super::Error;

/// Byte framing handles split UTF-8, LF/CRLF/CR, comments and multiline data.
pub(super) struct Sse {
    line: Vec<u8>,
    data: Vec<String>,
    event: String,
    size: usize,
    limit: usize,
    cr: bool,
    first: bool,
}
pub(super) struct Frame {
    pub event: String,
    pub data: String,
}
impl Sse {
    pub fn new(limit: usize) -> Self {
        Self {
            line: Vec::new(),
            data: Vec::new(),
            event: String::new(),
            size: 0,
            limit,
            cr: false,
            first: true,
        }
    }
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut emit: impl FnMut(Frame) -> Result<(), Error>,
    ) -> Result<(), Error> {
        for &byte in bytes {
            if self.cr {
                self.cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            self.size = self.size.checked_add(1).ok_or(Error::ResponseTooLarge)?;
            if self.size > self.limit {
                return Err(Error::ResponseTooLarge);
            }
            if byte == b'\r' || byte == b'\n' {
                self.cr = byte == b'\r';
                let raw = std::mem::take(&mut self.line);
                let mut line = std::str::from_utf8(&raw).map_err(|_| Error::InvalidResponse)?;
                if self.first {
                    line = line.strip_prefix('\u{feff}').unwrap_or(line);
                    self.first = false;
                }
                if line.is_empty() {
                    if !self.data.is_empty() {
                        emit(Frame {
                            event: std::mem::take(&mut self.event),
                            data: self.data.join("\n"),
                        })?;
                    }
                    self.data.clear();
                    self.event.clear();
                    self.size = 0;
                } else if !line.starts_with(':') {
                    let (field, value) = line.split_once(':').unwrap_or((line, ""));
                    let value = value.strip_prefix(' ').unwrap_or(value);
                    match field {
                        "data" => self.data.push(value.to_owned()),
                        "event" => self.event = value.to_owned(),
                        _ => {}
                    }
                }
            } else {
                self.line.push(byte);
            }
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.line.is_empty() && self.data.is_empty() && self.event.is_empty()
    }
}
