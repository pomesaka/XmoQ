//! Unidirectional stream types (§3.4), the Subgroup Header and the Objects that
//! follow it on a subgroup stream (§11.4.2).
//!
//! FIXME: decode datagrams (§11.3). Nothing here reads them, so an
//! `OBJECT_DATAGRAM` is lost and an unknown datagram type does not close the
//! session, which §11 requires ("An endpoint that receives an unknown datagram
//! type MUST close the session").

use crate::coding::{
    ReadError, ReadResult, Reader, ReceiveBuffer, parse_exact, put_length_prefixed,
};
use crate::kvp::{self, KeyValuePair, Value};
use crate::{Error, UsageError, varint};

pub(crate) const STREAM_TYPE_FETCH_HEADER: u64 = 0x05;
const STREAM_TYPE_SETUP: u64 = 0x2f00;
const STREAM_TYPE_PADDING: u64 = 0x132b_3e28;

const PROPERTIES_BIT: u64 = 0x01;
const SUBGROUP_ID_MODE_MASK: u64 = 0x06;
const END_OF_GROUP_BIT: u64 = 0x08;
const SUBGROUP_HEADER_BIT: u64 = 0x10;
const DEFAULT_PRIORITY_BIT: u64 = 0x20;
const FIRST_OBJECT_BIT: u64 = 0x40;

const MODE_ZERO: u64 = 0b00;
const MODE_FIRST_OBJECT_ID: u64 = 0b01;
const MODE_EXPLICIT: u64 = 0b10;

/// Prior Group ID Gap (§12.8).
const PRIOR_GROUP_ID_GAP: u64 = 0x3c;
/// Prior Object ID Gap (§12.9).
const PRIOR_OBJECT_ID_GAP: u64 = 0x3e;

/// What an incoming unidirectional stream carries, told from its first varint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniStreamType {
    /// The peer's control stream, which starts with SETUP.
    Control,
    /// A subgroup stream: a Subgroup Header, then Objects.
    Subgroup,
    /// A padding stream (§11.5.1). The receiver has to discard everything on it,
    /// or may cancel it; it carries nothing for the session.
    Padding,
}

/// Classifies a unidirectional stream from its first bytes, or returns `None`
/// until the type has arrived. The type is not consumed: the SETUP message type
/// and the Subgroup Header type double as the stream type.
///
/// # Errors
///
/// [`Error::UnknownStreamType`] for a type the draft does not define, and
/// [`Error::UnsupportedStreamType`] for a fetch stream.
pub fn uni_stream_type(buf: &[u8]) -> Result<Option<UniStreamType>, Error> {
    let Some((ty, _)) = varint::decode(buf) else {
        return Ok(None);
    };
    match ty {
        STREAM_TYPE_SETUP => Ok(Some(UniStreamType::Control)),
        STREAM_TYPE_PADDING => Ok(Some(UniStreamType::Padding)),
        ty if is_subgroup_header_type(ty) => Ok(Some(UniStreamType::Subgroup)),
        STREAM_TYPE_FETCH_HEADER => Err(Error::UnsupportedStreamType(ty)),
        ty => Err(Error::UnknownStreamType(ty)),
    }
}

/// The form 0b0XX1XXXX of §11.4.2.
fn is_subgroup_header_type(ty: u64) -> bool {
    ty <= 0x7f && ty & SUBGROUP_HEADER_BIT != 0
}

/// `SUBGROUP_ID_MODE` of the Subgroup Header type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubgroupId {
    /// Mode 0b00: no field, the Subgroup ID is 0.
    Zero,
    /// Mode 0b01: no field, the Subgroup ID is the Object ID of the first Object
    /// on the stream, whatever that ID is.
    FirstObjectId,
    /// Mode 0b10: the field is present.
    Explicit(u64),
}

/// `SUBGROUP_HEADER` (§11.4.2), the start of a subgroup stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubgroupHeader {
    /// The track the stream belongs to (§11.1).
    pub track_alias: u64,
    /// Group ID of every Object on the stream.
    pub group_id: u64,
    /// Subgroup ID of every Object on the stream.
    pub subgroup_id: SubgroupId,
    /// `None` sets `DEFAULT_PRIORITY`: the field is omitted and the subgroup takes
    /// the priority of the subscription.
    pub publisher_priority: Option<u8>,
    /// PROPERTIES: every Object on the stream carries a Properties field.
    pub properties: bool,
    /// `END_OF_GROUP`: the subgroup holds the largest Object of its Group, so when
    /// the stream ends with a FIN, no later Object of the Group exists.
    pub end_of_group: bool,
    /// `FIRST_OBJECT`: the first Object on the stream is the first one the original
    /// publisher put in the subgroup.
    pub first_object: bool,
}

