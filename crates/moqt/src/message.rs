//! Control messages (§10) and their framing on control and request streams.

use crate::coding::{
    ReadError, ReadResult, Reader, ReceiveBuffer, parse_exact, put_length_prefixed,
};
use crate::kvp::{self, KeyValuePair};
use crate::name::{FullTrackName, Location, TrackNamespace};
use crate::params::{self, MessageKind, Parameter};
use crate::{Error, UsageError, varint};

/// Largest Message Payload: the Message Length field is 16 bits (§10).
pub const MAX_PAYLOAD_LEN: usize = 65_535;
/// Longest Reason Phrase Value (§1.4.4).
pub const MAX_REASON_PHRASE_LEN: usize = 1_024;

const SETUP: u64 = 0x2f00;
const REQUEST_UPDATE: u64 = 0x02;
const SUBSCRIBE: u64 = 0x03;
const SUBSCRIBE_OK: u64 = 0x04;
const REQUEST_ERROR: u64 = 0x05;
const PUBLISH_NAMESPACE: u64 = 0x06;
const REQUEST_OK: u64 = 0x07;
const PUBLISH_DONE: u64 = 0x0b;
const PUBLISH: u64 = 0x1d;

/// Types Table 5 defines and this crate does not decode. Receiving one ends the
/// session with `INTERNAL_ERROR`, although the peer did nothing wrong.
///
/// FIXME: decode GOAWAY and hand it to the caller. A relay may send it at any
/// time (§10.4) and it does not touch the state of any request, so ending the
/// session over it is the worst of these.
const UNSUPPORTED: &[u64] = &[
    NAMESPACE,
    NAMESPACE_DONE,
    PUBLISH_BLOCKED,
    GOAWAY,
    PUBLISH_OK,
];
/// Requests the draft defines and this crate does not implement. Only their
/// Request ID is read, so that the request can be answered with `REQUEST_ERROR`
/// `NOT_SUPPORTED` instead of ending the session.
const UNSUPPORTED_REQUESTS: &[u64] = &[TRACK_STATUS, SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS];
const NAMESPACE: u64 = 0x08;
const TRACK_STATUS: u64 = 0x0d;
const NAMESPACE_DONE: u64 = 0x0e;
const PUBLISH_BLOCKED: u64 = 0x0f;
const GOAWAY: u64 = 0x10;
const FETCH: u64 = 0x16;
const FETCH_OK: u64 = 0x18;
/// Table 5 lists `PUBLISH_OK` as 0x1E, while §10.5 says a PUBLISH is answered
/// with `REQUEST_OK` (0x7) and calls that message `PUBLISH_OK`. This crate follows
/// §10.5, as the moqtail relay 0.15.0 does, so 0x1E is left undecoded.
const PUBLISH_OK: u64 = 0x1e;
const SUBSCRIBE_NAMESPACE: u64 = 0x50;
const SUBSCRIBE_TRACKS: u64 = 0x51;

/// `REQUEST_ERROR` Error Codes (§15.10.2).
pub mod request_error_code {
    /// `INTERNAL_ERROR`: an implementation specific or generic error.
    pub const INTERNAL_ERROR: u64 = 0x0;
    /// `UNAUTHORIZED`: the sender is not authorized for the request.
    pub const UNAUTHORIZED: u64 = 0x1;
    /// `TIMEOUT`: "The subscription could not be completed before an
    /// implementation specific timeout. For example, a relay could not
    /// establish an upstream subscription within the timeout."
    pub const TIMEOUT: u64 = 0x2;
    /// `NOT_SUPPORTED`: the endpoint does not support the type of request.
    pub const NOT_SUPPORTED: u64 = 0x3;
    /// `MALFORMED_AUTH_TOKEN`: the token could not be parsed.
    pub const MALFORMED_AUTH_TOKEN: u64 = 0x4;
    /// `EXPIRED_AUTH_TOKEN`: the token has expired.
    pub const EXPIRED_AUTH_TOKEN: u64 = 0x5;
    /// `GOING_AWAY`: "The endpoint has received a GOAWAY and MAY reject new
    /// requests."
    pub const GOING_AWAY: u64 = 0x6;
    /// `EXCESSIVE_LOAD`: the responder cannot process the request now.
    pub const EXCESSIVE_LOAD: u64 = 0x9;
    /// `DOES_NOT_EXIST`: the track or namespace is not available at the publisher.
    pub const DOES_NOT_EXIST: u64 = 0x10;
    /// `INVALID_RANGE`: the requested range of a subscription or fetch is not
    /// valid.
    pub const INVALID_RANGE: u64 = 0x11;
    /// `MALFORMED_TRACK`: the track was found to be malformed (§2.4.2).
    pub const MALFORMED_TRACK: u64 = 0x12;
    /// `DUPLICATE_SUBSCRIPTION`: the request would create a second subscription
    /// to a track in the same role (§5.1).
    pub const DUPLICATE_SUBSCRIPTION: u64 = 0x19;
    /// `UNINTERESTED`: the subscriber is not interested in the track or
    /// namespace.
    pub const UNINTERESTED: u64 = 0x20;
    /// `INVALID_JOINING_REQUEST_ID`: the Joining Request ID of a Joining FETCH
    /// names no subscription of the session (§10.12.2).
    pub const INVALID_JOINING_REQUEST_ID: u64 = 0x32;
    /// `UNSUPPORTED_EXTENSION`: the track has a Mandatory Track Property the
    /// endpoint does not understand (§2.5.1).
    pub const UNSUPPORTED_EXTENSION: u64 = 0x33;
    /// REDIRECT: the one code that carries a Redirect.
    pub const REDIRECT: u64 = 0x34;
}

/// Setup Option types (§15.4).
pub mod setup_option {
    /// PATH: the path of the `MoQ` URI, for a client on native QUIC (§10.3.1.2).
    pub const PATH: u64 = 0x01;
    /// AUTHORIZATION TOKEN, the one option that may repeat (§10.3.1.4).
    pub const AUTHORIZATION_TOKEN: u64 = 0x03;
    /// `MAX_AUTH_TOKEN_CACHE_SIZE` (§10.3.1.3).
    pub const MAX_AUTH_TOKEN_CACHE_SIZE: u64 = 0x04;
    /// AUTHORITY: the authority of the `MoQ` URI, for a client on native QUIC
    /// (§10.3.1.1).
    pub const AUTHORITY: u64 = 0x05;
    /// `MOQT_IMPLEMENTATION`: name and version of the sender (§10.3.1.5).
    pub const MOQT_IMPLEMENTATION: u64 = 0x07;
}

/// `PUBLISH_DONE` Status Codes (§10.11).
pub mod publish_done_status {
    /// `INTERNAL_ERROR`: an implementation specific or generic error
    /// occurred.
    pub const INTERNAL_ERROR: u64 = 0x0;
    /// `TRACK_ENDED`: the track is no longer being published.
    pub const TRACK_ENDED: u64 = 0x2;
    /// `TOO_FAR_BEHIND`: the publisher's queue for this subscriber exceeded its
    /// limit.
    pub const TOO_FAR_BEHIND: u64 = 0x5;
    /// `MALFORMED_TRACK`: a relay publisher detected that the track was
    /// malformed (§2.4.2).
    pub const MALFORMED_TRACK: u64 = 0x12;
    /// `UPDATE_FAILED`: a `REQUEST_UPDATE` failed on this subscription (§10.9.1).
    pub const UPDATE_FAILED: u64 = 0x8;
}

