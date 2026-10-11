//! The unidirectional stream that carries the Objects of a FETCH (§11.4.4):
//! a `FETCH_HEADER`, then Objects and End of Range indicators.
//!
//! An Object on such a stream says less than one on a subgroup stream where
//! it can: each field may be left out and taken from the Object before it.
//! So the stream is decoded and encoded in order, with that Object in mind.

use crate::coding::{
    ReadError, ReadResult, Reader, ReceiveBuffer, parse_exact, put_length_prefixed,
};
use crate::data::{STREAM_TYPE_FETCH_HEADER, object_properties_fault};
use crate::kvp::{self, KeyValuePair};
use crate::name::Location;
use crate::params::GroupOrder;
use crate::{Error, UsageError, varint};

/// §11.4.4.1, Table 8: how the Subgroup ID is given.
const SUBGROUP_MODE_MASK: u64 = 0x03;
const SUBGROUP_ZERO: u64 = 0x00;
const SUBGROUP_OF_THE_PRIOR: u64 = 0x01;
const SUBGROUP_OF_THE_PRIOR_PLUS_ONE: u64 = 0x02;
const SUBGROUP_PRESENT: u64 = 0x03;
/// §11.4.4.1, Table 9.
const OBJECT_ID_PRESENT: u64 = 0x04;
const GROUP_ID_PRESENT: u64 = 0x08;
const PRIORITY_PRESENT: u64 = 0x10;
const PROPERTIES_PRESENT: u64 = 0x20;
const DATAGRAM: u64 = 0x40;
/// §11.4.4: "When less than 128, the bits represent flags".
const FIRST_VALUE_THAT_IS_NO_FLAGS: u64 = 0x80;
const END_OF_NON_EXISTENT_RANGE: u64 = 0x8c;
const END_OF_UNKNOWN_RANGE: u64 = 0x10c;

/// `FETCH_HEADER` (§11.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchHeader {
    /// The Request ID of the FETCH the stream answers.
    pub request_id: u64,
}

/// One Object as a fetch stream gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchObject {
    /// Group ID.
    pub group_id: u64,
    /// Subgroup ID; `None` for an Object whose Forwarding Preference is
    /// Datagram, which has none (§11.4.4.1).
    pub subgroup_id: Option<u64>,
    /// Object ID within the Group.
    pub object_id: u64,
    /// Publisher Priority.
    pub publisher_priority: u8,
    /// Object Properties (§11.2.1.2).
    pub properties: Vec<KeyValuePair>,
    /// Object Payload. A fetch stream has no Object Status field.
    pub payload: Vec<u8>,
}

/// What an End of Range indicator says of the Objects up to its Location
/// (§11.4.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeEnd {
    /// 0x8C: they do not exist.
    NonExistent,
    /// 0x10C: the sender does not know.
    Unknown,
}

impl RangeEnd {
    fn flags(self) -> u64 {
        match self {
            Self::NonExistent => END_OF_NON_EXISTENT_RANGE,
            Self::Unknown => END_OF_UNKNOWN_RANGE,
        }
    }
}

/// What a fetch stream yields, in order: its header once, then Objects and
/// End of Range indicators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchEvent {
    /// The `FETCH_HEADER`.
    Header(FetchHeader),
    /// The next Object.
    Object(FetchObject),
    /// "All Objects with Locations between the last serialized Object, if
    /// any, and this Location, inclusive, either do not exist … or are
    /// unknown" (§11.4.4.2).
    EndOfRange {
        /// Which of the two.
        kind: RangeEnd,
        /// The last Location the indicator covers.
        location: Location,
    },
}

/// What the fields that an Object leaves out are taken from.
#[derive(Debug, Clone, Copy)]
struct Prior {
    /// Of the Object or the End of Range indicator before: "Prior Group ID
    /// and prior Object ID: The values from the End of Range indicator"
    /// (§11.4.4.2).
    location: Location,
    /// The Subgroup ID and the Publisher Priority of the last Object, which
    /// an indicator has none of. `None` before the first Object.
    object: Option<(Option<u64>, u8)>,
}

impl Prior {
    fn after_object(object: &FetchObject) -> Self {
        Self {
            location: Location {
                group: object.group_id,
                object: object.object_id,
            },
            object: Some((object.subgroup_id, object.publisher_priority)),
        }
    }

    /// `before` is what was prior to the indicator.
    fn after_range_end(before: Option<Self>, location: Location) -> Self {
        Self {
            location,
            object: before.and_then(|prior| prior.object),
        }
    }
}

