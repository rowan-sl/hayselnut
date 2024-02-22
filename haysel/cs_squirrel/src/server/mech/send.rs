//! Cursor buffer reference. used in server state machine

pub struct Send<'buf> {
    inner: &'buf [u8],
    last: Option<&'buf [u8]>,
    cursor: usize,
}

impl<'buf> Send<'buf> {
    pub const fn new(buf: &'buf [u8]) -> Self {
        Self {
            inner: buf,
            last: None,
            cursor: 0,
        }
    }

    pub fn done_sending(&self) -> bool {
        self.inner.len() == self.cursor
    }

    /// returns the next (max) `by` bytes of `buf`. None if that would be zero bytes,
    /// and less than `by` if there are only that many left
    ///
    /// advances self.cursor by `by`
    pub(in crate::server::mech) fn advance(&mut self, by: usize) -> Option<&[u8]> {
        if self.done_sending() {
            self.last = None;
            None?
        }
        let buf = &self.inner[self.cursor..];
        let amnt = std::cmp::min(buf.len(), by);
        self.cursor += amnt;
        let part = &buf[..amnt];
        self.last = Some(part);
        Some(part)
    }

    /// gets the *last* buffer provided by a call to `advance`
    pub(in crate::server::mech) fn prev(&self) -> Option<&[u8]> {
        self.last
    }
}