/// Reason Phrase (§1.4.4): up to 1024 bytes, which the draft says are a UTF-8
/// string. Kept as bytes because it names no error for bytes that are not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReasonPhrase(Vec<u8>);

impl ReasonPhrase {
    /// A Reason Phrase with this value.
    ///
    /// # Errors
    ///
    /// Fails for more than [`MAX_REASON_PHRASE_LEN`] bytes.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, UsageError> {
        let value = value.into();
        if value.len() > MAX_REASON_PHRASE_LEN {
            return Err(UsageError("reason phrase longer than 1024 bytes"));
        }
        Ok(Self(value))
    }

    /// The Reason Phrase Value.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        let len = reader.length()?;
        if len > MAX_REASON_PHRASE_LEN {
            return Err(Error::ProtocolViolation("reason phrase longer than 1024 bytes").into());
        }
        Ok(Self(reader.bytes(len)?.to_vec()))
    }
}

/// SETUP (§10.3), the first message on each control stream.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Setup {
    /// Setup Options, typed by [`setup_option`]. Unknown ones are kept.
    pub setup_options: Vec<KeyValuePair>,
}

/// SUBSCRIBE (§10.7): asks the publisher for newly published Objects of a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    /// Request ID (§10.1).
    pub request_id: u64,
    /// The Track Namespace and Track Name fields. They are held as one value
    /// because the 4096-byte limit is on their sum (§2.4.1).
    pub full_track_name: FullTrackName,
    /// Message Parameters.
    pub parameters: Vec<Parameter>,
}

impl Subscribe {
    /// The Track Namespace field.
    #[must_use]
    pub fn track_namespace(&self) -> &TrackNamespace {
        self.full_track_name.namespace()
    }

    /// The Track Name field.
    #[must_use]
    pub fn track_name(&self) -> &[u8] {
        self.full_track_name.name()
    }
}

/// `REQUEST_UPDATE` (§10.9): asks to change the parameters of a request.
///
/// The only update this crate decodes is one to a subscription, so its
/// parameters are checked against what such an update may carry (§10.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUpdate {
    /// Request ID of the update itself; each one consumes an ID (§10.1).
    pub request_id: u64,
    /// Message Parameters to change.
    pub parameters: Vec<Parameter>,
}

/// `SUBSCRIBE_OK` (§10.8): the publisher accepted a SUBSCRIBE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeOk {
    /// The alias the track has on data streams of this session (§11.1).
    pub track_alias: u64,
    /// Message Parameters.
    pub parameters: Vec<Parameter>,
    /// Track Properties (§2.5).
    pub track_properties: Vec<KeyValuePair>,
}

/// PUBLISH (§10.10): the publisher opens a subscription for a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    /// Request ID (§10.1).
    pub request_id: u64,
    /// The Track Namespace and Track Name fields. They are held as one value
    /// because the 4096-byte limit is on their sum (§2.4.1).
    pub full_track_name: FullTrackName,
    /// The alias the publisher chose for the track on data streams (§11.1).
    pub track_alias: u64,
    /// Message Parameters.
    pub parameters: Vec<Parameter>,
    /// Track Properties (§2.5).
    pub track_properties: Vec<KeyValuePair>,
}

impl Publish {
    /// The Track Namespace field.
    #[must_use]
    pub fn track_namespace(&self) -> &TrackNamespace {
        self.full_track_name.namespace()
    }

    /// The Track Name field.
    #[must_use]
    pub fn track_name(&self) -> &[u8] {
        self.full_track_name.name()
    }
}

/// `PUBLISH_NAMESPACE` (§10.15): the publisher has tracks in a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishNamespace {
    /// Request ID (§10.1).
    pub request_id: u64,
    /// Track Namespace.
    pub track_namespace: TrackNamespace,
    /// Message Parameters.
    pub parameters: Vec<Parameter>,
}

/// `REQUEST_OK` (§10.5). Which parameters it may carry depends on the request it
/// answers, so they are checked by the session, not here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestOk {
    /// Message Parameters.
    pub parameters: Vec<Parameter>,
    /// Track Properties; empty unless the message answers `TRACK_STATUS`.
    pub track_properties: Vec<KeyValuePair>,
}

/// Redirect (§10.6.1): where to retry a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// Connect URI; empty means the URI of the current session.
    pub connect_uri: Vec<u8>,
    /// The Track Namespace and Track Name fields as sent. What they mean
    /// depends on whether both are empty; see [`Self::track`].
    pub full_track_name: FullTrackName,
}

impl Redirect {
    /// The track to ask for instead, or `None` for the track of the original
    /// request, which is what a Track Namespace without fields together with an
    /// empty Track Name stands for (§10.6.1).
    #[must_use]
    pub fn track(&self) -> Option<&FullTrackName> {
        let same = self.full_track_name.namespace().fields().is_empty()
            && self.full_track_name.name().is_empty();
        (!same).then_some(&self.full_track_name)
    }
}

/// `REQUEST_ERROR` (§10.6): a request was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestError {
    /// Error Code, see [`request_error_code`].
    pub error_code: u64,
    /// Milliseconds to wait before retrying, plus one; 0 means do not retry.
    pub retry_interval: u64,
    /// Error Reason.
    pub error_reason: ReasonPhrase,
    /// Present exactly when `error_code` is REDIRECT (§10.6.2).
    pub redirect: Option<Redirect>,
}

/// `PUBLISH_DONE` (§10.11): the publisher ended a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishDone {
    /// Status Code, see [`publish_done_status`].
    pub status_code: u64,
    /// Data streams the publisher opened for the subscription, 2^62 - 1 when it
    /// does not know.
    pub stream_count: u64,
    /// Error Reason.
    pub error_reason: ReasonPhrase,
}

/// Which Objects a FETCH asks for (§10.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchRange {
    /// Fetch Type 0x1: a range of a track, independent of any subscription
    /// (§10.12.1).
    Standalone {
        /// The track.
        full_track_name: FullTrackName,
        /// The first Location of the range.
        start: Location,
        /// "The end Location, plus 1. A Location.Object value of 0 means
        /// the entire group is requested."
        end: Location,
    },
    /// Fetch Type 0x2: up to the Joining Location of a subscription, from
    /// `joining_start` Groups before it (§10.12.2).
    RelativeJoining {
        /// The Request ID of the subscription, in the same session.
        joining_request_id: u64,
        /// How many Groups before the Joining Location's the range starts.
        joining_start: u64,
    },
    /// Fetch Type 0x3: up to the Joining Location of a subscription, from
    /// the Group `joining_start` (§10.12.2).
    AbsoluteJoining {
        /// The Request ID of the subscription, in the same session.
        joining_request_id: u64,
        /// The Group the range starts with.
        joining_start: u64,
    },
}

/// FETCH (§10.12.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    /// Request ID (§10.1).
    pub request_id: u64,
    /// The Fetch Type and what belongs to it.
    pub range: FetchRange,
    /// Message Parameters.
    pub parameters: Vec<Parameter>,
}

/// `FETCH_OK` (§10.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOk {
    /// Whether the End Location is the final Object of the track.
    pub end_of_track: bool,
    /// The end of the range the response covers, in the encoding of a
    /// FETCH: the last Object plus 1, or Object 0 for a whole Group.
    pub end_location: Location,
    /// Message Parameters. No parameter of the draft may appear here.
    pub parameters: Vec<Parameter>,
    /// Track Properties (§2.5).
    pub track_properties: Vec<KeyValuePair>,
}

