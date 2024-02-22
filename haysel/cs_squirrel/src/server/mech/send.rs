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

#[test]
fn it_works() {
    let buf = [0u8; 10];
    let _send = Send::new(&buf);
}

#[test]
fn advance_once() {
    let buf = [1, 2, 3, 4, 5];
    let mut send = Send::new(&buf);
    let first_3 = send.advance(3).unwrap();
    assert_eq!(first_3, &[1, 2, 3]);
}

#[test]
fn advance_once_check_last() {
    let buf = [1, 2, 3, 4, 5];
    let mut send = Send::new(&buf);
    let first_3 = send.advance(3).unwrap();
    assert_eq!(first_3, &[1, 2, 3]);
    assert_eq!(Some(first_3.to_vec()), send.prev().map(|s| s.to_vec()));
}

#[test]
fn advance_twice() {
    let buf = [1, 2, 3, 4, 5, 6, 7];
    let mut send = Send::new(&buf);
    let first_3 = send.advance(3).unwrap();
    assert_eq!(first_3, &[1, 2, 3]);
    let next_2 = send.advance(2).unwrap();
    assert_eq!(next_2, &[4, 5]);
}

#[test]
fn advance_past_end() {
    let buf = [1, 2, 3, 4, 5, 6, 7];
    let mut send = Send::new(&buf);
    let first_3 = send.advance(3).unwrap();
    assert_eq!(first_3, &[1, 2, 3]);
    let next_4 = send.advance(10).unwrap();
    assert_eq!(next_4, &[4, 5, 6, 7]);
}

#[test]
fn advance_returns_none() {
    let buf = [1, 2, 3, 4, 5, 6, 7];
    let mut send = Send::new(&buf);
    let first_3 = send.advance(3).unwrap();
    assert_eq!(first_3, &[1, 2, 3]);
    let next_4 = send.advance(10).unwrap();
    assert_eq!(next_4, &[4, 5, 6, 7]);
    let none = send.advance(0);
    assert_eq!(none, None);
}
