use std::fmt;

/// Session termination codes (§3.5) that this crate can ask a caller to close with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TerminationCode {
    /// `INTERNAL_ERROR`: the limitation is on this side.
    InternalError,
    /// `PROTOCOL_VIOLATION`: the peer did something the draft forbids.
    ProtocolViolation,
    /// `INVALID_REQUEST_ID` (§10.1).
    InvalidRequestId,
    /// `DUPLICATE_TRACK_ALIAS` (§11.1).
    DuplicateTrackAlias,
    /// `INVALID_PATH` (§10.3.1.2).
    InvalidPath,
    /// `INVALID_AUTHORITY` (§10.3.1.1).
    InvalidAuthority,
}

impl TerminationCode {
    /// The value carried when the session is closed (§3.5).
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            Self::InternalError => 0x1,
            Self::ProtocolViolation => 0x3,
            Self::InvalidRequestId => 0x4,
            Self::DuplicateTrackAlias => 0x5,
            Self::InvalidPath => 0x8,
            Self::InvalidAuthority => 0x19,
        }
    }
}

/// Stream Reset Error Codes (§3.3.3), for resetting a stream and for
/// `STOP_SENDING`.
pub mod stream_reset_code {
    /// `INTERNAL_ERROR`: an implementation specific error.
    pub const INTERNAL_ERROR: u32 = 0x0;
    /// CANCELLED: the stream was cancelled by either endpoint.
    pub const CANCELLED: u32 = 0x1;
    /// `DELIVERY_TIMEOUT`: a delivery timeout (§8) was exceeded for the
    /// stream.
    pub const DELIVERY_TIMEOUT: u32 = 0x2;
    /// `SESSION_CLOSED`: the session is being closed.
    pub const SESSION_CLOSED: u32 = 0x3;
    /// `GOING_AWAY`: the endpoint has sent or received a GOAWAY.
    pub const GOING_AWAY: u32 = 0x4;
    /// `TOO_FAR_BEHIND`: the subscription exceeded the publisher's resource
    /// limits and is being terminated.
    pub const TOO_FAR_BEHIND: u32 = 0x5;
    /// `UNKNOWN_OBJECT_STATUS`: in response to a FETCH, the publisher cannot
    /// determine the status of the next Object in the requested range.
    pub const UNKNOWN_OBJECT_STATUS: u32 = 0x6;
    /// `EXCESSIVE_LOAD`: the endpoint is overloaded and is resetting the
    /// stream.
    pub const EXCESSIVE_LOAD: u32 = 0x9;
    /// `MALFORMED_TRACK`: the track was malformed (§2.4.2).
    pub const MALFORMED_TRACK: u32 = 0x12;
}

/// How much an [`Error`] ends, and the code to end it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorScope {
    /// Only the stream the input came from. The receiver stops it with
    /// `STOP_SENDING` and this code; the subscription and the session go on.
    Stream {
        /// One of [`stream_reset_code`].
        reset_code: u32,
    },
    /// The subscription the stream belongs to. The receiver cancels the
    /// subscription and stops its streams with this code; the session goes on.
    Subscription {
        /// One of [`stream_reset_code`].
        reset_code: u32,
    },
    /// The session, which has to be closed with this code.
    Session(TerminationCode),
}