impl RequestError {
    fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        let error_code = reader.varint()?;
        let retry_interval = reader.varint()?;
        let error_reason = ReasonPhrase::decode(reader)?;
        let redirect = if error_code == request_error_code::REDIRECT {
            let uri_len = reader.length()?;
            let connect_uri = reader.bytes(uri_len)?.to_vec();
            Some(Redirect {
                connect_uri,
                full_track_name: FullTrackName::decode(reader)?,
            })
        } else {
            None
        };
        Ok(Self {
            error_code,
            retry_interval,
            error_reason,
            redirect,
        })
    }
}

const FETCH_TYPE_STANDALONE: u64 = 0x1;
const FETCH_TYPE_RELATIVE_JOINING: u64 = 0x2;
const FETCH_TYPE_ABSOLUTE_JOINING: u64 = 0x3;

impl Fetch {
    fn encode_payload(&self, out: &mut Vec<u8>) -> Result<(), UsageError> {
        varint::encode(self.request_id, out);
        match &self.range {
            FetchRange::Standalone {
                full_track_name,
                start,
                end,
            } => {
                varint::encode(FETCH_TYPE_STANDALONE, out);
                full_track_name.encode(out);
                start.encode(out);
                end.encode(out);
            }
            FetchRange::RelativeJoining {
                joining_request_id,
                joining_start,
            } => {
                varint::encode(FETCH_TYPE_RELATIVE_JOINING, out);
                varint::encode(*joining_request_id, out);
                varint::encode(*joining_start, out);
            }
            FetchRange::AbsoluteJoining {
                joining_request_id,
                joining_start,
            } => {
                varint::encode(FETCH_TYPE_ABSOLUTE_JOINING, out);
                varint::encode(*joining_request_id, out);
                varint::encode(*joining_start, out);
            }
        }
        params::encode_all(&self.parameters, out)
    }

    fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        let request_id = reader.varint()?;
        let range = match reader.varint()? {
            FETCH_TYPE_STANDALONE => FetchRange::Standalone {
                full_track_name: FullTrackName::decode(reader)?,
                start: Location::decode(reader)?,
                end: Location::decode(reader)?,
            },
            FETCH_TYPE_RELATIVE_JOINING => FetchRange::RelativeJoining {
                joining_request_id: reader.varint()?,
                joining_start: reader.varint()?,
            },
            FETCH_TYPE_ABSOLUTE_JOINING => FetchRange::AbsoluteJoining {
                joining_request_id: reader.varint()?,
                joining_start: reader.varint()?,
            },
            // §10.12: "An endpoint that receives a Fetch Type other than 0x1,
            // 0x2 or 0x3 MUST close the session with a PROTOCOL_VIOLATION."
            _ => return Err(Error::ProtocolViolation("unknown fetch type").into()),
        };
        let parameters = params::decode_all(reader)?;
        check_parameters(&parameters, MessageKind::Fetch)?;
        Ok(Self {
            request_id,
            range,
            parameters,
        })
    }
}

impl FetchOk {
    fn encode_payload(&self, out: &mut Vec<u8>) -> Result<(), UsageError> {
        out.push(u8::from(self.end_of_track));
        self.end_location.encode(out);
        params::encode_all(&self.parameters, out)?;
        kvp::encode_all(&self.track_properties, out);
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        // §10.13 gives the field 8 bits and the values 1 and 0. WHY any other
        // value is a violation: nothing says what it would mean, and FORWARD,
        // the other field of that kind, is read so (§10.2.12).
        let end_of_track = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::ProtocolViolation("End Of Track is neither 0 nor 1").into()),
        };
        let end_location = Location::decode(reader)?;
        let parameters = params::decode_all(reader)?;
        check_parameters(&parameters, MessageKind::FetchOk)?;
        Ok(Self {
            end_of_track,
            end_location,
            parameters,
            track_properties: kvp::decode_all(reader)?,
        })
    }
}

/// One control message, of the types this crate implements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMessage {
    /// SETUP, type 0x2F00.
    Setup(Setup),
    /// `REQUEST_UPDATE`, type 0x2.
    RequestUpdate(RequestUpdate),
    /// SUBSCRIBE, type 0x3.
    Subscribe(Subscribe),
    /// `SUBSCRIBE_OK`, type 0x4.
    SubscribeOk(SubscribeOk),
    /// PUBLISH, type 0x1D.
    Publish(Publish),
    /// `PUBLISH_NAMESPACE`, type 0x6.
    PublishNamespace(PublishNamespace),
    /// `REQUEST_OK`, type 0x7.
    RequestOk(RequestOk),
    /// `REQUEST_ERROR`, type 0x5.
    RequestError(RequestError),
    /// `PUBLISH_DONE`, type 0xB.
    PublishDone(PublishDone),
    /// FETCH, type 0x16.
    Fetch(Fetch),
    /// `FETCH_OK`, type 0x18.
    FetchOk(FetchOk),
    /// A request this crate does not implement: `TRACK_STATUS` (0xD),
    /// `SUBSCRIBE_NAMESPACE` (0x50) or `SUBSCRIBE_TRACKS` (0x51). Only
    /// the Request ID, their first field, is decoded; the rest of the payload
    /// is skipped unchecked. It cannot be encoded.
    UnsupportedRequest {
        /// The Message Type.
        message_type: u64,
        /// Request ID (§10.1).
        request_id: u64,
    },
}

impl ControlMessage {
    /// Decodes one message from the start of `buf`.
    ///
    /// Returns the message and the number of bytes it used, or `None` when `buf`
    /// does not yet hold a whole message.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownMessageType`] or [`Error::UnsupportedMessageType`] for a
    /// type this crate does not decode, as soon as the type has arrived, and a
    /// protocol error for a payload that does not match its type or its length.
    pub fn decode(buf: &[u8]) -> Result<Option<(Self, usize)>, Error> {
        let mut reader = Reader::new(buf);
        let (ty, payload) = match split_frame(&mut reader) {
            Ok(frame) => frame,
            Err(ReadError::Short) => {
                // Refuse a type that can never be decoded before waiting for a
                // payload of up to 64 KiB that would only be thrown away.
                if let Some((ty, _)) = varint::decode(buf) {
                    check_type(ty)?;
                }
                return Ok(None);
            }
            Err(ReadError::Invalid(error)) => return Err(error),
        };
        check_type(ty)?;
        let message = parse_exact(payload, |reader| decode_payload(ty, reader))?;
        Ok(Some((message, reader.position())))
    }

