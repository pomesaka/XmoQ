//! Unidirectional stream types (§3.4), the Subgroup Header and the Objects that
//! follow it on a subgroup stream (§11.4.2).
//!
//! Of a datagram (§11.3) only its type is read ([`datagram_fault`]).
//! FIXME: decode `OBJECT_DATAGRAM`. Nothing here does, so its Object is lost.

use crate::coding::{
    ByteReader, ReadError, ReadResult, ReceiveBuffer, parse_exact, put_length_prefixed,
};
use crate::immutable;
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
    /// A fetch stream: a `FETCH_HEADER`, then what [`crate::fetch`] decodes.
    Fetch,
}

/// Classifies a unidirectional stream from its first bytes, or returns `None`
/// until the type has arrived. The type is not consumed: the SETUP message type
/// and the Subgroup Header type double as the stream type.
///
/// # Errors
///
/// [`Error::UnknownStreamType`] for a type the draft does not define.
pub fn uni_stream_type(buf: &[u8]) -> Result<Option<UniStreamType>, Error> {
    let Some((ty, _)) = varint::decode(buf) else {
        return Ok(None);
    };
    match ty {
        STREAM_TYPE_SETUP => Ok(Some(UniStreamType::Control)),
        STREAM_TYPE_PADDING => Ok(Some(UniStreamType::Padding)),
        ty if is_subgroup_header_type(ty) => Ok(Some(UniStreamType::Subgroup)),
        STREAM_TYPE_FETCH_HEADER => Ok(Some(UniStreamType::Fetch)),
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

/// How a data stream of the peer ended: a subgroup stream, or for
/// `xmoq-moqt-io` also a fetch stream. Reading the stream is the caller's
/// I/O; the type lives here so that the I/O crate and the relay's state
/// machine share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// With a FIN. On a subgroup stream, every Object of the Subgroup was
    /// delivered (§11.4.3: "If a sender has delivered all objects in a
    /// Subgroup to the QUIC stream ... it MUST close the stream with a FIN").
    Fin,
    /// Without one. `Some` is the Stream Reset Error Code the peer reset the
    /// stream with (§3.3.3). `None` is a reset whose code could not be read,
    /// or a stream that this endpoint stopped itself (its I/O side: on
    /// request, on an idle timeout, or on a malformed stream).
    Aborted(Option<u32>),
    /// Cut off by the end of the peer's connection. The peer said nothing
    /// about the stream itself.
    Lost,
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
    /// `None` sets `DEFAULT_PRIORITY`: the field is omitted and "this Subgroup
    /// inherits the Publisher Priority specified in the control message that
    /// established the subscription" (§11.4.2), or
    /// [`crate::message::track_property::DEFAULT_PUBLISHER_PRIORITY_IF_OMITTED`]
    /// where that message has none (§12.4).
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
    fn decode(reader: &mut ByteReader<'_>) -> ReadResult<Self> {
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
    /// The fields of the next Object before its payload were read, and its
    /// payload is not whole yet. Only where asked for
    /// ([`SubgroupStreamDecoder::report_begun`]), at most once for an Object,
    /// and always before [`Self::Object`] of the same Object, which follows
    /// once the payload is whole (and does not, when the stream ends first).
    ///
    /// WHY: §5.1.2 has the Largest Object update "when the first byte of an
    /// Object with a Location larger than the previous value is ... received",
    /// which a relay cannot follow from whole Objects alone.
    ObjectBegun {
        /// Object ID within the Group.
        object_id: u64,
        /// Its Object Payload Length, more than 0.
        payload_len: usize,
    },
    /// The next Object.
    Object(SubgroupObject),
}

/// What makes the Properties of one Object break the track, as far as the Object
/// alone shows it: [`Error::MalformedTrack`]. Both the decoder and the encoder
/// apply it, so the encoder cannot produce a stream the decoder rejects. What
/// is checked is looked for inside Immutable Properties as well (§12.7), so a
/// Prior Object ID Gap outside and one inside are two instances.
///
/// # Errors
///
/// [`Error::MalformedTrack`], and what reading Immutable Properties fails with
/// otherwise ([`crate::immutable`]): a `PROTOCOL_VIOLATION`, and
/// [`Error::TooManyObjectProperties`] for a sequence inside over the local
/// limit, which ends only the stream, as one outside does.
pub(crate) fn object_properties_fault(
    object_properties: &[KeyValuePair],
    group_id: u64,
    object_id: u64,
) -> Result<(), Error> {
    let properties = match immutable::properties_to_search(object_properties, immutable::Of::Object)
    {
        Ok(Ok(properties)) => properties,
        Ok(Err(why)) => return Err(Error::MalformedTrack(why)),
        Err(Error::TooManyKeyValuePairs { limit }) => {
            return Err(Error::TooManyObjectProperties { limit });
        }
        Err(error) => return Err(error),
    };
    // §2.5.1: a Mandatory Track Property on an Object is malformed.
    if properties
        .iter()
        .any(KeyValuePair::is_mandatory_track_property)
    {
        return Err(Error::MalformedTrack(
            "mandatory track property on an object",
        ));
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
            return Err(Error::MalformedTrack(repeated));
        }
        if matches!(gap.value(), Value::Varint(gap) if *gap > id) {
            return Err(Error::MalformedTrack(too_large));
        }
    }
    Ok(())
}

/// What the encoder of an Object refuses: what [`object_properties_fault`]
/// finds, as a [`UsageError`].
pub(crate) fn object_properties_usage_fault(
    properties: &[KeyValuePair],
    group_id: u64,
    object_id: u64,
) -> Result<(), UsageError> {
    object_properties_fault(properties, group_id, object_id).map_err(|error| {
        UsageError(match error {
            Error::MalformedTrack(why) | Error::ProtocolViolation(why) => why,
            _ => "more key-value pairs inside immutable properties than a receiver takes",
        })
    })
}

/// What is wrong with a datagram the peer sent, by its type alone: `None`
/// for an `OBJECT_DATAGRAM` (§11.3.1) and a padding datagram (§11.5.2), and
/// a protocol error that closes the session for any other.
///
/// A padding datagram is let go of whatever follows its type: §11.5.2 has
/// "The receiver MUST discard all data received in a padding datagram", and
/// "MUST all be set to zero" is the sender's to keep.
///
/// §11: "An endpoint that receives an unknown datagram type MUST close the
/// session." §11.3.1 names the code for the two kinds there are: "Type values
/// with both the STATUS bit (0x20) and `END_OF_GROUP` bit (0x02) set" and "Type
/// values that do not match the form 0b00X0XXXX", which "MUST close the
/// session with a `PROTOCOL_VIOLATION`".
///
/// WHY a datagram whose type cannot be read is one too: it is no
/// `OBJECT_DATAGRAM`, which "All MOQT datagrams start with" a type says it
/// would have to be (§11).
#[must_use]
pub fn datagram_fault(datagram: &[u8]) -> Option<Error> {
    const STATUS: u64 = 0x20;
    const END_OF_GROUP: u64 = 0x02;
    const PADDING: u64 = 0x132B_3E29;
    let Some((ty, _)) = varint::decode(datagram) else {
        return Some(Error::ProtocolViolation("a datagram without a type"));
    };
    if ty == PADDING {
        return None;
    }
    // 0b00X0XXXX: no bit but 0x20 and the four low ones.
    let of_the_form = ty & !(STATUS | 0x0f) == 0;
    if !of_the_form || ty & (STATUS | END_OF_GROUP) == STATUS | END_OF_GROUP {
        return Some(Error::ProtocolViolation("an unknown datagram type"));
    }
    None
}

/// Decodes one subgroup stream from the chunks it arrives in.
#[derive(Debug)]
pub struct SubgroupStreamDecoder {
    buffer: ReceiveBuffer,
    buffer_limit: usize,
    /// The subscription's limit on an Object Payload Length, where its
    /// subscriber gave one ([`Self::limit_payload`]).
    payload_limit: Option<usize>,
    header: Option<SubgroupHeader>,
    previous_object_id: Option<u64>,
    /// An End of Group or End of Track Object was decoded.
    ended: bool,
    /// [`SubgroupEvent::ObjectBegun`] is yielded ([`Self::report_begun`]).
    reports_begun: bool,
    /// [`SubgroupEvent::ObjectBegun`] was yielded for the Object being read.
    begun: bool,
    /// The fields before the payload of the Object being read, once they are
    /// whole and checked. WHY kept: a peer that sends the payload a byte at a
    /// time would otherwise have its Properties decoded and checked once a
    /// byte.
    head: Option<ObjectHead>,
    /// How many times the fields before a payload were decoded.
    #[cfg(test)]
    heads_decoded: usize,
}

/// The fields of an Object before its payload, as
/// [`SubgroupStreamDecoder`] keeps them until the payload is whole.
#[derive(Debug)]
struct ObjectHead {
    object_id: u64,
    properties: Vec<KeyValuePair>,
    payload_len: usize,
    /// The bytes these fields take on the stream.
    len: usize,
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
            payload_limit: None,
            header: None,
            previous_object_id: None,
            ended: false,
            reports_begun: false,
            begun: false,
            head: None,
            #[cfg(test)]
            heads_decoded: 0,
        }
    }

    /// Yields [`SubgroupEvent::ObjectBegun`] from now on.
    ///
    /// WHY asked for, and not yielded to every caller: only a relay counts
    /// what it has begun to receive (§5.1.2, §10.2.11), and a receiver that
    /// plays whole Objects has nothing to do with it.
    pub fn report_begun(&mut self) {
        self.reports_begun = true;
    }

    /// Refuses, with [`Error::ObjectTooLarge`], each Object whose Object
    /// Payload Length is over `limit` and is read from now on: the limit of
    /// the subscription the stream turned out to be of. An Object whose
    /// length was read before is held to the limit of [`Self::new`] alone.
    ///
    /// WHY at the length, before the payload: the limit is there so that a
    /// payload is not collected, which only a check where its length is read
    /// keeps from happening.
    pub fn limit_payload(&mut self, limit: usize) {
        self.payload_limit = Some(limit);
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
        let parsed = match self.header {
            None => {
                let mut reader = ByteReader::new(self.buffer.unread());
                SubgroupHeader::decode(&mut reader)
                    .map(|header| (SubgroupEvent::Header(header), reader.position()))
            }
            Some(header) => self
                .decode_object(&header)
                .map(|(object, used)| (SubgroupEvent::Object(object), used)),
        };
        let (event, used) = match parsed {
            Ok(parsed) => parsed,
            Err(ReadError::Short) => return Ok(self.begun_object()),
            Err(ReadError::Invalid(error)) => return Err(error),
        };
        self.buffer.consume(used);
        match &event {
            SubgroupEvent::Header(header) => self.header = Some(*header),
            SubgroupEvent::Object(object) => {
                self.previous_object_id = Some(object.object_id);
                self.ended = object.status != ObjectStatus::Normal;
                self.begun = false;
            }
            SubgroupEvent::ObjectBegun { .. } => {}
        }
        Ok(Some(event))
    }

    /// [`SubgroupEvent::ObjectBegun`] for the Object the buffer holds the
    /// start of, where it is asked for and was not yielded yet, and the fields
    /// before the payload are whole. Nothing is consumed: the Object is
    /// consumed whole once its payload is.
    fn begun_object(&mut self) -> Option<SubgroupEvent> {
        if !self.reports_begun || self.begun {
            return None;
        }
        let head = self.head.as_ref()?;
        // An Object without a payload has its status after the length, a
        // field or two that are whole an instant later; it is not begun.
        if head.payload_len == 0 {
            return None;
        }
        self.begun = true;
        Some(SubgroupEvent::ObjectBegun {
            object_id: head.object_id,
            payload_len: head.payload_len,
        })
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

    /// The next Object and the bytes it takes, where it is whole. The fields
    /// before its payload are decoded once ([`Self::head`]).
    fn decode_object(&mut self, header: &SubgroupHeader) -> ReadResult<(SubgroupObject, usize)> {
        let head = if let Some(head) = self.head.take() {
            head
        } else {
            let mut reader = ByteReader::new(self.buffer.unread());
            let (object_id, properties, payload_len) =
                self.decode_object_head(&mut reader, header)?;
            #[cfg(test)]
            {
                self.heads_decoded += 1;
            }
            ObjectHead {
                object_id,
                properties,
                payload_len,
                len: reader.position(),
            }
        };
        let unread = self.buffer.unread();
        let mut reader = ByteReader::new(unread.get(head.len..).unwrap_or_default());
        // The Object Status field exists only when the payload is empty.
        let rest = if head.payload_len == 0 {
            reader.varint().and_then(|code| {
                let status = ObjectStatus::from_code(code)
                    .ok_or(Error::ProtocolViolation("unknown object status"))?;
                Ok((status, Vec::new()))
            })
        } else {
            reader
                .bytes(head.payload_len)
                .map(|payload| (ObjectStatus::Normal, payload.to_vec()))
        };
        let used = head.len + reader.position();
        let (status, payload) = match rest {
            Ok(rest) => rest,
            Err(ReadError::Short) => {
                self.head = Some(head);
                return Err(ReadError::Short);
            }
            Err(error) => return Err(error),
        };

        if status != ObjectStatus::Normal && !head.properties.is_empty() {
            return Err(
                Error::ProtocolViolation("properties on an object that is not Normal").into(),
            );
        }
        Ok((
            SubgroupObject {
                object_id: head.object_id,
                properties: head.properties,
                status,
                payload,
            },
            used,
        ))
    }

    /// The fields of an Object before its payload: its Object ID, its
    /// Properties and its Object Payload Length.
    fn decode_object_head(
        &self,
        reader: &mut ByteReader<'_>,
        header: &SubgroupHeader,
    ) -> ReadResult<(u64, Vec<KeyValuePair>, usize)> {
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
        object_properties_fault(&properties, header.group_id, object_id)?;

        let payload_len = self.announced_length(reader)?;
        if let Some(limit) = self.payload_limit
            && payload_len > limit
        {
            return Err(Error::ObjectTooLarge { limit }.into());
        }
        Ok((object_id, properties, payload_len))
    }

    /// A length field of an Object. One over the buffer limit can never be
    /// satisfied, so it is refused as soon as it is read instead of after the
    /// buffer has filled up. A length at or just under the limit cannot be
    /// satisfied either, because the fields before it take room too; that case
    /// is left to `push`, which fails once the buffer is full.
    fn announced_length(&self, reader: &mut ByteReader<'_>) -> ReadResult<usize> {
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
    /// break the track (§2.5.1, §12.7, §12.8, §12.9), when an Object that is not Normal
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
        object_properties_usage_fault(&object.properties, self.group_id, object.object_id)?;

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
    use crate::error::ErrorScope;
    use crate::test_util::{fixture, fixture_of_loc};

    /// More than the largest payload of a fixture: a key frame of 21,533
    /// bytes.
    const LIMIT: usize = 32 * 1_024;

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
        captured_streams()
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

    /// The start of a video stream of `moqlivemock`'s publisher
    /// (`fixtures/loc-04/README.md`). Type 0x15: Object Properties, an
    /// explicit Subgroup ID, a priority; neither the end of the Group nor
    /// the first Object is marked. Each Object has the Timestamp of LOC
    /// (0x10), in microseconds; the Group ID is the same time in seconds.
    fn moqlivemock_stream() -> (&'static str, Stream) {
        let at = |object_id, microseconds, payload: &str| SubgroupObject {
            properties: vec![KeyValuePair::varint(0x10, microseconds).unwrap()],
            ..normal(object_id, &fixture_of_loc(payload))
        };
        (
            "subgroup-stream-two-objects-from-moqlivemock",
            (
                SubgroupHeader {
                    group_id: 1_791_690_527,
                    subgroup_id: SubgroupId::Explicit(0),
                    publisher_priority: Some(128),
                    properties: true,
                    ..plain_header()
                },
                vec![
                    at(
                        0,
                        1_791_690_527_000_000,
                        "video-key-length-prefixed-from-moqlivemock",
                    ),
                    at(
                        1,
                        1_791_690_527_040_000,
                        "video-delta-length-prefixed-from-moqlivemock",
                    ),
                ],
            ),
        )
    }

    /// The streams another implementation wrote: moqtail's, and one of
    /// `moqlivemock`.
    fn captured_streams() -> Vec<(&'static str, Stream)> {
        let mut streams = moqtail_streams();
        streams.push(moqlivemock_stream());
        streams
    }

    #[test]
    fn decodes_streams_captured_from_another_implementation() {
        for (name, (header, objects)) in captured_streams() {
            assert_eq!(
                decode_stream(&fixture(name), 7),
                Ok(events(header, &objects)),
                "{name}"
            );
        }
    }

    #[test]
    fn encodes_the_bytes_another_implementation_sent() {
        for (name, (header, objects)) in captured_streams() {
            assert_eq!(encode_stream(&header, &objects), fixture(name), "{name}");
        }
    }

    #[test]
    fn yields_an_object_begun_once_before_an_object_whose_payload_is_not_whole() {
        let moqlivemock = fixture("subgroup-stream-two-objects-from-moqlivemock");
        // Header (Track Alias 0, Group 0), then Object 0 with End of Group
        // (Payload Length 0, status 0x3): no payload to wait for.
        let end_of_group: &[u8] = &[0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03];
        // (the stream, the bytes pushed at once, whether asked for, the events)
        let cases: [(&[u8], usize, bool, &[&str]); 5] = [
            (
                &moqlivemock,
                1,
                true,
                &[
                    "header",
                    "begun 0 of 21533",
                    "object 0",
                    // The sizes of fixtures/loc-04/README.md.
                    "begun 1 of 931",
                    "object 1",
                ],
            ),
            (&moqlivemock, 1, false, &["header", "object 0", "object 1"]),
            // Each Object whole in one push: it is begun and whole at once.
            (
                &moqlivemock,
                moqlivemock.len(),
                true,
                &["header", "object 0", "object 1"],
            ),
            (end_of_group, 1, true, &["header", "object 0"]),
            (end_of_group, 1, false, &["header", "object 0"]),
        ];
        for (bytes, chunk_len, asked, expected) in cases {
            let mut decoder = SubgroupStreamDecoder::new(LIMIT);
            if asked {
                decoder.report_begun();
            }
            let mut seen = Vec::new();
            for chunk in bytes.chunks(chunk_len) {
                decoder.push(chunk).unwrap();
                while let Some(event) = decoder.next_event().unwrap() {
                    seen.push(match event {
                        SubgroupEvent::Header(_) => "header".to_string(),
                        SubgroupEvent::ObjectBegun {
                            object_id,
                            payload_len,
                        } => format!("begun {object_id} of {payload_len}"),
                        SubgroupEvent::Object(object) => format!("object {}", object.object_id),
                    });
                }
            }
            assert_eq!(seen, expected, "{chunk_len} {asked}");
        }
    }

    #[test]
    fn the_fields_before_a_payload_are_decoded_once_however_the_bytes_arrive() {
        let moqlivemock = fixture("subgroup-stream-two-objects-from-moqlivemock");
        // (the bytes pushed at once, whether ObjectBegun is asked for)
        let cases: [(usize, bool); 4] = [
            (1, false),
            (1, true),
            (7, false),
            (moqlivemock.len(), false),
        ];
        for (chunk_len, asked) in cases {
            let mut decoder = SubgroupStreamDecoder::new(LIMIT);
            if asked {
                decoder.report_begun();
            }
            let mut objects = 0;
            for chunk in moqlivemock.chunks(chunk_len) {
                decoder.push(chunk).unwrap();
                while let Some(event) = decoder.next_event().unwrap() {
                    if let SubgroupEvent::Object(object) = event {
                        assert!(!object.properties.is_empty(), "{chunk_len} {asked}");
                        objects += 1;
                    }
                }
            }
            assert_eq!(
                (objects, decoder.heads_decoded),
                (2, 2),
                "{chunk_len} {asked}"
            );
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
    fn a_unidirectional_stream_is_what_the_shared_table_says() {
        // fixtures/moqt-18-verdicts/streams.tsv, which `@xmoq/moqt` reads as
        // well: the stream types of Table 3 (§3.4); "An endpoint that
        // receives an unknown stream type MUST close the session" (§3.4), and
        // a padding stream is read and discarded (§11.5.1).
        let rows = crate::test_util::table_rows::<3>("moqt-18-verdicts/streams.tsv");
        // A table that lost rows would pass.
        assert_eq!(rows.len(), 11);
        for [name, hex, verdict] in rows {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect();
            let got = match uni_stream_type(&bytes) {
                Ok(Some(UniStreamType::Control)) => "control".to_string(),
                Ok(Some(UniStreamType::Subgroup)) => "subgroup".to_string(),
                Ok(Some(UniStreamType::Fetch)) => "fetch".to_string(),
                Ok(Some(UniStreamType::Padding)) => "padding".to_string(),
                Ok(None) => panic!("{name}: no type yet"),
                Err(error) => format!(
                    "closed:{:#x}",
                    error
                        .termination_code()
                        .expect("it closes the session")
                        .code()
                ),
            };
            assert_eq!(got, verdict, "{name}");
        }
    }

    #[test]
    fn a_unidirectional_stream_has_no_type_until_its_type_has_arrived() {
        for bytes in [&[][..], &[0xaf], &[0xf0, 0x13]] {
            assert_eq!(uni_stream_type(bytes), Ok(None), "{bytes:02x?}");
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

    /// The rows of `fixtures/moqt-18-verdicts/subgroup-stream.tsv`, which
    /// `@xmoq/moqt` reads as well: a subgroup stream, whole and a byte at a
    /// time, ended with a FIN, and whether it is read, closes the session,
    /// makes the track malformed, or ends that stream alone (a limit of this
    /// end). §11.4: a FIN inside a serialized Object, or inside the header, is
    /// a `PROTOCOL_VIOLATION`. §2.4.2 and §11.4.3: an Object after End of
    /// Group or End of Track makes the track malformed as soon as its first
    /// byte is there. §2.5.1, §12.8 and §12.9: a Mandatory Track Property on
    /// an Object, and a Prior Group ID Gap or Prior Object ID Gap that is
    /// larger than its ID or comes twice, make it malformed, decided when the
    /// properties are read, before the status that would close the session
    /// (an Object is judged field by field as it arrives). More than
    /// `kvp::MAX_PAIRS` properties are this end's limit.
    #[test]
    fn subgroup_streams_are_read_or_refused_as_the_shared_table_says() {
        let rows = crate::test_util::table_rows::<3>("moqt-18-verdicts/subgroup-stream.tsv");
        // A table that lost rows would pass.
        assert_eq!(rows.len(), 23);
        for [name, hex, verdict] in rows {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect();
            for chunk in [bytes.len().max(1), 1] {
                let got = match decode_stream(&bytes, chunk) {
                    Ok(_) => "ok".to_string(),
                    Err(error) => match error.scope() {
                        ErrorScope::Session(code) => format!("closed:{:#x}", code.code()),
                        ErrorScope::Subscription { .. } => "malformed".to_string(),
                        ErrorScope::Stream { .. } => "stopped".to_string(),
                    },
                };
                assert_eq!(got, verdict, "{name}, in pieces of {chunk}");
            }
        }
    }

    /// §11, §11.3.1: the types of a datagram.
    #[test]
    fn a_datagram_closes_the_session_or_not_as_the_shared_table_says() {
        // fixtures/moqt-18-verdicts/datagrams.tsv, which `@xmoq/moqt` reads as
        // well. §11: "An endpoint that receives an unknown datagram type MUST
        // close the session"; §11.3.1 for the types of the form 0b00X0XXXX and
        // STATUS with END_OF_GROUP; §11.5.2: "The receiver MUST discard all
        // data received in a padding datagram".
        let rows = crate::test_util::table_rows::<3>("moqt-18-verdicts/datagrams.tsv");
        // A table that lost rows would pass.
        assert_eq!(rows.len(), 33);
        for [name, hex, verdict] in rows {
            let datagram: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect();
            let got = match datagram_fault(&datagram) {
                None => "dropped".to_string(),
                Some(error) => format!(
                    "closed:{:#x}",
                    error
                        .termination_code()
                        .expect("it closes the session")
                        .code()
                ),
            };
            assert_eq!(got, verdict, "{name}");
        }
    }

    #[test]
    fn a_limit_of_the_subscription_refuses_an_object_from_its_length_on() {
        // A header (Track Alias 0, Group 0), then Objects whose Payload Length
        // is 4 or 5, each with that many bytes; the limit of the stream is far.
        let header = [0x10, 0x00, 0x00, 0x00];
        let object = |len: u8| {
            let mut bytes = vec![0x00, len];
            bytes.extend(std::iter::repeat_n(0x61, usize::from(len)));
            bytes
        };
        // (the limit set after the header, if any; the Payload Length of the
        //  Object; what the decoder says of it)
        let cases = [
            (None, 5, Ok(())),
            (Some(4), 4, Ok(())),
            (Some(4), 5, Err(Error::ObjectTooLarge { limit: 4 })),
            (Some(5), 5, Ok(())),
        ];
        for (limit, len, expected) in cases {
            let mut decoder = SubgroupStreamDecoder::new(1024);
            decoder.push(&header).unwrap();
            assert!(matches!(
                decoder.next_event(),
                Ok(Some(SubgroupEvent::Header(_)))
            ));
            if let Some(limit) = limit {
                decoder.limit_payload(limit);
            }
            // The length alone is enough to decide: the payload is not pushed.
            decoder.push(&object(len)[..2]).unwrap();
            let decided = match decoder.next_event() {
                Err(error) => Err(error),
                Ok(_) => Ok(()),
            };
            assert_eq!(decided, expected, "{limit:?} {len}");
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
            // Prior Group ID Gap (0x3C) twice, and one of 2^40, which is
            // larger than the Group ID of the header.
            (
                announcing_properties(),
                vec![],
                with(
                    2,
                    vec![
                        KeyValuePair::varint(0x3c, 1).unwrap(),
                        KeyValuePair::varint(0x3c, 1).unwrap(),
                    ],
                ),
                "more than one prior group id gap",
            ),
            (
                announcing_properties(),
                vec![],
                with(2, property(0x3c, 1 << 40)),
                "prior group id gap larger than the group id",
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
