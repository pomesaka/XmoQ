//! A cursor over received bytes that separates "not enough bytes yet" from
//! "these bytes are wrong".

use crate::{Error, varint};

pub(crate) enum ReadError {
    /// The input ended inside a field. On a stream this means "wait for more";
    /// inside a length-delimited region it means the length was wrong.
    Short,
    Invalid(Error),
}

impl From<Error> for ReadError {
    fn from(error: Error) -> Self {
        Self::Invalid(error)
    }
}

pub(crate) type ReadResult<T> = Result<T, ReadError>;

pub(crate) struct Reader<'a> {
    /// What is left to read.
    rest: &'a [u8],
    consumed: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self {
            rest: buf,
            consumed: 0,
        }
    }

    /// Bytes read so far.
    pub(crate) fn position(&self) -> usize {
        self.consumed
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    pub(crate) fn u8(&mut self) -> ReadResult<u8> {
        let (first, rest) = self.rest.split_first().ok_or(ReadError::Short)?;
        self.advance(rest, 1);
        Ok(*first)
    }

    pub(crate) fn u16(&mut self) -> ReadResult<u16> {
        let (bytes, rest) = self.rest.split_first_chunk::<2>().ok_or(ReadError::Short)?;
        self.advance(rest, 2);
        Ok(u16::from_be_bytes(*bytes))
    }

    pub(crate) fn varint(&mut self) -> ReadResult<u64> {
        let (value, len) = varint::decode(self.rest).ok_or(ReadError::Short)?;
        self.bytes(len)?;
        Ok(value)
    }

    /// A varint used as a byte count. Nothing is allocated from it: `bytes` only
    /// hands out what has already arrived.
    pub(crate) fn length(&mut self) -> ReadResult<usize> {
        let value = self.varint()?;
        usize::try_from(value)
            .map_err(|_| Error::ProtocolViolation("length does not fit in memory").into())
    }

    pub(crate) fn bytes(&mut self, len: usize) -> ReadResult<&'a [u8]> {
        let (bytes, rest) = self.rest.split_at_checked(len).ok_or(ReadError::Short)?;
        self.advance(rest, len);
        Ok(bytes)
    }

    /// Skips whatever is left, for a payload whose remaining fields are not
    /// decoded.
    pub(crate) fn skip_rest(&mut self) {
        let len = self.rest.len();
        self.advance(&[], len);
    }

    fn advance(&mut self, rest: &'a [u8], by: usize) {
        self.rest = rest;
        self.consumed = self.consumed.saturating_add(by);
    }
}

/// Parses a region whose length is already known, such as a message payload.
///
/// Running out of bytes or leaving some unread both mean the stated length did
/// not match the content, which is a `PROTOCOL_VIOLATION` for control messages (§10).
pub(crate) fn parse_exact<T>(
    region: &[u8],
    parse: impl FnOnce(&mut Reader<'_>) -> ReadResult<T>,
) -> Result<T, Error> {
    let mut reader = Reader::new(region);
    let value = match parse(&mut reader) {
        Ok(value) => value,
        Err(ReadError::Short) => {
            return Err(Error::ProtocolViolation(
                "length is shorter than the fields it holds",
            ));
        }
        Err(ReadError::Invalid(error)) => return Err(error),
    };
    if !reader.is_empty() {
        return Err(Error::ProtocolViolation(
            "length is longer than the fields it holds",
        ));
    }
    Ok(value)
}

/// Appends a varint count, for lengths and numbers of elements.
pub(crate) fn put_count(count: usize, out: &mut Vec<u8>) {
    // usize is at most 64 bits on every target Rust supports, so the fallback is
    // never taken; it only avoids a cast that could truncate.
    varint::encode(u64::try_from(count).unwrap_or(u64::MAX), out);
}

pub(crate) fn put_length_prefixed(bytes: &[u8], out: &mut Vec<u8>) {
    put_count(bytes.len(), out);
    out.extend_from_slice(bytes);
}

/// A receive buffer that is consumed from the front.
///
/// WHY a read offset instead of removing consumed bytes each time: removing
/// shifts everything that is left, so a peer packing many tiny messages into one
/// buffer would make every one of them cost a copy of the whole remainder.
/// Consumed bytes are dropped once per `push` instead.
#[derive(Debug)]
pub(crate) struct ReceiveBuffer {
    bytes: Vec<u8>,
    consumed: usize,
    limit: usize,
}

impl ReceiveBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            consumed: 0,
            limit,
        }
    }

    /// Appends `data`, or fails with the limit when the unread bytes would
    /// exceed it. What that means differs by stream, so the caller picks the error.
    pub(crate) fn push(&mut self, data: &[u8]) -> Result<(), usize> {
        if self.unread().len().saturating_add(data.len()) > self.limit {
            return Err(self.limit);
        }
        self.bytes.drain(..self.consumed);
        self.consumed = 0;
        self.bytes.extend_from_slice(data);
        Ok(())
    }

    pub(crate) fn unread(&self) -> &[u8] {
        self.bytes.get(self.consumed..).unwrap_or_default()
    }

    pub(crate) fn consume(&mut self, len: usize) {
        self.consumed = self.consumed.saturating_add(len).min(self.bytes.len());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.unread().is_empty()
    }
}