/// What can go wrong with input from a peer.
///
/// Input from a peer never panics; it ends in one of these. Mistakes of the
/// caller are a separate type, [`UsageError`], so that a bug on this side is never
/// reported to the peer as its violation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The peer broke a rule for which the draft requires `PROTOCOL_VIOLATION`.
    ProtocolViolation(&'static str),
    /// A message type the draft does not define (§10).
    UnknownMessageType(u64),
    /// A message type the draft defines and this crate does not implement.
    UnsupportedMessageType(u64),
    /// A unidirectional stream type the draft does not define (§3.4).
    UnknownStreamType(u64),
    /// A Message Parameter type the draft does not define (§10.2).
    UnknownParameter(u64),
    /// A Track Alias already in use for another track (§11.1).
    DuplicateTrackAlias(u64),
    /// A Request ID of the wrong parity for its sender, or one the sender
    /// already used (§10.1).
    InvalidRequestId(u64),
    /// AUTHORITY from a server or over WebTransport (§10.3.1.1).
    InvalidAuthority,
    /// PATH from a server or over WebTransport (§10.3.1.2).
    InvalidPath,
    /// A track broke the object model (§2.4.2). The subscription is cancelled; the
    /// session stays open.
    MalformedTrack(&'static str),
    /// More control-message bytes were buffered than the decoder holds. The
    /// session cannot go on without its control messages.
    BufferLimitExceeded {
        /// The cap, in bytes.
        limit: usize,
    },
    /// A subgroup stream held more undecoded bytes than the decoder was
    /// configured for, which means an Object over the size this endpoint takes.
    /// Only that stream is stopped; the session stays open.
    ObjectTooLarge {
        /// The configured cap, in bytes.
        limit: usize,
    },
    /// A control message held more Key-Value-Pairs than this crate decodes. The
    /// session cannot go on without its control messages.
    TooManyKeyValuePairs {
        /// The cap, in pairs.
        limit: usize,
    },
    /// A control message held more Message Parameters than this crate decodes.
    /// The session cannot go on without its control messages.
    TooManyParameters {
        /// The cap, in parameters.
        limit: usize,
    },
    /// An Object held more Properties than this crate decodes. Only that stream
    /// is stopped; the session stays open.
    TooManyObjectProperties {
        /// The cap, in pairs.
        limit: usize,
    },
}

impl Error {
    /// What the error ends: the stream it came from, the subscription, or the
    /// session.
    #[must_use]
    pub fn scope(&self) -> ErrorScope {
        let session = ErrorScope::Session;
        match self {
            // §10: an unknown message type closes the session; §3.4: so does an
            // unknown stream type. Neither names a code, PROTOCOL_VIOLATION is the
            // closest one defined.
            Self::ProtocolViolation(_)
            | Self::UnknownMessageType(_)
            | Self::UnknownStreamType(_)
            | Self::UnknownParameter(_) => session(TerminationCode::ProtocolViolation),
            Self::DuplicateTrackAlias(_) => session(TerminationCode::DuplicateTrackAlias),
            Self::InvalidRequestId(_) => session(TerminationCode::InvalidRequestId),
            Self::InvalidAuthority => session(TerminationCode::InvalidAuthority),
            Self::InvalidPath => session(TerminationCode::InvalidPath),
            // The peer was within the draft; the limitation is ours.
            Self::UnsupportedMessageType(_)
            | Self::BufferLimitExceeded { .. }
            | Self::TooManyKeyValuePairs { .. }
            | Self::TooManyParameters { .. } => session(TerminationCode::InternalError),
            // §2.4.2: the receiver of a malformed track cancels the subscription.
            Self::MalformedTrack(_) => ErrorScope::Subscription {
                reset_code: stream_reset_code::MALFORMED_TRACK,
            },
            // A limit of this endpoint on one Object says nothing about the
            // rest of the track.
            Self::ObjectTooLarge { .. } | Self::TooManyObjectProperties { .. } => {
                ErrorScope::Stream {
                    reset_code: stream_reset_code::INTERNAL_ERROR,
                }
            }
        }
    }

    /// The code to close the session with, or `None` when the error does not call
    /// for closing it.
    #[must_use]
    pub fn termination_code(&self) -> Option<TerminationCode> {
        match self.scope() {
            ErrorScope::Session(code) => Some(code),
            ErrorScope::Stream { .. } | ErrorScope::Subscription { .. } => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtocolViolation(why) => write!(f, "protocol violation: {why}"),
            Self::UnknownMessageType(ty) => write!(f, "unknown message type {ty:#x}"),
            Self::UnsupportedMessageType(ty) => write!(f, "unsupported message type {ty:#x}"),
            Self::UnknownStreamType(ty) => write!(f, "unknown stream type {ty:#x}"),
            Self::UnknownParameter(ty) => write!(f, "unknown message parameter {ty:#x}"),
            Self::DuplicateTrackAlias(alias) => write!(f, "duplicate track alias {alias}"),
            Self::InvalidRequestId(id) => write!(f, "invalid request id {id}"),
            Self::InvalidAuthority => write!(f, "AUTHORITY option not allowed here"),
            Self::InvalidPath => write!(f, "PATH option not allowed here"),
            Self::MalformedTrack(why) => write!(f, "malformed track: {why}"),
            Self::BufferLimitExceeded { limit } => {
                write!(f, "buffer limit of {limit} bytes exceeded")
            }
            Self::ObjectTooLarge { limit } => {
                write!(f, "object does not fit the stream buffer of {limit} bytes")
            }
            Self::TooManyKeyValuePairs { limit } => {
                write!(f, "more than {limit} key-value pairs in a control message")
            }
            Self::TooManyParameters { limit } => {
                write!(f, "more than {limit} parameters in a control message")
            }
            Self::TooManyObjectProperties { limit } => {
                write!(f, "more than {limit} properties on an object")
            }
        }
    }
}

impl std::error::Error for Error {}

/// A mistake of the caller: a value the wire cannot carry, or a state machine
/// used out of order. It says nothing about the peer and never closes a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageError(pub(crate) &'static str);

impl UsageError {
    /// What was wrong, as a fixed English sentence fragment.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid use of the MoQ Transport API: {}", self.0)
    }
}