/// Decodes the `FETCH_HEADER` a fetch stream begins with. Returns it and the
/// number of bytes it took, or `None` until it has arrived. For a reader that
/// passes the rest of the stream on as it is, and so has no use for a
/// [`FetchStreamDecoder`].
///
/// # Errors
///
/// [`Error::ProtocolViolation`] when the stream is of another type.
pub fn decode_fetch_header(buf: &[u8]) -> Result<Option<(FetchHeader, usize)>, Error> {
    let mut reader = Reader::new(buf);
    match FetchStreamDecoder::decode_header(&mut reader) {
        Ok(header) => Ok(Some((header, reader.position()))),
        Err(ReadError::Short) => Ok(None),
        Err(ReadError::Invalid(error)) => Err(error),
    }
}

/// The Group ID a Group ID Delta stands for after `prior` (§11.4.4.1).
fn group_after(prior: u64, delta: u64, order: GroupOrder) -> Option<u64> {
    let step = delta.checked_add(1)?;
    match order {
        GroupOrder::Ascending => prior.checked_add(step),
        GroupOrder::Descending => prior.checked_sub(step),
    }
}

/// Decodes one fetch stream from the chunks it arrives in.
#[derive(Debug)]
pub struct FetchStreamDecoder {
    buffer: ReceiveBuffer,
    buffer_limit: usize,
    group_order: GroupOrder,
    header: Option<FetchHeader>,
    prior: Option<Prior>,
}

impl FetchStreamDecoder {
    /// `buffer_limit` caps the bytes held at once, and with them the size of
    /// an Object, as for a subgroup stream. `group_order` is the one the
    /// FETCH asked for, Ascending where it named none (§10.2.8): a Group ID
    /// Delta counts upwards or downwards by it.
    #[must_use]
    pub fn new(buffer_limit: usize, group_order: GroupOrder) -> Self {
        Self {
            buffer: ReceiveBuffer::new(buffer_limit),
            buffer_limit,
            group_order,
            header: None,
            prior: None,
        }
    }

    /// Adds bytes read from the stream.
    ///
    /// # Errors
    ///
    /// [`Error::ObjectTooLarge`] when the bytes not yet decoded would exceed
    /// the limit, so drain with [`Self::next_event`] after every push. The
    /// error ends this stream only.
    pub fn push(&mut self, data: &[u8]) -> Result<(), Error> {
        self.buffer
            .push(data)
            .map_err(|limit| Error::ObjectTooLarge { limit })
    }

    /// The header first, then one Object or End of Range indicator per call;
    /// `None` until more bytes are pushed.
    ///
    /// # Errors
    ///
    /// [`Error::MalformedTrack`] for Properties that break the track and
    /// [`Error::ObjectTooLarge`] for an Object over the limit end this stream
    /// and leave the session open. Anything else is a protocol error that
    /// closes the session: Serialization Flags the draft does not define, a
    /// field taken from an Object that was not there, an ID past 64 bits.
    pub fn next_event(&mut self) -> Result<Option<FetchEvent>, Error> {
        let mut reader = Reader::new(self.buffer.unread());
        let parsed = match self.header {
            None => Self::decode_header(&mut reader).map(FetchEvent::Header),
            Some(_) => self.decode_item(&mut reader),
        };
        let event = match parsed {
            Ok(event) => event,
            Err(ReadError::Short) => return Ok(None),
            Err(ReadError::Invalid(error)) => return Err(error),
        };
        let used = reader.position();
        self.buffer.consume(used);
        match &event {
            FetchEvent::Header(header) => self.header = Some(*header),
            FetchEvent::Object(object) => self.prior = Some(Prior::after_object(object)),
            FetchEvent::EndOfRange { location, .. } => {
                self.prior = Some(Prior::after_range_end(self.prior, *location));
            }
        }
        Ok(Some(event))
    }