impl SubgroupHeader {
    fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        let ty = reader.varint()?;
        if !is_subgroup_header_type(ty) {
            return Err(Error::ProtocolViolation("not a subgroup header type").into());
        }
        let mode = (ty & SUBGROUP_ID_MODE_MASK) >> 1;
        if ![MODE_ZERO, MODE_FIRST_OBJECT_ID, MODE_EXPLICIT].contains(&mode) {
            return Err(Error::ProtocolViolation("reserved subgroup id mode").into());
        }
        let track_alias = reader.varint()?;
        let group_id = reader.varint()?;
        let subgroup_id = match mode {
            MODE_ZERO => SubgroupId::Zero,
            MODE_FIRST_OBJECT_ID => SubgroupId::FirstObjectId,
            _ => SubgroupId::Explicit(reader.varint()?),
        };
        let publisher_priority = if ty & DEFAULT_PRIORITY_BIT == 0 {
            Some(reader.u8()?)
        } else {
            None
        };
        Ok(Self {
            track_alias,
            group_id,
            subgroup_id,
            publisher_priority,
            properties: ty & PROPERTIES_BIT != 0,
            end_of_group: ty & END_OF_GROUP_BIT != 0,
            first_object: ty & FIRST_OBJECT_BIT != 0,
        })
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let mode = match self.subgroup_id {
            SubgroupId::Zero => MODE_ZERO,
            SubgroupId::FirstObjectId => MODE_FIRST_OBJECT_ID,
            SubgroupId::Explicit(_) => MODE_EXPLICIT,
        };
        let mut ty = SUBGROUP_HEADER_BIT | (mode << 1);
        for (set, bit) in [
            (self.properties, PROPERTIES_BIT),
            (self.end_of_group, END_OF_GROUP_BIT),
            (self.publisher_priority.is_none(), DEFAULT_PRIORITY_BIT),
            (self.first_object, FIRST_OBJECT_BIT),
        ] {
            if set {
                ty |= bit;
            }
        }
        varint::encode(ty, out);
        varint::encode(self.track_alias, out);
        varint::encode(self.group_id, out);
        if let SubgroupId::Explicit(subgroup_id) = self.subgroup_id {
            varint::encode(subgroup_id, out);
        }
        if let Some(priority) = self.publisher_priority {
            out.push(priority);
        }
    }
}

/// Object Status (§11.2.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectStatus {
    /// 0x0: an Object with a payload, possibly an empty one.
    Normal,
    /// 0x3: no Object with this or a larger Object ID exists in the Group.
    EndOfGroup,
    /// 0x4: no Object at this or a larger Location exists in the track.
    EndOfTrack,
}

impl ObjectStatus {
    fn code(self) -> u64 {
        match self {
            Self::Normal => 0x0,
            Self::EndOfGroup => 0x3,
            Self::EndOfTrack => 0x4,
        }
    }

    fn from_code(code: u64) -> Option<Self> {
        [Self::Normal, Self::EndOfGroup, Self::EndOfTrack]
            .into_iter()
            .find(|status| status.code() == code)
    }
}

/// One Object as it appears on a subgroup stream. The Group and Subgroup come
/// from the stream's header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgroupObject {
    /// Object ID within the Group.
    pub object_id: u64,
    /// Object Properties (§11.2.1.2); only a Normal Object may have any.
    pub properties: Vec<KeyValuePair>,
    /// Object Status.
    pub status: ObjectStatus,
    /// Object Payload; empty unless `status` is Normal.
    pub payload: Vec<u8>,
}

/// What a subgroup stream yields, in order: its header once, then its Objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubgroupEvent {
    /// The Subgroup Header.
    Header(SubgroupHeader),
    /// The next Object.
    Object(SubgroupObject),
}

/// What makes the Properties of one Object break the track, as far as the Object
/// alone shows it. Both the decoder and the encoder apply it, so the encoder
/// cannot produce a stream the decoder rejects.
pub(crate) fn object_properties_fault(
    properties: &[KeyValuePair],
    group_id: u64,
    object_id: u64,
) -> Option<&'static str> {
    // §2.5.1: a Mandatory Track Property on an Object is malformed.
    if properties
        .iter()
        .any(KeyValuePair::is_mandatory_track_property)
    {
        return Some("mandatory track property on an object");
    }
    // §12.8 and §12.9: at most one instance, and a gap cannot reach below 0.
    for (ty, id, repeated, too_large) in [
        (
            PRIOR_GROUP_ID_GAP,
            group_id,
            "more than one prior group id gap",
            "prior group id gap larger than the group id",
        ),
        (
            PRIOR_OBJECT_ID_GAP,
            object_id,
            "more than one prior object id gap",
            "prior object id gap larger than the object id",
        ),
    ] {
        let mut gaps = properties.iter().filter(|property| property.ty() == ty);
        let Some(gap) = gaps.next() else {
            continue;
        };
        if gaps.next().is_some() {
            return Some(repeated);
        }
        if matches!(gap.value(), Value::Varint(gap) if *gap > id) {
            return Some(too_large);
        }
    }
    None
}