impl std::error::Error for UsageError {}

/// Why a message for a request could not be taken: the peer broke the protocol,
/// or the caller named a request that is not this session's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestFault {
    /// The peer is at fault. The session has to be closed with
    /// [`Error::termination_code`].
    Peer(Error),
    /// The caller is at fault. Nothing was changed and nothing is sent to the
    /// peer.
    Usage(UsageError),
}

impl From<Error> for RequestFault {
    fn from(error: Error) -> Self {
        Self::Peer(error)
    }
}

impl From<UsageError> for RequestFault {
    fn from(error: UsageError) -> Self {
        Self::Usage(error)
    }
}

impl fmt::Display for RequestFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Peer(error) => error.fmt(f),
            Self::Usage(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RequestFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Peer(error) => Some(error),
            Self::Usage(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_ends_a_stream_a_subscription_or_the_session() {
        let cases = [
            (
                Error::ProtocolViolation("test"),
                ErrorScope::Session(TerminationCode::ProtocolViolation),
                Some(0x3),
            ),
            (
                Error::UnknownMessageType(0x3f),
                ErrorScope::Session(TerminationCode::ProtocolViolation),
                Some(0x3),
            ),
            (
                Error::UnsupportedMessageType(0x10),
                ErrorScope::Session(TerminationCode::InternalError),
                Some(0x1),
            ),
            (
                Error::BufferLimitExceeded { limit: 8 },
                ErrorScope::Session(TerminationCode::InternalError),
                Some(0x1),
            ),
            (
                Error::TooManyKeyValuePairs { limit: 8 },
                ErrorScope::Session(TerminationCode::InternalError),
                Some(0x1),
            ),
            (
                Error::TooManyParameters { limit: 8 },
                ErrorScope::Session(TerminationCode::InternalError),
                Some(0x1),
            ),
            (
                Error::DuplicateTrackAlias(7),
                ErrorScope::Session(TerminationCode::DuplicateTrackAlias),
                Some(0x5),
            ),
            (
                Error::InvalidRequestId(3),
                ErrorScope::Session(TerminationCode::InvalidRequestId),
                Some(0x4),
            ),
            (
                Error::InvalidPath,
                ErrorScope::Session(TerminationCode::InvalidPath),
                Some(0x8),
            ),
            (
                Error::InvalidAuthority,
                ErrorScope::Session(TerminationCode::InvalidAuthority),
                Some(0x19),
            ),
            (
                Error::MalformedTrack("test"),
                ErrorScope::Subscription { reset_code: 0x12 },
                None,
            ),
            (
                Error::ObjectTooLarge { limit: 8 },
                ErrorScope::Stream { reset_code: 0x0 },
                None,
            ),
            (
                Error::TooManyObjectProperties { limit: 8 },
                ErrorScope::Stream { reset_code: 0x0 },
                None,
            ),
        ];
        for (error, scope, code) in cases {
            assert_eq!(error.scope(), scope, "{error:?}");
            assert_eq!(
                error.termination_code().map(TerminationCode::code),
                code,
                "{error:?}"
            );
        }
    }
}