    /// Call when the stream ends with a FIN.
    ///
    /// # Errors
    ///
    /// [`Error::ProtocolViolation`] when undecoded bytes are left, as for a
    /// subgroup stream (§11.4), and when the stream ended before its
    /// `FETCH_HEADER` was there: a fetch without Objects still has "the
    /// `FETCH_HEADER`" before the FIN (§10.12.3).
    pub fn finish(&self) -> Result<(), Error> {
        if self.header.is_none() {
            return Err(Error::ProtocolViolation(
                "stream ended before its FETCH_HEADER",
            ));
        }
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(Error::ProtocolViolation(
                "stream ended inside a serialized object",
            ))
        }
    }

    fn decode_header(reader: &mut Reader<'_>) -> ReadResult<FetchHeader> {
        if reader.varint()? != STREAM_TYPE_FETCH_HEADER {
            return Err(Error::ProtocolViolation("not a fetch stream").into());
        }
        Ok(FetchHeader {
            request_id: reader.varint()?,
        })
    }

    fn decode_item(&self, reader: &mut Reader<'_>) -> ReadResult<FetchEvent> {
        let flags = reader.varint()?;
        if flags >= FIRST_VALUE_THAT_IS_NO_FLAGS {
            let kind = match flags {
                END_OF_NON_EXISTENT_RANGE => RangeEnd::NonExistent,
                END_OF_UNKNOWN_RANGE => RangeEnd::Unknown,
                // §11.4.4: "Any other value is a PROTOCOL_VIOLATION."
                _ => return Err(Error::ProtocolViolation("unknown serialization flags").into()),
            };
            // WHY read as a Location and not as deltas, though the figure of
            // §11.4.4 names the fields Group ID Delta and Object ID Delta:
            // §11.4.4.2 has "the Group ID and Object ID fields are present"
            // and calls what they give "this Location".
            let location = Location::decode(reader)?;
            return Ok(FetchEvent::EndOfRange { kind, location });
        }
        self.decode_object(flags, reader).map(FetchEvent::Object)
    }

    fn decode_object(&self, flags: u64, reader: &mut Reader<'_>) -> ReadResult<FetchObject> {
        // §11.4.4.1: "If the first Object in the FETCH response uses a flag
        // that references fields in the prior Object, the Subscriber MUST
        // close the session with a PROTOCOL_VIOLATION."
        let no_prior = |what| Error::ProtocolViolation(what);
        let too_large = Error::ProtocolViolation("id exceeds 64 bits");
        let group_present = flags & GROUP_ID_PRESENT != 0;

        let group_id = match (group_present, self.prior) {
            (true, None) => reader.varint()?,
            (true, Some(prior)) => {
                group_after(prior.location.group, reader.varint()?, self.group_order)
                    .ok_or(Error::ProtocolViolation("group id out of range"))?
            }
            (false, Some(prior)) => prior.location.group,
            (false, None) => return Err(no_prior("no prior group id").into()),
        };

        let prior_subgroup = || {
            self.prior
                .and_then(|prior| prior.object)
                .and_then(|(subgroup_id, _)| subgroup_id)
                .ok_or(no_prior("no prior subgroup id"))
        };
        // §11.4.4.1: with 0x40 "the subscriber MUST ignore the bits".
        let subgroup_id = if flags & DATAGRAM != 0 {
            None
        } else {
            Some(match flags & SUBGROUP_MODE_MASK {
                SUBGROUP_ZERO => 0,
                SUBGROUP_OF_THE_PRIOR => prior_subgroup()?,
                SUBGROUP_OF_THE_PRIOR_PLUS_ONE => {
                    prior_subgroup()?.checked_add(1).ok_or(too_large.clone())?
                }
                _ => reader.varint()?,
            })
        };

        let object_id = match (flags & OBJECT_ID_PRESENT != 0, self.prior) {
            // "When the Group ID Delta field is present, the Object ID is the
            // value of Object ID Delta if present."
            (true, None) => reader.varint()?,
            (true, Some(_)) if group_present => reader.varint()?,
            (true, Some(prior)) => prior
                .location
                .object
                .checked_add(reader.varint()?)
                .ok_or(too_large.clone())?,
            // "the prior Object's ID plus one, regardless of which group it
            // belongs to".
            (false, Some(prior)) => prior.location.object.checked_add(1).ok_or(too_large)?,
            (false, None) => return Err(no_prior("no prior object id").into()),
        };

        let publisher_priority = if flags & PRIORITY_PRESENT != 0 {
            reader.u8()?
        } else {
            self.prior
                .and_then(|prior| prior.object)
                .map(|(_, priority)| priority)
                .ok_or(no_prior("no prior publisher priority"))?
        };

        // WHY the Properties are decoded only once the payload is there:
        // this runs again on every call until the Object is complete, and a
        // peer that sends byte by byte would have them decoded once a byte.
        let encoded_properties = if flags & PROPERTIES_PRESENT != 0 {
            let len = self.announced_length(reader)?;
            reader.bytes(len)?
        } else {
            &[]
        };
        let payload_len = self.announced_length(reader)?;
        let payload = reader.bytes(payload_len)?.to_vec();

        let properties =
            parse_exact(encoded_properties, kvp::decode_all).map_err(|error| match error {
                Error::TooManyKeyValuePairs { limit } => Error::TooManyObjectProperties { limit },
                error => error,
            })?;
        if let Some(fault) = object_properties_fault(&properties, group_id, object_id) {
            return Err(Error::MalformedTrack(fault).into());
        }
        Ok(FetchObject {
            group_id,
            subgroup_id,
            object_id,
            publisher_priority,
            properties,
            payload,
        })
    }

    /// A length field of an Object; one over the limit is refused when it is
    /// read, as on a subgroup stream.
    fn announced_length(&self, reader: &mut Reader<'_>) -> ReadResult<usize> {
        let len = reader.length()?;
        if len > self.buffer_limit {
            return Err(Error::ObjectTooLarge {
                limit: self.buffer_limit,
            }
            .into());
        }
        Ok(len)
    }
}