/// Decodes one subgroup stream from the chunks it arrives in.
#[derive(Debug)]
pub struct SubgroupStreamDecoder {
    buffer: ReceiveBuffer,
    buffer_limit: usize,
    header: Option<SubgroupHeader>,
    previous_object_id: Option<u64>,
    /// An End of Group or End of Track Object was decoded.
    ended: bool,
}

impl SubgroupStreamDecoder {
    /// `buffer_limit` caps the bytes held at once. A whole Object has to fit
    /// under it together with the chunk that completes it, so it also bounds the
    /// Object size this decoder accepts.
    #[must_use]
    pub fn new(buffer_limit: usize) -> Self {
        Self {
            buffer: ReceiveBuffer::new(buffer_limit),
            buffer_limit,
            header: None,
            previous_object_id: None,
            ended: false,
        }
    }

    /// Bytes pushed and not yet returned as events: what this decoder holds in
    /// memory. A caller that reads many streams can add these up to bound them
    /// together.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.unread().len()
    }

    /// The Subgroup Header, once [`Self::next_event`] has returned it.
    #[must_use]
    pub fn header(&self) -> Option<&SubgroupHeader> {
        self.header.as_ref()
    }

    /// Adds bytes read from the stream.
    ///
    /// # Errors
    ///
    /// [`Error::ObjectTooLarge`] when the bytes not yet decoded would exceed the
    /// limit given to [`Self::new`], so drain with [`Self::next_event`] after
    /// every push. The error ends this stream only: stop reading it and leave
    /// the session open.
    pub fn push(&mut self, data: &[u8]) -> Result<(), Error> {
        self.buffer
            .push(data)
            .map_err(|limit| Error::ObjectTooLarge { limit })
    }

    /// The header first, then one Object per call; `None` until more bytes are
    /// pushed.
    ///
    /// # Errors
    ///
    /// Errors that end this stream and leave the session open:
    /// [`Error::MalformedTrack`] for an Object after End of Group or End of
    /// Track, or with Properties that break the track, after which the
    /// subscription has to be cancelled (§2.4.2); and [`Error::ObjectTooLarge`]
    /// for an Object that announces more bytes than the limit. Anything else is
    /// a protocol error that closes the session.
    pub fn next_event(&mut self) -> Result<Option<SubgroupEvent>, Error> {
        if self.ended {
            // WHY on the first byte, without decoding the Object it begins:
            // nothing but a FIN may follow an End of Group or End of Track, and
            // the Object is thrown away either way, so there is no reason to
            // buffer it up to the limit first.
            //
            // §2.4.2 lists an Object beyond the final one of its Group or track
            // as a malformed track. §11.4.3 calls it "a protocol violation" in
            // passing; the list in §2.4.2 is taken as the rule.
            return if self.buffer.is_empty() {
                Ok(None)
            } else {
                Err(Error::MalformedTrack(
                    "object after end of group or end of track",
                ))
            };
        }
        let mut reader = Reader::new(self.buffer.unread());
        let parsed = match self.header {
            None => SubgroupHeader::decode(&mut reader).map(SubgroupEvent::Header),
            Some(header) => self
                .decode_object(&mut reader, &header)
                .map(SubgroupEvent::Object),
        };
        let event = match parsed {
            Ok(event) => event,
            Err(ReadError::Short) => return Ok(None),
            Err(ReadError::Invalid(error)) => return Err(error),
        };
        let used = reader.position();
        self.buffer.consume(used);
        match &event {
            SubgroupEvent::Header(header) => self.header = Some(*header),
            SubgroupEvent::Object(object) => {
                self.previous_object_id = Some(object.object_id);
                self.ended = object.status != ObjectStatus::Normal;
            }
        }
        Ok(Some(event))
    }

    /// Call when the stream ends with a FIN. §11.4 says a session SHOULD be
    /// closed with `PROTOCOL_VIOLATION` when a stream ends in the middle of a
    /// serialized Object; a stream that ends inside its header is treated the
    /// same way, which the draft does not spell out.
    ///
    /// # Errors
    ///
    /// [`Error::ProtocolViolation`] when undecoded bytes are left.
    pub fn finish(&self) -> Result<(), Error> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(Error::ProtocolViolation(
                "stream ended inside a serialized object",
            ))
        }
    }

    fn decode_object(
        &self,
        reader: &mut Reader<'_>,
        header: &SubgroupHeader,
    ) -> ReadResult<SubgroupObject> {
        let delta = reader.varint()?;
        // §11.4.2: the first Object ID is the delta itself; later ones add delta + 1.
        let object_id = match self.previous_object_id {
            None => delta,
            Some(previous) => previous
                .checked_add(delta)
                .and_then(|id| id.checked_add(1))
                .ok_or(Error::ProtocolViolation("object id exceeds 64 bits"))?,
        };

        let properties = if header.properties {
            let len = self.announced_length(reader)?;
            parse_exact(reader.bytes(len)?, kvp::decode_all).map_err(|error| match error {
                // A receive limit hit on a data stream stops that stream, not
                // the session, as an Object over the size limit does.
                Error::TooManyKeyValuePairs { limit } => Error::TooManyObjectProperties { limit },
                error => error,
            })?
        } else {
            Vec::new()
        };

        // WHY here, before the payload is read: Properties that break the track
        // do so whatever follows them, and the Object is thrown away either way,
        // so there is no reason to buffer its payload first. A stream that ends
        // inside that payload is then a malformed track too, not a stream cut
        // short.
        if let Some(fault) = object_properties_fault(&properties, header.group_id, object_id) {
            return Err(Error::MalformedTrack(fault).into());
        }

        let payload_len = self.announced_length(reader)?;
        // The Object Status field exists only when the payload is empty.
        let (status, payload) = if payload_len == 0 {
            let status = ObjectStatus::from_code(reader.varint()?)
                .ok_or(Error::ProtocolViolation("unknown object status"))?;
            (status, Vec::new())
        } else {
            (ObjectStatus::Normal, reader.bytes(payload_len)?.to_vec())
        };

        if status != ObjectStatus::Normal && !properties.is_empty() {
            return Err(
                Error::ProtocolViolation("properties on an object that is not Normal").into(),
            );
        }
        Ok(SubgroupObject {
            object_id,
            properties,
            status,
            payload,
        })
    }

    /// A length field of an Object. One over the buffer limit can never be
    /// satisfied, so it is refused as soon as it is read instead of after the
    /// buffer has filled up. A length at or just under the limit cannot be
    /// satisfied either, because the fields before it take room too; that case
    /// is left to `push`, which fails once the buffer is full.
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