    /// Appends the message with its type and length.
    ///
    /// # Errors
    ///
    /// Fails when the payload exceeds [`MAX_PAYLOAD_LEN`], when a parameter is
    /// repeated or holds a value that cannot be encoded, or when a Redirect and
    /// the error code REDIRECT do not go together.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), UsageError> {
        let mut payload = Vec::new();
        let ty = self.encode_payload(&mut payload)?;
        let len = u16::try_from(payload.len())
            .map_err(|_| UsageError("message payload longer than 65535 bytes"))?;
        varint::encode(ty, out);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&payload);
        Ok(())
    }

    fn encode_payload(&self, out: &mut Vec<u8>) -> Result<u64, UsageError> {
        Ok(match self {
            Self::Setup(message) => {
                kvp::encode_all(&message.setup_options, out);
                SETUP
            }
            Self::RequestUpdate(message) => {
                varint::encode(message.request_id, out);
                params::encode_all(&message.parameters, out)?;
                REQUEST_UPDATE
            }
            Self::Subscribe(message) => {
                varint::encode(message.request_id, out);
                message.full_track_name.encode(out);
                params::encode_all(&message.parameters, out)?;
                SUBSCRIBE
            }
            Self::SubscribeOk(message) => {
                varint::encode(message.track_alias, out);
                params::encode_all(&message.parameters, out)?;
                kvp::encode_all(&message.track_properties, out);
                SUBSCRIBE_OK
            }
            Self::Publish(message) => {
                varint::encode(message.request_id, out);
                message.full_track_name.encode(out);
                varint::encode(message.track_alias, out);
                params::encode_all(&message.parameters, out)?;
                kvp::encode_all(&message.track_properties, out);
                PUBLISH
            }
            Self::PublishNamespace(message) => {
                varint::encode(message.request_id, out);
                message.track_namespace.encode(out);
                params::encode_all(&message.parameters, out)?;
                PUBLISH_NAMESPACE
            }
            Self::RequestOk(message) => {
                params::encode_all(&message.parameters, out)?;
                kvp::encode_all(&message.track_properties, out);
                REQUEST_OK
            }
            Self::RequestError(message) => {
                let is_redirect = message.error_code == request_error_code::REDIRECT;
                if message.redirect.is_some() != is_redirect {
                    return Err(UsageError(
                        "a Redirect goes with error code REDIRECT and no other",
                    ));
                }
                varint::encode(message.error_code, out);
                varint::encode(message.retry_interval, out);
                put_length_prefixed(message.error_reason.as_bytes(), out);
                if let Some(redirect) = &message.redirect {
                    put_length_prefixed(&redirect.connect_uri, out);
                    redirect.full_track_name.encode(out);
                }
                REQUEST_ERROR
            }
            Self::UnsupportedRequest { .. } => {
                return Err(UsageError(
                    "this crate does not send requests it does not implement",
                ));
            }
            Self::PublishDone(message) => {
                varint::encode(message.status_code, out);
                varint::encode(message.stream_count, out);
                put_length_prefixed(message.error_reason.as_bytes(), out);
                PUBLISH_DONE
            }
            Self::Fetch(message) => {
                message.encode_payload(out)?;
                FETCH
            }
            Self::FetchOk(message) => {
                message.encode_payload(out)?;
                FETCH_OK
            }
        })
    }
}

/// Cuts `Message Type, Message Length, Message Payload` off the front (§10).
fn split_frame<'a>(reader: &mut Reader<'a>) -> ReadResult<(u64, &'a [u8])> {
    let ty = reader.varint()?;
    let len = usize::from(reader.u16()?);
    Ok((ty, reader.bytes(len)?))
}

fn check_type(ty: u64) -> Result<(), Error> {
    match ty {
        SETUP | REQUEST_UPDATE | SUBSCRIBE | SUBSCRIBE_OK | REQUEST_ERROR | PUBLISH_NAMESPACE
        | REQUEST_OK | PUBLISH_DONE | PUBLISH | FETCH | FETCH_OK => Ok(()),
        ty if UNSUPPORTED_REQUESTS.contains(&ty) => Ok(()),
        ty if UNSUPPORTED.contains(&ty) => Err(Error::UnsupportedMessageType(ty)),
        // Includes the types Table 5 marks RESERVED (0x01, 0x20, 0x21, 0x40,
        // 0x41): they are setup messages of older drafts, not messages of this one.
        ty => Err(Error::UnknownMessageType(ty)),
    }
}

/// §10.2.1: a parameter in a message that its definition does not list is a
/// `PROTOCOL_VIOLATION`.
fn check_parameters(parameters: &[Parameter], kind: MessageKind) -> Result<(), Error> {
    if params::all_allowed(parameters, kind) {
        Ok(())
    } else {
        Err(Error::ProtocolViolation(
            "parameter not allowed in this message",
        ))
    }
}

fn decode_payload(ty: u64, reader: &mut Reader<'_>) -> ReadResult<ControlMessage> {
    Ok(match ty {
        SETUP => ControlMessage::Setup(Setup {
            setup_options: kvp::decode_all(reader)?,
        }),
        REQUEST_UPDATE => {
            let request_id = reader.varint()?;
            let parameters = params::decode_all(reader)?;
            check_parameters(&parameters, MessageKind::RequestUpdate)?;
            ControlMessage::RequestUpdate(RequestUpdate {
                request_id,
                parameters,
            })
        }
        SUBSCRIBE => {
            let request_id = reader.varint()?;
            let full_track_name = FullTrackName::decode(reader)?;
            let parameters = params::decode_all(reader)?;
            check_parameters(&parameters, MessageKind::Subscribe)?;
            ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name,
                parameters,
            })
        }
        SUBSCRIBE_OK => {
            let track_alias = reader.varint()?;
            let parameters = params::decode_all(reader)?;
            check_parameters(&parameters, MessageKind::SubscribeOk)?;
            ControlMessage::SubscribeOk(SubscribeOk {
                track_alias,
                parameters,
                track_properties: kvp::decode_all(reader)?,
            })
        }
        PUBLISH => {
            let request_id = reader.varint()?;
            let full_track_name = FullTrackName::decode(reader)?;
            let track_alias = reader.varint()?;
            let parameters = params::decode_all(reader)?;
            check_parameters(&parameters, MessageKind::Publish)?;
            ControlMessage::Publish(Publish {
                request_id,
                full_track_name,
                track_alias,
                parameters,
                track_properties: kvp::decode_all(reader)?,
            })
        }
        PUBLISH_NAMESPACE => {
            let request_id = reader.varint()?;
            let track_namespace = TrackNamespace::decode(reader)?;
            let parameters = params::decode_all(reader)?;
            check_parameters(&parameters, MessageKind::PublishNamespace)?;
            ControlMessage::PublishNamespace(PublishNamespace {
                request_id,
                track_namespace,
                parameters,
            })
        }
        REQUEST_OK => ControlMessage::RequestOk(RequestOk {
            parameters: params::decode_all(reader)?,
            track_properties: kvp::decode_all(reader)?,
        }),
        REQUEST_ERROR => ControlMessage::RequestError(RequestError::decode(reader)?),
        PUBLISH_DONE => ControlMessage::PublishDone(PublishDone {
            status_code: reader.varint()?,
            stream_count: reader.varint()?,
            error_reason: ReasonPhrase::decode(reader)?,
        }),
        FETCH => ControlMessage::Fetch(Fetch::decode(reader)?),
        FETCH_OK => ControlMessage::FetchOk(FetchOk::decode(reader)?),
        ty if UNSUPPORTED_REQUESTS.contains(&ty) => {
            let request_id = reader.varint()?;
            reader.skip_rest();
            ControlMessage::UnsupportedRequest {
                message_type: ty,
                request_id,
            }
        }
        // `check_type` runs first and lets only the types above through.
        ty => return Err(Error::UnknownMessageType(ty).into()),
    })
}

/// Reassembles control messages from the chunks a stream delivers.
#[derive(Debug)]
pub struct MessageDecoder {
    buffer: ReceiveBuffer,
}