/// Encodes one fetch stream: the header, then Objects and End of Range
/// indicators in the order of the fetch.
///
/// A field is left out where it can be: the Group ID is there for the first
/// Object and for a new Group; the Object ID unless it is the prior one plus
/// one in the same Group; the priority when it changes; the Subgroup ID when
/// it is neither 0 nor the prior one nor that plus one. WHY these flags and
/// no others, where several encodings give the same Object: the moqtail
/// crate 0.15.0 leaves a field out in exactly these places, so the bytes it
/// wrote can be compared with what this writes.
///
/// WHY NOT refuse an End of Range indicator that goes back, as an Object
/// that does is refused: §11.4.4.2 has an indicator cover what lies "between
/// the last serialized Object, if any, and this Location" and does not say
/// which of the two comes first in a Descending fetch. The decoder checks
/// the order of neither Objects nor indicators.
#[derive(Debug)]
pub struct FetchStreamEncoder {
    group_order: GroupOrder,
    prior: Option<Prior>,
    /// Encoded bytes not yet taken by the caller.
    bytes: Vec<u8>,
}

impl FetchStreamEncoder {
    /// An encoder whose first bytes are the header.
    #[must_use]
    pub fn new(header: FetchHeader, group_order: GroupOrder) -> Self {
        let mut bytes = Vec::new();
        varint::encode(STREAM_TYPE_FETCH_HEADER, &mut bytes);
        varint::encode(header.request_id, &mut bytes);
        Self {
            group_order,
            prior: None,
            bytes,
        }
    }

    /// The Group ID Delta for a Group that comes after the prior one.
    fn group_delta(&self, prior: u64, group_id: u64) -> Option<u64> {
        let apart = match self.group_order {
            GroupOrder::Ascending => group_id.checked_sub(prior)?,
            GroupOrder::Descending => prior.checked_sub(group_id)?,
        };
        apart.checked_sub(1)
    }

    /// Encodes the next Object.
    ///
    /// # Errors
    ///
    /// Fails, encoding nothing, when the Object does not come after what was
    /// encoded before it: an earlier Group by the Group Order, or the same
    /// Group and no larger Object ID; and when its properties break the
    /// track (§2.5.1, §12.8, §12.9).
    pub fn object(&mut self, object: &FetchObject) -> Result<(), UsageError> {
        if let Some(fault) =
            object_properties_fault(&object.properties, object.group_id, object.object_id)
        {
            return Err(UsageError(fault));
        }
        let mut flags = 0;
        let mut fields = Vec::new();

        let same_group = self
            .prior
            .is_some_and(|prior| prior.location.group == object.group_id);
        match self.prior {
            None => {
                flags |= GROUP_ID_PRESENT;
                varint::encode(object.group_id, &mut fields);
            }
            Some(_) if same_group => {}
            Some(prior) => {
                let delta = self
                    .group_delta(prior.location.group, object.group_id)
                    .ok_or(UsageError("groups must follow the group order"))?;
                flags |= GROUP_ID_PRESENT;
                varint::encode(delta, &mut fields);
            }
        }

        let prior_object = self.prior.and_then(|prior| prior.object);
        match (object.subgroup_id, prior_object.and_then(|(id, _)| id)) {
            (None, _) => flags |= DATAGRAM,
            (Some(0), _) => flags |= SUBGROUP_ZERO,
            (Some(id), Some(prior)) if id == prior => flags |= SUBGROUP_OF_THE_PRIOR,
            (Some(id), Some(prior)) if prior.checked_add(1) == Some(id) => {
                flags |= SUBGROUP_OF_THE_PRIOR_PLUS_ONE;
            }
            (Some(id), _) => {
                flags |= SUBGROUP_PRESENT;
                varint::encode(id, &mut fields);
            }
        }

        match self.prior {
            Some(prior) if same_group => {
                let ahead = object
                    .object_id
                    .checked_sub(prior.location.object)
                    .filter(|ahead| *ahead > 0)
                    .ok_or(UsageError("object ids must increase within a group"))?;
                if ahead > 1 {
                    flags |= OBJECT_ID_PRESENT;
                    varint::encode(ahead, &mut fields);
                }
            }
            _ => {
                flags |= OBJECT_ID_PRESENT;
                varint::encode(object.object_id, &mut fields);
            }
        }

        if prior_object.is_none_or(|(_, priority)| priority != object.publisher_priority) {
            flags |= PRIORITY_PRESENT;
            fields.push(object.publisher_priority);
        }
        if !object.properties.is_empty() {
            flags |= PROPERTIES_PRESENT;
            let mut properties = Vec::new();
            kvp::encode_all(&object.properties, &mut properties);
            put_length_prefixed(&properties, &mut fields);
        }
        put_length_prefixed(&object.payload, &mut fields);

        varint::encode(flags, &mut self.bytes);
        self.bytes.extend_from_slice(&fields);
        self.prior = Some(Prior::after_object(object));
        Ok(())
    }