/// Encodes one subgroup stream: the header, then Objects in ascending Object ID.
///
/// What the header promises about the Objects is the caller's to keep: that the
/// stream holds the largest Object of the Group when `END_OF_GROUP` is set, and the
/// subgroup's first Object when `FIRST_OBJECT` is set. With
/// [`SubgroupId::FirstObjectId`] the Subgroup ID is whatever Object ID is encoded
/// first, so there is nothing to check.
#[derive(Debug)]
pub struct SubgroupStreamEncoder {
    group_id: u64,
    has_properties: bool,
    previous_object_id: Option<u64>,
    ended: bool,
    /// Encoded bytes not yet taken by the caller.
    bytes: Vec<u8>,
}

impl SubgroupStreamEncoder {
    /// An encoder whose first bytes are the header.
    #[must_use]
    pub fn new(header: &SubgroupHeader) -> Self {
        let mut bytes = Vec::new();
        header.encode(&mut bytes);
        Self {
            group_id: header.group_id,
            has_properties: header.properties,
            previous_object_id: None,
            ended: false,
            bytes,
        }
    }

    /// Encodes the next Object.
    ///
    /// # Errors
    ///
    /// Fails, encoding nothing, when the Object ID does not increase, when the
    /// Object has properties the header did not announce or properties that
    /// break the track (§2.5.1, §12.8, §12.9), when an Object that is not Normal
    /// has a payload or properties, or when an End of Group or End of Track
    /// Object was already encoded.
    pub fn object(&mut self, object: &SubgroupObject) -> Result<(), UsageError> {
        if self.ended {
            return Err(UsageError("no object follows end of group or end of track"));
        }
        let delta = match self.previous_object_id {
            None => object.object_id,
            // The guard makes the difference at least 1.
            Some(previous) if object.object_id > previous => {
                object.object_id.saturating_sub(previous).saturating_sub(1)
            }
            Some(_) => return Err(UsageError("object ids must increase on a stream")),
        };
        if !self.has_properties && !object.properties.is_empty() {
            return Err(UsageError(
                "the subgroup header does not announce properties",
            ));
        }
        if object.status != ObjectStatus::Normal
            && (!object.payload.is_empty() || !object.properties.is_empty())
        {
            return Err(UsageError(
                "only a Normal object carries a payload or properties",
            ));
        }
        if let Some(fault) =
            object_properties_fault(&object.properties, self.group_id, object.object_id)
        {
            return Err(UsageError(fault));
        }

        varint::encode(delta, &mut self.bytes);
        if self.has_properties {
            let mut properties = Vec::new();
            kvp::encode_all(&object.properties, &mut properties);
            put_length_prefixed(&properties, &mut self.bytes);
        }
        put_length_prefixed(&object.payload, &mut self.bytes);
        if object.payload.is_empty() {
            varint::encode(object.status.code(), &mut self.bytes);
        }
        self.previous_object_id = Some(object.object_id);
        self.ended = object.status != ObjectStatus::Normal;
        Ok(())
    }

    /// Hands over what has been encoded since the last call, to be written to
    /// the stream.
    pub fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_util::fixture;

    const LIMIT: usize = 1_024;