impl Default for MessageDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageDecoder {
    /// Most undecoded bytes held at once: room for one message of the largest
    /// size plus as much again of the next.
    pub const BUFFER_LIMIT: usize = 2 * (varint::MAX_LEN + 2 + MAX_PAYLOAD_LEN);

    /// An empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: ReceiveBuffer::new(Self::BUFFER_LIMIT),
        }
    }

    /// Adds bytes read from the stream.
    ///
    /// # Errors
    ///
    /// [`Error::BufferLimitExceeded`] when the bytes not yet decoded would exceed
    /// [`Self::BUFFER_LIMIT`]. That includes a single chunk larger than the
    /// limit, however valid: push what a read returned, at most 64 KiB at a time,
    /// and drain with [`Self::next_message`] after every push.
    pub fn push(&mut self, data: &[u8]) -> Result<(), Error> {
        self.buffer
            .push(data)
            .map_err(|limit| Error::BufferLimitExceeded { limit })
    }

    /// The next whole message, or `None` until more bytes are pushed.
    ///
    /// # Errors
    ///
    /// Whatever [`ControlMessage::decode`] reports for the buffered bytes.
    pub fn next_message(&mut self) -> Result<Option<ControlMessage>, Error> {
        let Some((message, used)) = ControlMessage::decode(self.buffer.unread())? else {
            return Ok(None);
        };
        self.buffer.consume(used);
        Ok(Some(message))
    }

    /// Call when the stream ends. The draft has no rule for a stream that ends
    /// inside a message; it is treated like a Message Length that its payload
    /// does not match, a `PROTOCOL_VIOLATION` (§10).
    ///
    /// # Errors
    ///
    /// [`Error::ProtocolViolation`] when undecoded bytes are left.
    pub fn finish(&self) -> Result<(), Error> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(Error::ProtocolViolation("stream ended inside a message"))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::kvp::KeyValuePair;
    use crate::name::Location;
    use crate::params::{GroupOrder, SubscriptionFilter};
    use crate::test_util::{fixture, fixture_from_a_library};

    fn xmoq_namespace() -> TrackNamespace {
        TrackNamespace::new(vec![b"xmoq".to_vec(), b"phase0".to_vec()]).unwrap()
    }

    fn xmoq_track() -> FullTrackName {
        FullTrackName::new(xmoq_namespace(), b"object".to_vec()).unwrap()
    }

    /// Every control message captured from moqtail, with the value it stands for.
    /// In two functions only because one would be longer than clippy accepts.
    /// The fixture files this module reads.
    pub(crate) fn fixture_names() -> Vec<&'static str> {
        moqtail_messages()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    fn moqtail_messages() -> Vec<(&'static str, ControlMessage)> {
        let mut messages = moqtail_publish_side();
        messages.extend(moqtail_subscribe_side());
        messages.extend(moqtail_fetch_side());
        messages
    }

    fn moqtail_publish_side() -> Vec<(&'static str, ControlMessage)> {
        vec![
            (
                "setup-moqtail-ts",
                ControlMessage::Setup(Setup {
                    setup_options: vec![],
                }),
            ),
            (
                "setup-moqtail-relay",
                ControlMessage::Setup(Setup {
                    setup_options: vec![
                        KeyValuePair::bytes(0x07, b"moqtail/0.15.0+dd8623d".to_vec()).unwrap(),
                    ],
                }),
            ),
            (
                "publish-namespace-moqtail-ts",
                ControlMessage::PublishNamespace(PublishNamespace {
                    request_id: 0,
                    track_namespace: xmoq_namespace(),
                    parameters: vec![],
                }),
            ),
            (
                "request-ok-publish-namespace-moqtail-relay",
                ControlMessage::RequestOk(RequestOk {
                    parameters: vec![],
                    track_properties: vec![],
                }),
            ),
            (
                "publish-moqtail-ts",
                ControlMessage::Publish(Publish {
                    request_id: 2,
                    full_track_name: xmoq_track(),
                    track_alias: 779_698_128_977_928_661,
                    parameters: vec![Parameter::Forward(true)],
                    track_properties: vec![],
                }),
            ),
            (
                "publish-moqtail-rs",
                ControlMessage::Publish(Publish {
                    request_id: 0,
                    full_track_name: xmoq_track(),
                    track_alias: 1,
                    parameters: vec![Parameter::Forward(true)],
                    track_properties: vec![],
                }),
            ),
            (
                "request-ok-publish-moqtail-relay",
                ControlMessage::RequestOk(RequestOk {
                    parameters: vec![
                        Parameter::Forward(true),
                        Parameter::GroupOrder(GroupOrder::Ascending),
                    ],
                    track_properties: vec![],
                }),
            ),
        ]
    }

    fn moqtail_subscribe_side() -> Vec<(&'static str, ControlMessage)> {
        vec![
            (
                "subscribe-moqtail-rs",
                ControlMessage::Subscribe(Subscribe {
                    request_id: 0,
                    full_track_name: xmoq_track(),
                    parameters: vec![
                        Parameter::Forward(true),
                        Parameter::SubscriberPriority(128),
                        Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                        Parameter::GroupOrder(GroupOrder::Ascending),
                    ],
                }),
            ),
            (
                "subscribe-moqtail-ts",
                ControlMessage::Subscribe(Subscribe {
                    request_id: 0,
                    full_track_name: xmoq_track(),
                    parameters: vec![
                        Parameter::Forward(true),
                        Parameter::SubscriberPriority(128),
                        Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                    ],
                }),
            ),
            (
                "subscribe-ok-moqtail-relay",
                ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 0,
                    parameters: vec![],
                    track_properties: vec![],
                }),
            ),
            // Sent to a subscriber that joined after Objects had been published.
            (
                "subscribe-ok-largest-object-moqtail-relay",
                ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 0,
                    parameters: vec![Parameter::LargestObject(Location {
                        group: 3,
                        object: 2,
                    })],
                    track_properties: vec![],
                }),
            ),
            (
                "publish-done-moqtail-relay",
                ControlMessage::PublishDone(PublishDone {
                    status_code: 2,
                    stream_count: 1,
                    error_reason: ReasonPhrase::new(*b"Publisher disconnected").unwrap(),
                }),
            ),
            (
                "request-error-does-not-exist-moqtail-relay",
                ControlMessage::RequestError(RequestError {
                    error_code: 0x10,
                    retry_interval: 0,
                    error_reason: ReasonPhrase::new(*b"Unknown track namespace").unwrap(),
                    redirect: None,
                }),
            ),
        ]
    }

    /// What moqtail's test client sent for a fetch, and what its relay
    /// answered.
    fn moqtail_fetch_side() -> Vec<(&'static str, ControlMessage)> {
        vec![
            (
                "fetch-standalone-moqtail-rs",
                ControlMessage::Fetch(Fetch {
                    request_id: 0,
                    range: FetchRange::Standalone {
                        full_track_name: xmoq_track(),
                        start: Location {
                            group: 0,
                            object: 1,
                        },
                        end: Location {
                            group: 1,
                            object: 2,
                        },
                    },
                    parameters: vec![Parameter::SubscriberPriority(200)],
                }),
            ),
            (
                "fetch-relative-joining-moqtail-rs",
                ControlMessage::Fetch(Fetch {
                    request_id: 2,
                    range: FetchRange::RelativeJoining {
                        joining_request_id: 0,
                        joining_start: 0,
                    },
                    parameters: vec![Parameter::SubscriberPriority(200)],
                }),
            ),
            (
                "fetch-ok-moqtail-relay",
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: Location {
                        group: 1,
                        object: 2,
                    },
                    parameters: vec![],
                    track_properties: vec![],
                }),
            ),
            (
                "fetch-ok-joining-moqtail-relay",
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: Location {
                        group: 6,
                        object: 2,
                    },
                    parameters: vec![],
                    track_properties: vec![],
                }),
            ),
        ]
    }

    /// The messages of `fixtures/moqt-18-from-libraries/`, each with what
    /// its README says it decodes to.
    fn messages_a_library_wrote() -> Vec<(&'static str, ControlMessage)> {
        let subscribe = |parameters| {
            ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                full_track_name: xmoq_track(),
                parameters,
            })
        };
        vec![
            (
                "subscribe-delivery-timeouts-moqtail-ts",
                subscribe(vec![
                    Parameter::ObjectDeliveryTimeout(250),
                    Parameter::SubgroupDeliveryTimeout(500),
                    Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                ]),
            ),
            (
                "subscribe-subgroup-delivery-timeout-moqtail-ts",
                subscribe(vec![
                    Parameter::SubgroupDeliveryTimeout(500),
                    Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                ]),
            ),
        ]
    }

    pub(crate) fn names_of_files_a_library_wrote() -> Vec<&'static str> {
        messages_a_library_wrote()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[test]
    fn decodes_and_encodes_the_bytes_a_library_wrote() {
        for (name, message) in messages_a_library_wrote() {
            let bytes = fixture_from_a_library(name);
            assert_eq!(
                ControlMessage::decode(&bytes),
                Ok(Some((message.clone(), bytes.len()))),
                "{name}"
            );
            let mut out = Vec::new();
            message.encode(&mut out).unwrap();
            assert_eq!(out, bytes, "{name}");
        }
    }

    #[test]
    fn decodes_bytes_captured_from_moqtail() {
        for (name, message) in moqtail_messages() {
            let bytes = fixture(name);
            assert_eq!(
                ControlMessage::decode(&bytes),
                Ok(Some((message, bytes.len()))),
                "{name}"
            );
        }
    }

    #[test]
    fn encodes_the_bytes_moqtail_sent() {
        for (name, message) in moqtail_messages() {
            let mut out = Vec::new();
            message.encode(&mut out).unwrap();
            assert_eq!(out, fixture(name), "{name}");
        }
    }

    #[test]
    fn decodes_and_encodes_messages_moqtail_did_not_send() {
        let cases: &[(&[u8], ControlMessage)] = &[
            // SUBSCRIBE_OK with EXPIRES, LARGEST_OBJECT and one Track Property.
            (
                &[
                    0x04, 0x00, 0x09, 0x07, 0x02, 0x08, 0x05, 0x01, 0x03, 0x04, 0x02, 0x01,
                ],
                ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 7,
                    parameters: vec![
                        Parameter::Expires(5),
                        Parameter::LargestObject(Location {
                            group: 3,
                            object: 4,
                        }),
                    ],
                    track_properties: vec![KeyValuePair::varint(2, 1).unwrap()],
                }),
            ),
            // REQUEST_UPDATE with Request ID 4 and FORWARD = 0.
            (
                &[0x02, 0x00, 0x04, 0x04, 0x01, 0x10, 0x00],
                ControlMessage::RequestUpdate(RequestUpdate {
                    request_id: 4,
                    parameters: vec![Parameter::Forward(false)],
                }),
            ),
            // REQUEST_ERROR with REDIRECT to "a" for track () / "t".
            (
                &[
                    0x05, 0x00, 0x09, 0x34, 0x01, 0x01, 0x78, 0x01, 0x61, 0x00, 0x01, 0x74,
                ],
                ControlMessage::RequestError(RequestError {
                    error_code: 0x34,
                    retry_interval: 1,
                    error_reason: ReasonPhrase::new(*b"x").unwrap(),
                    redirect: Some(Redirect {
                        connect_uri: b"a".to_vec(),
                        full_track_name: FullTrackName::new(
                            TrackNamespace::new(vec![]).unwrap(),
                            b"t".to_vec(),
                        )
                        .unwrap(),
                    }),
                }),
            ),
        ];
        for (bytes, message) in cases {
            assert_eq!(
                ControlMessage::decode(bytes),
                Ok(Some((message.clone(), bytes.len()))),
                "{bytes:02x?}"
            );
            let mut out = Vec::new();
            message.encode(&mut out).unwrap();
            assert_eq!(out.as_slice(), *bytes, "{message:?}");
        }
    }

    #[test]
    fn waits_for_a_whole_message() {
        let bytes = fixture("subscribe-moqtail-rs");
        for len in 0..bytes.len() {
            assert_eq!(ControlMessage::decode(&bytes[..len]), Ok(None), "{len}");
        }
    }

    fn fetches_sent_to_take_the_fixtures() -> Vec<(Vec<u8>, Fetch)> {
        // Written by hand, from the two FETCH messages moqtail's test client
        // sent, and sent to the moqtail relay to take its answers
        // (`fixtures/moqt-18/README.md`). WHY read here: one of them was
        // first written with the varints of QUIC, and the relay answered
        // something else than was asked.
        let name: &[u8] = &[
            0x02, 0x04, b'x', b'm', b'o', b'q', 0x06, b'p', b'h', b'a', b's', b'e', b'0', 0x06,
            b'o', b'b', b'j', b'e', b'c', b't',
        ];
        // The Message Length (the second and third byte) is given with
        // each: 27 or 29 bytes.
        let standalone = |length: u8, after_the_name: &[u8]| {
            let mut bytes = vec![0x16, 0x00, length, 0x00, 0x01];
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(after_the_name);
            bytes
        };
        let range = |start: (u64, u64), end: (u64, u64)| FetchRange::Standalone {
            full_track_name: xmoq_track(),
            start: Location {
                group: start.0,
                object: start.1,
            },
            end: Location {
                group: end.0,
                object: end.1,
            },
        };
        let joining = |request_id, range| Fetch {
            request_id,
            range,
            parameters: vec![Parameter::SubscriberPriority(200)],
        };
        vec![
            (
                vec![0x16, 0x00, 0x07, 0x02, 0x02, 0x00, 0x01, 0x01, 0x20, 0xc8],
                joining(
                    2,
                    FetchRange::RelativeJoining {
                        joining_request_id: 0,
                        joining_start: 1,
                    },
                ),
            ),
            (
                vec![0x16, 0x00, 0x07, 0x02, 0x03, 0x00, 0x02, 0x01, 0x20, 0xc8],
                joining(
                    2,
                    FetchRange::AbsoluteJoining {
                        joining_request_id: 0,
                        joining_start: 2,
                    },
                ),
            ),
            (
                vec![0x16, 0x00, 0x05, 0x00, 0x02, 0x08, 0x00, 0x00],
                Fetch {
                    request_id: 0,
                    range: FetchRange::RelativeJoining {
                        joining_request_id: 8,
                        joining_start: 0,
                    },
                    parameters: vec![],
                },
            ),
            (
                standalone(0x1d, &[0x00, 0x00, 0x02, 0x00, 0x01, 0x22, 0x02]),
                Fetch {
                    request_id: 0,
                    range: range((0, 0), (2, 0)),
                    parameters: vec![Parameter::GroupOrder(GroupOrder::Descending)],
                },
            ),
            (
                standalone(0x1b, &[0x00, 0x00, 0x04, 0x00, 0x00]),
                Fetch {
                    request_id: 0,
                    range: range((0, 0), (4, 0)),
                    parameters: vec![],
                },
            ),
            (
                standalone(0x1b, &[0x01, 0x00, 0x01, 0x00, 0x00]),
                Fetch {
                    request_id: 0,
                    range: range((1, 0), (1, 0)),
                    parameters: vec![],
                },
            ),
            (
                standalone(0x1b, &[0x64, 0x00, 0x65, 0x00, 0x00]),
                Fetch {
                    request_id: 0,
                    range: range((100, 0), (101, 0)),
                    parameters: vec![],
                },
            ),
        ]
    }

    #[test]
    fn the_fetches_sent_to_take_the_fixtures_say_what_they_were_meant_to() {
        for (bytes, fetch) in fetches_sent_to_take_the_fixtures() {
            let message = ControlMessage::Fetch(fetch);
            assert_eq!(
                ControlMessage::decode(&bytes),
                Ok(Some((message.clone(), bytes.len()))),
                "{bytes:02x?}"
            );
            let mut encoded = Vec::new();
            message.encode(&mut encoded).unwrap();
            assert_eq!(encoded, bytes, "{message:?}");
        }
    }

    #[test]
    fn decodes_and_encodes_what_no_capture_has() {
        // Bytes written by hand from §10.12, §10.13 and §10.2: the reader of
        // the draft and the writer of the expectation are the same.
        let cases = [
            (
                // A Relative Joining FETCH, Request ID 4, with two
                // parameters: FILL_TIMEOUT (0x0A) = 5, and GROUP_ORDER
                // (0x22, written as the delta 0x18 from 0x0A) = 2.
                vec![
                    0x16, 0x00, 0x09, 0x04, 0x02, 0x00, 0x00, 0x02, 0x0a, 0x05, 0x18, 0x02,
                ],
                ControlMessage::Fetch(Fetch {
                    request_id: 4,
                    range: FetchRange::RelativeJoining {
                        joining_request_id: 0,
                        joining_start: 0,
                    },
                    parameters: vec![
                        Parameter::FillTimeout(5),
                        Parameter::GroupOrder(GroupOrder::Descending),
                    ],
                }),
            ),
            (
                // A Relative Joining FETCH with AUTHORIZATION_TOKEN (0x03),
                // one byte `aa`, and FILL_TIMEOUT (0x0A, the delta 0x07) = 5.
                vec![
                    0x16, 0x00, 0x0a, 0x00, 0x02, 0x00, 0x00, 0x02, 0x03, 0x01, 0xaa, 0x07, 0x05,
                ],
                ControlMessage::Fetch(Fetch {
                    request_id: 0,
                    range: FetchRange::RelativeJoining {
                        joining_request_id: 0,
                        joining_start: 0,
                    },
                    parameters: vec![
                        Parameter::AuthorizationToken(vec![0xaa]),
                        Parameter::FillTimeout(5),
                    ],
                }),
            ),
            (
                // FETCH_OK with End Of Track 1, End Location {3, 0}, no
                // parameters, and the Track Property 0x02 = 5.
                vec![0x18, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x02, 0x05],
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: true,
                    end_location: Location {
                        group: 3,
                        object: 0,
                    },
                    parameters: vec![],
                    track_properties: vec![KeyValuePair::varint(0x02, 5).unwrap()],
                }),
            ),
            (
                // FETCH_OK with End Of Track 1, End Location {2, 3}, no
                // parameters, and the Track Property 0x0E = 5.
                vec![0x18, 0x00, 0x06, 0x01, 0x02, 0x03, 0x00, 0x0e, 0x05],
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: true,
                    end_location: Location {
                        group: 2,
                        object: 3,
                    },
                    parameters: vec![],
                    track_properties: vec![KeyValuePair::varint(0x0e, 5).unwrap()],
                }),
            ),
        ];
        for (bytes, message) in cases {
            assert_eq!(
                ControlMessage::decode(&bytes),
                Ok(Some((message.clone(), bytes.len()))),
                "{bytes:02x?}"
            );
            let mut encoded = Vec::new();
            message.encode(&mut encoded).unwrap();
            assert_eq!(encoded, bytes, "{message:?}");
        }
    }

    #[test]
    fn rejects_malformed_input() {
        let cases: &[(&[u8], Error)] = &[
            (&[0x3f, 0x00, 0x00], Error::UnknownMessageType(0x3f)),
            // An unknown type is refused before its payload arrives.
            (&[0x3f], Error::UnknownMessageType(0x3f)),
            // GOAWAY and the PUBLISH_OK of Table 5 are in the draft, not in this crate.
            (&[0x10, 0x00, 0x00], Error::UnsupportedMessageType(0x10)),
            (&[0x1e, 0x00, 0x00], Error::UnsupportedMessageType(0x1e)),
            // SUBSCRIBE_OK whose length covers one byte more than its fields: the
            // extra byte is read as a truncated Track Property.
            (
                &[0x04, 0x00, 0x03, 0x00, 0x00, 0x00],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // PUBLISH_DONE whose length stops inside the Reason Phrase.
            (
                &[0x0b, 0x00, 0x03, 0x02, 0x01, 0x16],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // PUBLISH_DONE with one byte after the Reason Phrase.
            (
                &[0x0b, 0x00, 0x04, 0x02, 0x01, 0x00, 0x00],
                Error::ProtocolViolation("length is longer than the fields it holds"),
            ),
            // REQUEST_UPDATE carrying EXPIRES (0x08) = 5, which only a publisher
            // sends, and one carrying GROUP_ORDER (0x22) = 1, which no update may.
            (
                &[0x02, 0x00, 0x04, 0x04, 0x01, 0x08, 0x05],
                Error::ProtocolViolation("parameter not allowed in this message"),
            ),
            (
                &[0x02, 0x00, 0x04, 0x04, 0x01, 0x22, 0x01],
                Error::ProtocolViolation("parameter not allowed in this message"),
            ),
            // SUBSCRIBE_OK carrying FORWARD, a SUBSCRIBE parameter.
            (
                &[0x04, 0x00, 0x04, 0x00, 0x01, 0x10, 0x01],
                Error::ProtocolViolation("parameter not allowed in this message"),
            ),
            // FETCH with Fetch Type 4, and with Fetch Type 0 (§10.12).
            (
                &[0x16, 0x00, 0x03, 0x00, 0x04, 0x00],
                Error::ProtocolViolation("unknown fetch type"),
            ),
            (
                &[0x16, 0x00, 0x03, 0x00, 0x00, 0x00],
                Error::ProtocolViolation("unknown fetch type"),
            ),
            // A Relative Joining FETCH carrying FORWARD, a SUBSCRIBE parameter,
            // and one carrying GROUP_ORDER 3 (§10.2.8 has 1 and 2).
            (
                &[0x16, 0x00, 0x07, 0x02, 0x02, 0x00, 0x00, 0x01, 0x10, 0x01],
                Error::ProtocolViolation("parameter not allowed in this message"),
            ),
            (
                &[0x16, 0x00, 0x07, 0x02, 0x02, 0x00, 0x00, 0x01, 0x22, 0x03],
                Error::ProtocolViolation("GROUP_ORDER is neither 1 nor 2"),
            ),
            // A Standalone FETCH that stops after its Start Location.
            (
                &[0x16, 0x00, 0x07, 0x00, 0x01, 0x00, 0x01, 0x61, 0x00, 0x01],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // A Relative Joining FETCH with the parameter 0x01, which draft 18
            // does not define (§10.2.1).
            (
                &[0x16, 0x00, 0x07, 0x00, 0x02, 0x00, 0x00, 0x01, 0x01, 0x00],
                Error::UnknownParameter(0x01),
            ),
            // FETCH_OK with End Of Track 2, and one carrying EXPIRES (0x08):
            // no parameter of the draft lists FETCH_OK.
            (
                &[0x18, 0x00, 0x04, 0x02, 0x01, 0x02, 0x00],
                Error::ProtocolViolation("End Of Track is neither 0 nor 1"),
            ),
            (
                &[0x18, 0x00, 0x06, 0x00, 0x01, 0x02, 0x01, 0x08, 0x05],
                Error::ProtocolViolation("parameter not allowed in this message"),
            ),
            // REQUEST_ERROR with a 1025-byte Reason Phrase announced.
            (
                &[0x05, 0x00, 0x04, 0x00, 0x00, 0x84, 0x01],
                Error::ProtocolViolation("reason phrase longer than 1024 bytes"),
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(
                ControlMessage::decode(bytes),
                Err(error.clone()),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn refuses_to_encode_what_the_wire_cannot_carry() {
        let cases = [
            (
                ControlMessage::Setup(Setup {
                    setup_options: vec![KeyValuePair::bytes(0x07, vec![0; 65_535]).unwrap()],
                }),
                UsageError("message payload longer than 65535 bytes"),
            ),
            (
                ControlMessage::RequestError(RequestError {
                    error_code: 0x10,
                    retry_interval: 0,
                    error_reason: ReasonPhrase::default(),
                    redirect: Some(Redirect {
                        connect_uri: vec![],
                        full_track_name: xmoq_track(),
                    }),
                }),
                UsageError("a Redirect goes with error code REDIRECT and no other"),
            ),
        ];
        for (message, error) in cases {
            assert_eq!(message.encode(&mut Vec::new()), Err(error));
        }
    }

    #[test]
    fn a_redirect_without_a_track_means_the_original_one() {
        type Case = (Vec<Vec<u8>>, &'static [u8], bool);
        let redirect = |fields: Vec<Vec<u8>>, name: &[u8]| Redirect {
            connect_uri: vec![],
            full_track_name: FullTrackName::new(
                TrackNamespace::new(fields).unwrap(),
                name.to_vec(),
            )
            .unwrap(),
        };
        // (namespace fields, track name, whether the Redirect names a track)
        let cases: [Case; 4] = [
            (vec![], b"", false),
            (vec![], b"t", true),
            (vec![b"n".to_vec()], b"", true),
            (vec![b"n".to_vec()], b"t", true),
        ];
        for (fields, name, names_a_track) in cases {
            let redirect = redirect(fields.clone(), name);
            assert_eq!(
                redirect.track().is_some(),
                names_a_track,
                "{fields:?} / {name:?}"
            );
        }
    }

    #[test]
    fn messages_expose_the_two_track_fields() {
        let subscribe = Subscribe {
            request_id: 0,
            full_track_name: xmoq_track(),
            parameters: vec![],
        };
        assert_eq!(subscribe.track_namespace(), &xmoq_namespace());
        assert_eq!(subscribe.track_name(), b"object");

        let publish = Publish {
            request_id: 0,
            full_track_name: xmoq_track(),
            track_alias: 1,
            parameters: vec![],
            track_properties: vec![],
        };
        assert_eq!(publish.track_namespace(), &xmoq_namespace());
        assert_eq!(publish.track_name(), b"object");
    }

    #[test]
    fn requests_this_crate_does_not_implement_yield_their_request_id() {
        // (bytes, message type, Request ID). Only the Request ID is decoded;
        // the rest of the payload (0x01, 0x02) is skipped.
        let cases: [(&[u8], u64, u64); 3] = [
            (&[0x0d, 0x00, 0x03, 0x06, 0x01, 0x02], 0x0d, 6),
            (&[0x50, 0x00, 0x03, 0x06, 0x01, 0x02], 0x50, 6),
            (&[0x51, 0x00, 0x03, 0x06, 0x01, 0x02], 0x51, 6),
        ];
        for (bytes, message_type, request_id) in cases {
            let message = ControlMessage::UnsupportedRequest {
                message_type,
                request_id,
            };
            assert_eq!(
                ControlMessage::decode(bytes),
                Ok(Some((message.clone(), bytes.len()))),
                "{bytes:02x?}"
            );
            assert_eq!(
                message.encode(&mut Vec::new()),
                Err(UsageError(
                    "this crate does not send requests it does not implement"
                )),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn reassembles_messages_split_across_chunks() {
        let names = ["setup-moqtail-relay", "request-ok-publish-moqtail-relay"];
        let table = moqtail_messages();
        let expected: Vec<ControlMessage> = names
            .iter()
            .map(|name| {
                let (_, message) = table
                    .iter()
                    .find(|(listed, _)| listed == name)
                    .unwrap_or_else(|| panic!("{name} is not in the table"));
                message.clone()
            })
            .collect();
        let stream: Vec<u8> = names.iter().flat_map(|name| fixture(name)).collect();

        for chunk_len in [1, 3, 7, stream.len()] {
            let mut decoder = MessageDecoder::new();
            let mut messages = Vec::new();
            for chunk in stream.chunks(chunk_len) {
                decoder.push(chunk).unwrap();
                while let Some(message) = decoder.next_message().unwrap() {
                    messages.push(message);
                }
            }
            assert_eq!(messages, expected, "chunks of {chunk_len}");
            assert_eq!(decoder.finish(), Ok(()));
        }
    }

    #[test]
    fn a_stream_must_not_end_inside_a_message() {
        // What had arrived when the stream ended. None of these is a whole
        // message, so the decoder has yielded nothing.
        let cases: [&[u8]; 5] = [
            // REQUEST_OK: the type and half of the length.
            &[0x07, 0x00],
            // FETCH: a length of 5, and 3 bytes of the payload.
            &[0x16, 0x00, 0x05, 0x01, 0x02, 0x00],
            // TRACK_STATUS, which is not decoded: a length of 9, and 2 bytes.
            &[0x0d, 0x00, 0x09, 0x01, 0x00],
            // SUBSCRIBE: a length of 31, and 1 byte.
            &[0x03, 0x00, 0x1f, 0x80],
            // SUBSCRIBE: the type alone.
            &[0x03],
        ];
        for bytes in cases {
            let mut decoder = MessageDecoder::new();
            decoder.push(bytes).unwrap();
            assert_eq!(decoder.next_message(), Ok(None), "{bytes:02x?}");
            assert_eq!(
                decoder.finish(),
                Err(Error::ProtocolViolation("stream ended inside a message")),
                "{bytes:02x?}"
            );
        }
    }

    #[test]
    fn bounds_what_it_buffers() {
        let mut decoder = MessageDecoder::new();
        let chunk = vec![0u8; MessageDecoder::BUFFER_LIMIT];
        decoder.push(&chunk).unwrap();
        assert_eq!(
            decoder.push(&[0]),
            Err(Error::BufferLimitExceeded { limit: 131_092 })
        );
    }
}