    /// Encodes an End of Range indicator: the Objects after what was encoded
    /// before, up to and with `location`, do not exist or are not known.
    pub fn end_of_range(&mut self, kind: RangeEnd, location: Location) {
        varint::encode(kind.flags(), &mut self.bytes);
        location.encode(&mut self.bytes);
        self.prior = Some(Prior::after_range_end(self.prior, location));
    }

    /// Hands over what has been encoded since the last call.
    pub fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_util::{fixture, fixture_by_hand};

    const LIMIT: usize = 1_024;
    /// The payload moqtail's test client gives every Object.
    const PAYLOAD: [u8; 8] = [0x4d, 0x3b, 0xc3, 0x24, 0x4c, 0x84, 0x57, 0x2d];

    fn decode_stream(
        bytes: &[u8],
        chunk_len: usize,
        order: GroupOrder,
    ) -> Result<Vec<FetchEvent>, Error> {
        let mut decoder = FetchStreamDecoder::new(LIMIT, order);
        let mut events = Vec::new();
        for chunk in bytes.chunks(chunk_len) {
            decoder.push(chunk)?;
            while let Some(event) = decoder.next_event()? {
                events.push(event);
            }
        }
        decoder.finish()?;
        Ok(events)
    }

    fn encode_stream(events: &[FetchEvent], order: GroupOrder) -> Vec<u8> {
        let Some(FetchEvent::Header(header)) = events.first() else {
            panic!("the first event is the header");
        };
        let mut encoder = FetchStreamEncoder::new(*header, order);
        for event in &events[1..] {
            match event {
                FetchEvent::Header(_) => panic!("one header"),
                FetchEvent::Object(object) => encoder.object(object).unwrap(),
                FetchEvent::EndOfRange { kind, location } => encoder.end_of_range(*kind, *location),
            }
        }
        encoder.take_bytes()
    }

    /// An Object of moqtail's test client: Subgroup 1, priority 128.
    fn object(group_id: u64, object_id: u64) -> FetchEvent {
        with_properties(group_id, object_id, vec![])
    }

    fn with_properties(group_id: u64, object_id: u64, properties: Vec<KeyValuePair>) -> FetchEvent {
        FetchEvent::Object(FetchObject {
            group_id,
            subgroup_id: Some(1),
            object_id,
            publisher_priority: 128,
            properties,
            payload: PAYLOAD.to_vec(),
        })
    }

    fn header(request_id: u64) -> FetchEvent {
        FetchEvent::Header(FetchHeader { request_id })
    }

    pub(crate) fn fixture_names() -> Vec<&'static str> {
        moqtail_streams()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// What the moqtail relay 0.15.0 wrote in answer to a FETCH.
    fn moqtail_streams() -> Vec<(&'static str, Vec<FetchEvent>)> {
        // Prior Group ID Gap (0x3C) and Prior Object ID Gap (0x3E), §12.8
        // and §12.9, which the test client sets where it leaves IDs out.
        let gap = |ty| vec![KeyValuePair::varint(ty, 1).unwrap()];
        vec![
            (
                "fetch-stream-from-moqtail-relay",
                vec![
                    header(0),
                    object(0, 1),
                    object(0, 2),
                    object(1, 0),
                    object(1, 1),
                ],
            ),
            (
                "fetch-stream-five-groups-from-moqtail-relay",
                vec![
                    header(2),
                    object(2, 0),
                    object(2, 1),
                    object(2, 2),
                    object(3, 0),
                    object(3, 1),
                    object(3, 2),
                    object(4, 0),
                    object(4, 1),
                    object(4, 2),
                    object(5, 0),
                    object(5, 1),
                    object(5, 2),
                    object(6, 0),
                ],
            ),
            (
                "fetch-stream-gaps-from-moqtail-relay",
                vec![
                    header(0),
                    object(0, 0),
                    with_properties(0, 2, gap(0x3e)),
                    with_properties(2, 0, gap(0x3c)),
                    with_properties(2, 2, gap(0x3e)),
                ],
            ),
            ("fetch-stream-empty-from-moqtail-relay", vec![header(0)]),
        ]
    }