    fn decode_stream(bytes: &[u8], chunk_len: usize) -> Result<Vec<SubgroupEvent>, Error> {
        let mut decoder = SubgroupStreamDecoder::new(LIMIT);
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

    fn encode_stream(header: &SubgroupHeader, objects: &[SubgroupObject]) -> Vec<u8> {
        let mut encoder = SubgroupStreamEncoder::new(header);
        for object in objects {
            encoder.object(object).unwrap();
        }
        encoder.take_bytes()
    }

    fn events(header: SubgroupHeader, objects: &[SubgroupObject]) -> Vec<SubgroupEvent> {
        std::iter::once(SubgroupEvent::Header(header))
            .chain(objects.iter().cloned().map(SubgroupEvent::Object))
            .collect()
    }

    /// The header most cases start from: Track Alias 0, Group 0, Subgroup ID 0
    /// by mode, priority 0, no flags.
    fn plain_header() -> SubgroupHeader {
        SubgroupHeader {
            track_alias: 0,
            group_id: 0,
            subgroup_id: SubgroupId::Zero,
            publisher_priority: Some(0),
            properties: false,
            end_of_group: false,
            first_object: false,
        }
    }

    fn normal(object_id: u64, payload: &[u8]) -> SubgroupObject {
        SubgroupObject {
            object_id,
            properties: vec![],
            status: ObjectStatus::Normal,
            payload: payload.to_vec(),
        }
    }

    fn end(object_id: u64, status: ObjectStatus) -> SubgroupObject {
        SubgroupObject {
            object_id,
            properties: vec![],
            status,
            payload: vec![],
        }
    }

    /// A Normal Object carrying Prior Object ID Gap (0x3E) = 1, as the moqtail
    /// test client sends when it skips every other Object ID.
    fn with_gap(object_id: u64, payload: &[u8]) -> SubgroupObject {
        SubgroupObject {
            properties: vec![KeyValuePair::varint(0x3e, 1).unwrap()],
            ..normal(object_id, payload)
        }
    }

    type Stream = (SubgroupHeader, Vec<SubgroupObject>);

    /// Streams written by hand from the draft.
    fn streams() -> Vec<(Vec<u8>, Stream)> {
        vec![
            // First example of §11.6.
            (
                vec![
                    0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x04, 0x61, 0x62, 0x63, 0x64, 0x00, 0x04,
                    0x65, 0x66, 0x67, 0x68,
                ],
                (
                    SubgroupHeader {
                        track_alias: 2,
                        subgroup_id: SubgroupId::Explicit(0),
                        ..plain_header()
                    },
                    vec![normal(0, b"abcd"), normal(1, b"efgh")],
                ),
            ),
            // Type 0x35 as in the second example of §11.6: properties, explicit
            // Subgroup ID, default priority. Object IDs 3 and 7, then End of Group
            // at 8.
            (
                vec![
                    0x35, 0x02, 0x05, 0x01, 0x03, 0x05, 0x04, 0x25, 0x05, 0x01, 0xab, 0x01, 0x61,
                    0x03, 0x00, 0x01, 0x62, 0x00, 0x00, 0x00, 0x03,
                ],
                (
                    SubgroupHeader {
                        track_alias: 2,
                        group_id: 5,
                        subgroup_id: SubgroupId::Explicit(1),
                        publisher_priority: None,
                        properties: true,
                        ..plain_header()
                    },
                    vec![
                        SubgroupObject {
                            properties: vec![
                                KeyValuePair::varint(4, 37).unwrap(),
                                KeyValuePair::bytes(9, vec![0xab]).unwrap(),
                            ],
                            ..normal(3, b"a")
                        },
                        normal(7, b"b"),
                        end(8, ObjectStatus::EndOfGroup),
                    ],
                ),
            ),
            // Mode 0b01, and a Normal object with an empty payload, which has to
            // spell out its status.
            (
                vec![0x52, 0x09, 0x01, 0x80, 0x04, 0x00, 0x00],
                (
                    SubgroupHeader {
                        track_alias: 9,
                        group_id: 1,
                        subgroup_id: SubgroupId::FirstObjectId,
                        publisher_priority: Some(128),
                        first_object: true,
                        ..plain_header()
                    },
                    vec![normal(4, b"")],
                ),
            ),
            // Group 7 with Prior Group ID Gap (0x3C) = 7 and, on Object 2, Prior
            // Object ID Gap (0x3E) = 2: each gap as large as its ID allows.
            (
                vec![
                    0x11, 0x00, 0x07, 0x00, 0x02, 0x04, 0x3c, 0x07, 0x02, 0x02, 0x01, 0x61,
                ],
                (
                    SubgroupHeader {
                        group_id: 7,
                        properties: true,
                        ..plain_header()
                    },
                    vec![SubgroupObject {
                        properties: vec![
                            KeyValuePair::varint(0x3c, 7).unwrap(),
                            KeyValuePair::varint(0x3e, 2).unwrap(),
                        ],
                        ..normal(2, b"a")
                    }],
                ),
            ),
        ]
    }

    #[test]
    fn decodes_streams_in_any_chunking() {
        for (bytes, (header, objects)) in streams() {
            for chunk_len in [1, 2, 5, bytes.len()] {
                assert_eq!(
                    decode_stream(&bytes, chunk_len),
                    Ok(events(header, &objects)),
                    "{bytes:02x?} in chunks of {chunk_len}"
                );
            }
        }
    }

    #[test]
    fn encodes_the_same_bytes() {
        for (bytes, (header, objects)) in streams() {
            assert_eq!(encode_stream(&header, &objects), bytes, "{objects:?}");
        }
    }

    /// Streams received from the moqtail relay 0.15.0, see `fixtures/moqt-18/README.md`.
    /// The fixture files this module reads.
    pub(crate) fn fixture_names() -> Vec<&'static str> {
        moqtail_streams()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    fn moqtail_streams() -> Vec<(&'static str, Stream)> {
        // Header type 0x5D with priority 128, as moqtail's test client sends it.
        let test_client_header = |group_id| SubgroupHeader {
            group_id,
            subgroup_id: SubgroupId::Explicit(1),
            publisher_priority: Some(128),
            properties: true,
            end_of_group: true,
            first_object: true,
            ..plain_header()
        };
        let payload = [0x4d, 0x3b, 0xc3, 0x24, 0x4c, 0x84, 0x57, 0x2d];
        vec![
            // Type 0x58: Subgroup ID 0 by mode, priority present, no properties.
            (
                "subgroup-stream-from-moqtail-ts",
                (
                    SubgroupHeader {
                        end_of_group: true,
                        first_object: true,
                        ..plain_header()
                    },
                    vec![normal(
                        0,
                        b"xmoq-phase0 2026-10-10T01:18:43.029Z f69e3bea2a578c6b",
                    )],
                ),
            ),
            // Type 0x5d: explicit Subgroup ID, priority present, properties
            // announced and empty.
            (
                "subgroup-stream-from-moqtail-rs",
                (
                    SubgroupHeader {
                        subgroup_id: SubgroupId::Explicit(0),
                        properties: true,
                        end_of_group: true,
                        first_object: true,
                        ..plain_header()
                    },
                    vec![normal(
                        0,
                        b"xmoq-phase0 2026-10-10T01:19:03.764Z bb30f5dd09c8981f",
                    )],
                ),
            ),
            // Three Objects with IDs 0, 2 and 4: a non-zero Object ID Delta, and
            // the Prior Object ID Gap property (0x3E) with value 1 on the last two.
            (
                "subgroup-stream-object-id-gaps-from-moqtail-rs",
                (
                    test_client_header(4),
                    vec![
                        normal(0, &payload),
                        with_gap(2, &payload),
                        with_gap(4, &payload),
                    ],
                ),
            ),
            // Two Object Properties the draft does not define: an odd type
            // (a length and bytes) and an even one (an integer of 8 bytes).
            (
                "subgroup-stream-object-properties-from-own-ts",
                (
                    SubgroupHeader {
                        subgroup_id: SubgroupId::Explicit(0),
                        properties: true,
                        end_of_group: true,
                        first_object: true,
                        ..plain_header()
                    },
                    vec![SubgroupObject {
                        properties: vec![
                            KeyValuePair::bytes(
                                0x0d,
                                vec![
                                    0x01, 0x42, 0xe0, 0x1f, 0xff, 0xe1, 0x00, 0x09, 0x67, 0x42,
                                    0xe0, 0x1f, 0x8c, 0x8d, 0x40, 0x50, 0x00, 0x00,
                                ],
                            )
                            .unwrap(),
                            KeyValuePair::varint(0x10, 1_760_140_800_123_456).unwrap(),
                        ],
                        ..normal(0, b"xmoq-phase0b 2026-10-10T15:06:02.523Z c5e55f133a0eddce")
                    }],
                ),
            ),
            // A stream the relay opened for a subscriber that joined inside
            // Group 3: its first Object has ID 4, and FIRST_OBJECT is set all the
            // same.
            (
                "subgroup-stream-joined-mid-group-from-moqtail-rs",
                (test_client_header(3), vec![with_gap(4, &payload)]),
            ),
        ]
    }

    #[test]
    fn decodes_streams_captured_from_moqtail() {
        for (name, (header, objects)) in moqtail_streams() {
            assert_eq!(
                decode_stream(&fixture(name), 7),
                Ok(events(header, &objects)),
                "{name}"
            );
        }
    }

    #[test]
    fn encodes_the_bytes_moqtail_sent() {
        for (name, (header, objects)) in moqtail_streams() {
            assert_eq!(encode_stream(&header, &objects), fixture(name), "{name}");
        }
    }

    #[test]
    fn the_decoder_exposes_the_header_it_read() {
        let mut decoder = SubgroupStreamDecoder::new(LIMIT);
        assert_eq!(decoder.header(), None);
        decoder.push(&[0x10, 0x07, 0x00, 0x00]).unwrap();
        assert_eq!(decoder.header(), None);
        decoder.next_event().unwrap();
        assert_eq!(
            decoder.header(),
            Some(&SubgroupHeader {
                track_alias: 7,
                ..plain_header()
            })
        );
    }

    #[test]
    fn classifies_unidirectional_streams() {
        type Classified = Result<Option<UniStreamType>, Error>;
        let cases: &[(&[u8], Classified)] = &[
            (&[0xaf, 0x00], Ok(Some(UniStreamType::Control))),
            (&[0x10], Ok(Some(UniStreamType::Subgroup))),
            (&[0x58, 0x00], Ok(Some(UniStreamType::Subgroup))),
            (&[0x7d], Ok(Some(UniStreamType::Subgroup))),
            (&[], Ok(None)),
            (&[0xaf], Ok(None)),
            (&[0x05], Err(Error::UnsupportedStreamType(0x05))),
            (
                &[0xf0, 0x13, 0x2b, 0x3e, 0x28],
                Ok(Some(UniStreamType::Padding)),
            ),
            (&[0x00], Err(Error::UnknownStreamType(0x00))),
            (&[0x20], Err(Error::UnknownStreamType(0x20))),
            // 0x90 has bit 4 set and is outside 0b0XX1XXXX.
            (&[0x80, 0x90], Err(Error::UnknownStreamType(0x90))),
        ];
        for (bytes, expected) in cases {
            assert_eq!(uni_stream_type(bytes), *expected, "{bytes:02x?}");
        }
    }

    #[test]
    fn rejects_malformed_streams() {
        let cases: &[(&[u8], Error)] = &[
            // Types with SUBGROUP_ID_MODE 0b11.
            (
                &[0x16, 0x00, 0x00, 0x00],
                Error::ProtocolViolation("reserved subgroup id mode"),
            ),
            (
                &[0x7f, 0x00, 0x00],
                Error::ProtocolViolation("reserved subgroup id mode"),
            ),
            (
                &[0x05, 0x00],
                Error::ProtocolViolation("not a subgroup header type"),
            ),
            // Object status 1 is not defined.
            (
                &[0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
                Error::ProtocolViolation("unknown object status"),
            ),
            // End of Group with a property.
            (
                &[0x11, 0x00, 0x00, 0x00, 0x00, 0x02, 0x04, 0x25, 0x00, 0x03],
                Error::ProtocolViolation("properties on an object that is not Normal"),
            ),
            // Properties Length 1 holding a type and no value.
            (
                &[0x11, 0x00, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x61],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // Object ID 2^64-1 followed by another object.
            (
                &[
                    0x10, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                    0x01, 0x61, 0x00, 0x01, 0x62,
                ],
                Error::ProtocolViolation("object id exceeds 64 bits"),
            ),
            // FIN after the payload length and before the payload.
            (
                &[0x10, 0x00, 0x00, 0x00, 0x00, 0x04, 0x61],
                Error::ProtocolViolation("stream ended inside a serialized object"),
            ),
            // FIN inside the header.
            (
                &[0x10, 0x00],
                Error::ProtocolViolation("stream ended inside a serialized object"),
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(decode_stream(bytes, 3), Err(error.clone()), "{bytes:02x?}");
        }
    }

    #[test]
    fn reports_a_malformed_track() {
        let cases: &[(&[u8], &str)] = &[
            // After End of Group, the first byte of another Object is enough;
            // after End of Track, a whole Object.
            (
                &[0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00],
                "object after end of group or end of track",
            ),
            (
                &[0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x03],
                "object after end of group or end of track",
            ),
            // Property type 0x4000 is a Mandatory Track Property.
            (
                &[
                    0x11, 0x00, 0x00, 0x00, 0x00, 0x04, 0xc0, 0x40, 0x00, 0x01, 0x01, 0x61,
                ],
                "mandatory track property on an object",
            ),
            // Prior Object ID Gap (0x3E) twice on one Object.
            (
                &[
                    0x11, 0x00, 0x00, 0x00, 0x05, 0x04, 0x3e, 0x01, 0x00, 0x01, 0x01, 0x61,
                ],
                "more than one prior object id gap",
            ),
            // Prior Object ID Gap 3 on Object 2.
            (
                &[0x11, 0x00, 0x00, 0x00, 0x02, 0x02, 0x3e, 0x03, 0x01, 0x61],
                "prior object id gap larger than the object id",
            ),
            // The same Object cut off after its Properties, and one whose
            // Payload Length (5) has arrived without the payload: the Properties
            // alone decide.
            (
                &[0x11, 0x00, 0x00, 0x00, 0x02, 0x02, 0x3e, 0x03],
                "prior object id gap larger than the object id",
            ),
            (
                &[0x11, 0x00, 0x00, 0x00, 0x02, 0x02, 0x3e, 0x03, 0x05],
                "prior object id gap larger than the object id",
            ),
            // Prior Group ID Gap (0x3C) twice on one Object.
            (
                &[
                    0x11, 0x00, 0x07, 0x00, 0x00, 0x04, 0x3c, 0x01, 0x00, 0x01, 0x01, 0x61,
                ],
                "more than one prior group id gap",
            ),
            // Prior Group ID Gap 8 in Group 7.
            (
                &[0x11, 0x00, 0x07, 0x00, 0x00, 0x02, 0x3c, 0x08, 0x01, 0x61],
                "prior group id gap larger than the group id",
            ),
        ];
        for (bytes, fault) in cases {
            assert_eq!(
                decode_stream(bytes, 3),
                Err(Error::MalformedTrack(fault)),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn bounds_what_it_buffers() {
        let mut decoder = SubgroupStreamDecoder::new(8);
        // Header, then an object announcing 8 bytes of payload, the limit.
        decoder.push(&[0x10, 0x00, 0x00, 0x00, 0x00, 0x08]).unwrap();
        assert!(matches!(
            decoder.next_event(),
            Ok(Some(SubgroupEvent::Header(_)))
        ));
        assert_eq!(decoder.next_event(), Ok(None));
        decoder.push(&[0x61; 6]).unwrap();
        assert_eq!(
            decoder.push(&[0x61]),
            Err(Error::ObjectTooLarge { limit: 8 })
        );
    }

    #[test]
    fn refuses_a_length_over_the_limit_as_soon_as_it_is_read() {
        let cases: &[&[u8]] = &[
            // Payload Length 9.
            &[0x10, 0x00, 0x00, 0x00, 0x00, 0x09],
            // Properties Length 9.
            &[0x11, 0x00, 0x00, 0x00, 0x00, 0x09],
        ];
        for bytes in cases {
            let mut decoder = SubgroupStreamDecoder::new(8);
            decoder.push(bytes).unwrap();
            assert!(matches!(
                decoder.next_event(),
                Ok(Some(SubgroupEvent::Header(_)))
            ));
            assert_eq!(
                decoder.next_event(),
                Err(Error::ObjectTooLarge { limit: 8 }),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn bounds_the_properties_of_an_object() {
        // Header announcing properties; Object 0 with a Properties Length of
        // 130 (0x80 0x82): 65 pairs of type 0 with the value 0.
        let mut bytes = vec![0x11, 0x00, 0x00, 0x00, 0x00, 0x80, 0x82];
        bytes.extend([0x00, 0x00].repeat(65));
        bytes.extend([0x01, 0x61]);
        assert_eq!(
            decode_stream(&bytes, 7),
            Err(Error::TooManyObjectProperties { limit: 64 })
        );
    }

    #[test]
    fn refuses_to_encode_an_invalid_stream() {
        let property = |ty, value| vec![KeyValuePair::varint(ty, value).unwrap()];
        let with = |object_id, properties| SubgroupObject {
            properties,
            ..normal(object_id, b"a")
        };
        let announcing_properties = || SubgroupHeader {
            properties: true,
            ..plain_header()
        };
        // (header, Objects encoded first, the Object refused, why)
        let cases = [
            (
                plain_header(),
                vec![normal(1, b"a")],
                normal(1, b"a"),
                "object ids must increase on a stream",
            ),
            (
                plain_header(),
                vec![],
                with(2, property(4, 37)),
                "the subgroup header does not announce properties",
            ),
            (
                plain_header(),
                vec![],
                SubgroupObject {
                    payload: b"a".to_vec(),
                    ..end(2, ObjectStatus::EndOfGroup)
                },
                "only a Normal object carries a payload or properties",
            ),
            (
                plain_header(),
                vec![end(0, ObjectStatus::EndOfGroup)],
                normal(1, b"a"),
                "no object follows end of group or end of track",
            ),
            (
                plain_header(),
                vec![end(0, ObjectStatus::EndOfTrack)],
                normal(1, b"a"),
                "no object follows end of group or end of track",
            ),
            (
                announcing_properties(),
                vec![],
                with(2, property(0x4000, 1)),
                "mandatory track property on an object",
            ),
            (
                announcing_properties(),
                vec![],
                with(2, property(0x3e, 3)),
                "prior object id gap larger than the object id",
            ),
        ];
        for (header, accepted, refused, why) in cases {
            let mut encoder = SubgroupStreamEncoder::new(&header);
            for object in &accepted {
                encoder.object(object).unwrap();
            }
            let before = encoder.take_bytes();
            assert!(!before.is_empty(), "{why}");
            assert_eq!(encoder.object(&refused), Err(UsageError(why)), "{why}");
            // A refused Object leaves nothing behind.
            assert_eq!(encoder.take_bytes(), Vec::<u8>::new(), "{why}");
        }
    }
}