    #[test]
    fn the_header_alone_is_decoded_from_the_streams_moqtail_wrote() {
        // (the file; the Request ID). Two bytes each: the type 0x05 and a
        // Request ID of one byte.
        let cases = [
            ("fetch-stream-from-moqtail-relay", 0),
            ("fetch-stream-five-groups-from-moqtail-relay", 2),
            ("fetch-stream-gaps-from-moqtail-relay", 0),
            ("fetch-stream-empty-from-moqtail-relay", 0),
        ];
        for (name, request_id) in cases {
            let bytes = fixture(name);
            assert_eq!(
                decode_fetch_header(&bytes),
                Ok(Some((FetchHeader { request_id }, 2))),
                "{name}"
            );
            // Not there until its last byte is.
            assert_eq!(decode_fetch_header(&bytes[..1]), Ok(None), "{name}");
        }
        assert_eq!(decode_fetch_header(&[]), Ok(None));
        // A subgroup stream.
        assert_eq!(
            decode_fetch_header(&[0x10, 0x00]),
            Err(Error::ProtocolViolation("not a fetch stream"))
        );
    }

    #[test]
    fn decodes_the_streams_moqtail_wrote_whole_and_byte_by_byte() {
        for (name, events) in moqtail_streams() {
            let bytes = fixture(name);
            for chunk_len in [bytes.len(), 1] {
                assert_eq!(
                    decode_stream(&bytes, chunk_len, GroupOrder::Ascending),
                    Ok(events.clone()),
                    "{name} in chunks of {chunk_len}"
                );
            }
        }
    }

    #[test]
    fn encodes_the_bytes_moqtail_wrote() {
        for (name, events) in moqtail_streams() {
            assert_eq!(
                encode_stream(&events, GroupOrder::Ascending),
                fixture(name),
                "{name}"
            );
        }
    }

    /// `fixtures/moqt-18-by-hand/`: bytes written by hand from §11.4.4, for
    /// what no capture has. The reader of the draft and the writer of the
    /// expectation are the same, so these check nothing against another
    /// implementation; the TypeScript implementation reads the same files.
    /// The README there goes through each file byte by byte.
    fn streams_written_by_hand() -> Vec<(&'static str, GroupOrder, Vec<FetchEvent>)> {
        let plain = |group_id, subgroup_id, object_id, publisher_priority, payload: &[u8]| {
            FetchEvent::Object(FetchObject {
                group_id,
                subgroup_id,
                object_id,
                publisher_priority,
                properties: vec![],
                payload: payload.to_vec(),
            })
        };
        let end = |kind, group, object| FetchEvent::EndOfRange {
            kind,
            location: Location { group, object },
        };
        vec![
            (
                "fetch-stream-descending",
                GroupOrder::Descending,
                vec![
                    header(4),
                    plain(9, Some(0), 0, 7, &[0xaa]),
                    plain(7, Some(0), 1, 7, &[0xbb]),
                    plain(6, Some(0), 5, 7, &[0xcc]),
                ],
            ),
            (
                "fetch-stream-end-of-range",
                GroupOrder::Ascending,
                vec![
                    header(0),
                    plain(0, Some(0), 0, 7, &[0xaa]),
                    end(RangeEnd::NonExistent, 0, 5),
                    plain(0, Some(0), 6, 7, &[0xbb]),
                    end(RangeEnd::Unknown, 2, 0),
                    plain(2, Some(0), 3, 7, &[0xcc]),
                ],
            ),
            (
                "fetch-stream-indicator-first",
                GroupOrder::Ascending,
                vec![
                    header(0),
                    end(RangeEnd::Unknown, 3, 1),
                    plain(3, Some(0), 2, 7, &[]),
                ],
            ),
            (
                // Group 9 by §11.4.4.2, where §11.4.4.1 would have Group 5.
                "fetch-stream-indicator-first-group-delta",
                GroupOrder::Ascending,
                vec![
                    header(0),
                    end(RangeEnd::Unknown, 3, 1),
                    plain(9, Some(0), 0, 7, &[]),
                ],
            ),
            (
                "fetch-stream-subgroup-id-modes",
                GroupOrder::Ascending,
                vec![
                    header(0),
                    plain(1, None, 0, 7, &[]),
                    plain(2, Some(4), 0, 9, &[]),
                    plain(2, Some(5), 1, 9, &[]),
                ],
            ),
        ]
    }

    pub(crate) fn names_of_files_written_by_hand() -> Vec<&'static str> {
        streams_written_by_hand()
            .into_iter()
            .map(|(name, _, _)| name)
            .collect()
    }

    #[test]
    fn decodes_and_encodes_what_the_draft_describes() {
        for (name, order, events) in streams_written_by_hand() {
            let bytes = fixture_by_hand(name);
            for chunk_len in [bytes.len(), 1] {
                assert_eq!(
                    decode_stream(&bytes, chunk_len, order),
                    Ok(events.clone()),
                    "{name} in chunks of {chunk_len}"
                );
            }
            assert_eq!(encode_stream(&events, order), bytes, "{name}");
        }
    }

    #[test]
    fn rejects_malformed_input() {
        let violation = Error::ProtocolViolation;
        let cases: &[(&[u8], Error)] = &[
            // A subgroup stream.
            (&[0x10, 0x00], violation("not a fetch stream")),
            // A stream that ends after its type, without the Request ID.
            (&[0x05], violation("stream ended before its FETCH_HEADER")),
            // Serialization Flags 0x80, and 0x8D: 128 and above are only the
            // two End of Range values (§11.4.4).
            (
                &[0x05, 0x00, 0x80, 0x80],
                violation("unknown serialization flags"),
            ),
            (
                &[0x05, 0x00, 0x80, 0x8d],
                violation("unknown serialization flags"),
            ),
            // The first Object without a Group ID, without an Object ID,
            // without a priority, and with the Subgroup of a prior Object.
            (
                &[0x05, 0x00, 0x14, 0x00, 0x07, 0x00],
                violation("no prior group id"),
            ),
            (
                &[0x05, 0x00, 0x18, 0x00, 0x07, 0x00],
                violation("no prior object id"),
            ),
            (
                &[0x05, 0x00, 0x0c, 0x00, 0x00, 0x00],
                violation("no prior publisher priority"),
            ),
            (
                &[0x05, 0x00, 0x1d, 0x00, 0x00, 0x07, 0x00],
                violation("no prior subgroup id"),
            ),
            // After an indicator, the Subgroup and the priority of an Object
            // that never came.
            (
                &[0x05, 0x00, 0x81, 0x0c, 0x03, 0x01, 0x11, 0x07, 0x00],
                violation("no prior subgroup id"),
            ),
            (
                &[0x05, 0x00, 0x81, 0x0c, 0x03, 0x01, 0x00, 0x00],
                violation("no prior publisher priority"),
            ),
            // The Subgroup of a prior Object that was a datagram.
            (
                &[0x05, 0x00, 0x5c, 0x01, 0x00, 0x07, 0x00, 0x01, 0x00],
                violation("no prior subgroup id"),
            ),
            // Subgroup ID 2^64 - 1 present (flags 0x1F), then the prior
            // Subgroup plus one.
            (
                &[
                    0x05, 0x00, 0x1f, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                    0x00, 0x07, 0x00, 0x02, 0x00,
                ],
                violation("id exceeds 64 bits"),
            ),
            // Object ID 2^64 - 1 (flags 0x1C), then the prior Object ID plus
            // one (flags 0x00), and the prior one plus the Object ID Delta 1
            // (flags 0x04).
            (
                &[
                    0x05, 0x00, 0x1c, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                    0x07, 0x00, 0x00, 0x00,
                ],
                violation("id exceeds 64 bits"),
            ),
            (
                &[
                    0x05, 0x00, 0x1c, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                    0x07, 0x00, 0x04, 0x01, 0x00,
                ],
                violation("id exceeds 64 bits"),
            ),
            // Object Properties of 1025 bytes announced, over the limit.
            (
                &[0x05, 0x00, 0x3c, 0x00, 0x00, 0x07, 0x84, 0x01],
                Error::ObjectTooLarge { limit: LIMIT },
            ),
            // Mandatory Track Property 0x4000 on an Object (§2.5.1).
            (
                &[
                    0x05, 0x00, 0x3c, 0x00, 0x00, 0x07, 0x04, 0xc0, 0x40, 0x00, 0x01, 0x00,
                ],
                Error::MalformedTrack("mandatory track property on an object"),
            ),
            // A payload of 1025 bytes announced, over the limit of 1024.
            (
                &[0x05, 0x00, 0x1c, 0x00, 0x00, 0x07, 0x84, 0x01],
                Error::ObjectTooLarge { limit: LIMIT },
            ),
            // The stream ends inside an Object.
            (
                &[0x05, 0x00, 0x1c, 0x00, 0x00, 0x07, 0x02, 0xaa],
                violation("stream ended inside a serialized object"),
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(
                decode_stream(bytes, bytes.len(), GroupOrder::Ascending),
                Err(error.clone()),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn more_object_properties_than_the_limit_end_the_stream() {
        // Flags 0x3C, Group 0, Object 0, priority 7, then Properties of 130
        // bytes (the length 0x80 0x82): 65 pairs of a type that is 2 more
        // than the one before and the value 0. Then a payload of 0 bytes.
        let mut bytes = vec![0x05, 0x00, 0x3c, 0x00, 0x00, 0x07, 0x80, 0x82];
        for _ in 0..65 {
            bytes.extend_from_slice(&[0x02, 0x00]);
        }
        bytes.push(0x00);
        assert_eq!(
            decode_stream(&bytes, bytes.len(), GroupOrder::Ascending),
            Err(Error::TooManyObjectProperties { limit: 64 })
        );
    }

    #[test]
    fn refuses_to_encode_properties_that_break_the_track() {
        let gap = |ty, value| KeyValuePair::varint(ty, value).unwrap();
        // (the properties of an Object at {2, 3}; why it is refused).
        let cases = [
            (
                vec![gap(0x4000, 1)],
                "mandatory track property on an object",
            ),
            // Prior Group ID Gap (0x3C) twice, and larger than the Group ID.
            (
                vec![gap(0x3c, 1), gap(0x3c, 1)],
                "more than one prior group id gap",
            ),
            (
                vec![gap(0x3c, 3)],
                "prior group id gap larger than the group id",
            ),
            // Prior Object ID Gap (0x3E) larger than the Object ID.
            (
                vec![gap(0x3e, 4)],
                "prior object id gap larger than the object id",
            ),
        ];
        for (properties, why) in cases {
            let mut encoder =
                FetchStreamEncoder::new(FetchHeader { request_id: 0 }, GroupOrder::Ascending);
            encoder.take_bytes();
            let object = FetchObject {
                group_id: 2,
                subgroup_id: Some(0),
                object_id: 3,
                publisher_priority: 0,
                properties,
                payload: vec![],
            };
            assert_eq!(encoder.object(&object), Err(UsageError(why)), "{why}");
            assert_eq!(encoder.take_bytes(), [], "{why}");
        }
    }

    #[test]
    fn a_group_id_past_either_end_is_refused() {
        // (the Group Order; the first Group; the Group ID Delta of the next).
        // Descending below 0, and ascending past 2^64 - 1 (§11.4.4.1).
        let largest = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let cases: [(GroupOrder, &[u8], u8); 2] = [
            (GroupOrder::Descending, &[0x01], 0x01),
            (GroupOrder::Ascending, &largest, 0x00),
        ];
        for (order, first_group, delta) in cases {
            let mut bytes = vec![0x05, 0x00, 0x1c];
            bytes.extend_from_slice(first_group);
            bytes.extend_from_slice(&[0x00, 0x07, 0x00, 0x08, delta, 0x00]);
            assert_eq!(
                decode_stream(&bytes, bytes.len(), order),
                Err(Error::ProtocolViolation("group id out of range")),
                "{order:?}"
            );
        }
    }

    #[test]
    fn refuses_to_encode_objects_out_of_order() {
        let at = |group_id, object_id| FetchObject {
            group_id,
            subgroup_id: Some(0),
            object_id,
            publisher_priority: 0,
            properties: vec![],
            payload: vec![],
        };
        // (the Group Order; the Object before; the Object that is refused).
        let cases = [
            (
                GroupOrder::Ascending,
                at(2, 3),
                at(2, 3),
                "object ids must increase within a group",
            ),
            (
                GroupOrder::Ascending,
                at(2, 3),
                at(2, 1),
                "object ids must increase within a group",
            ),
            (
                GroupOrder::Ascending,
                at(2, 3),
                at(1, 0),
                "groups must follow the group order",
            ),
            (
                GroupOrder::Descending,
                at(2, 3),
                at(3, 0),
                "groups must follow the group order",
            ),
        ];
        for (order, before, refused, why) in cases {
            let mut encoder = FetchStreamEncoder::new(FetchHeader { request_id: 0 }, order);
            encoder.object(&before).unwrap();
            encoder.take_bytes();
            assert_eq!(encoder.object(&refused), Err(UsageError(why)), "{order:?}");
            assert_eq!(encoder.take_bytes(), [], "{order:?}");
        }
    }

    #[test]
    fn an_object_id_that_is_left_out_is_the_prior_one_plus_one_also_in_a_new_group() {
        // Flags 0x1C: Group 2, Object 5, priority 7. Then 0x08: Group ID
        // Delta 0, so Group 3, and no Object ID Delta: "the prior Object's ID
        // plus one, regardless of which group it belongs to" (§11.4.4.1), so
        // 6. The encoder of this crate does not write a new Group that way.
        let bytes = [0x05, 0x00, 0x1c, 0x02, 0x05, 0x07, 0x00, 0x08, 0x00, 0x00];
        let mut decoder = FetchStreamDecoder::new(64, GroupOrder::Ascending);
        decoder.push(&bytes).unwrap();
        let mut objects = Vec::new();
        while let Some(event) = decoder.next_event().unwrap() {
            if let FetchEvent::Object(object) = event {
                objects.push((object.group_id, object.object_id));
            }
        }
        assert_eq!(objects, [(2, 5), (3, 6)]);
    }
}
