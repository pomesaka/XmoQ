//! Message Parameters (§10.2).
//!
//! Unlike Key-Value-Pairs, the value encoding of a parameter comes from its
//! definition and not from the parity of its type, and a block is bounded by a
//! count. A receiver therefore has to know every type it is sent.

use crate::coding::{ByteReader, ReadResult, parse_exact, put_count, put_length_prefixed};
use crate::name::{Location, TrackNamespace};
use crate::{Error, UsageError, varint};

const OBJECT_DELIVERY_TIMEOUT: u64 = 0x02;
const AUTHORIZATION_TOKEN: u64 = 0x03;
const RENDEZVOUS_TIMEOUT: u64 = 0x04;
const SUBGROUP_DELIVERY_TIMEOUT: u64 = 0x06;
const EXPIRES: u64 = 0x08;
const LARGEST_OBJECT: u64 = 0x09;
const FILL_TIMEOUT: u64 = 0x0a;
const FORWARD: u64 = 0x10;
const SUBSCRIBER_PRIORITY: u64 = 0x20;
const SUBSCRIPTION_FILTER: u64 = 0x21;
const GROUP_ORDER: u64 = 0x22;
const NEW_GROUP_REQUEST: u64 = 0x32;
const TRACK_NAMESPACE_PREFIX: u64 = 0x34;

/// Most Message Parameters decoded from one message.
///
/// WHY a count: only `AUTHORIZATION_TOKEN` may repeat, and an empty token takes
/// two bytes on the wire and far more as a value in memory, so one message
/// could otherwise carry tens of thousands of them.
///
/// Provisional, and the same number as [`crate::kvp::MAX_PAIRS`]: the draft sets
/// no limit, and `XmoQ` has not decided one.
pub const MAX_PARAMETERS: usize = crate::kvp::MAX_PAIRS;

const FILTER_NEXT_GROUP_START: u64 = 0x1;
const FILTER_LARGEST_OBJECT: u64 = 0x2;
const FILTER_ABSOLUTE_START: u64 = 0x3;
const FILTER_ABSOLUTE_RANGE: u64 = 0x4;

/// Value of the `GROUP_ORDER` parameter (§10.2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOrder {
    /// 0x1: lower Group IDs first.
    Ascending,
    /// 0x2: higher Group IDs first.
    Descending,
}

impl GroupOrder {
    fn code(self) -> u8 {
        match self {
            Self::Ascending => 0x1,
            Self::Descending => 0x2,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        [Self::Ascending, Self::Descending]
            .into_iter()
            .find(|order| order.code() == code)
    }
}

/// Subscription Filter (§5.1.2): which Objects of a track a subscription passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionFilter {
    /// 0x1: from the first Object of the Group after the Largest Object.
    NextGroupStart,
    /// 0x2: from the Object after the Largest Object.
    LargestObject,
    /// 0x3: from `start`, open ended.
    AbsoluteStart {
        /// First Location that passes.
        start: Location,
    },
    /// 0x4: from `start` to the end of the Group `start.group + end_group_delta`.
    AbsoluteRange {
        /// First Location that passes.
        start: Location,
        /// Groups to deliver after the one `start` is in.
        end_group_delta: u64,
    },
}

/// One Message Parameter. Each variant is named after the parameter it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parameter {
    /// 0x02, milliseconds (§10.2.4).
    ObjectDeliveryTimeout(u64),
    /// 0x03. The Token structure of §10.2.2, kept as received.
    ///
    /// The session does not decode it, and leaves two requirements of
    /// §10.2.2 to its caller: a server closes the session for Alias Type
    /// DELETE or `USE_ALIAS` in SETUP, and for a Token that cannot be decoded
    /// with `KEY_VALUE_FORMATTING_ERROR`. The relay meets both when it is
    /// given a key (`crates/relay/src/auth.rs`, with [`crate::token`]).
    ///
    /// FIXME: a relay without a key, and every client, meets neither.
    AuthorizationToken(Vec<u8>),
    /// 0x04, milliseconds (§10.2.6).
    RendezvousTimeout(u64),
    /// 0x06, milliseconds (§10.2.3).
    SubgroupDeliveryTimeout(u64),
    /// 0x08, milliseconds until the sender ends the subscription; 0 for no or an
    /// unknown expiry (§10.2.10).
    Expires(u64),
    /// 0x09, the largest Location the sender has seen in the track (§10.2.11).
    LargestObject(Location),
    /// 0x0A, milliseconds a relay may spend on getting Objects of a FETCH
    /// from upstream; 0 for none at all (§10.2.5).
    FillTimeout(u64),
    /// 0x10, the Forward State: whether Objects are sent (§10.2.12).
    Forward(bool),
    /// 0x20, where a lower number is a higher priority (§10.2.7).
    SubscriberPriority(u8),
    /// 0x21 (§10.2.9).
    SubscriptionFilter(SubscriptionFilter),
    /// 0x22 (§10.2.8).
    GroupOrder(GroupOrder),
    /// 0x32, the largest Group ID the subscriber knows plus 1 (§10.2.13).
    NewGroupRequest(u64),
    /// 0x34, the new Track Namespace Prefix of a `SUBSCRIBE_NAMESPACE` or
    /// `SUBSCRIBE_TRACKS` (§10.2.14).
    TrackNamespacePrefix(TrackNamespace),
}

/// The messages whose parameters this crate checks. `PublishOk` and
/// `PublishNamespaceOk` are the draft's names for a `REQUEST_OK` that answers
/// PUBLISH or `PUBLISH_NAMESPACE` (§10.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// SUBSCRIBE.
    Subscribe,
    /// `SUBSCRIBE_OK`.
    SubscribeOk,
    /// PUBLISH.
    Publish,
    /// `REQUEST_OK` answering PUBLISH.
    PublishOk,
    /// `PUBLISH_NAMESPACE`.
    PublishNamespace,
    /// `REQUEST_OK` answering `PUBLISH_NAMESPACE`.
    PublishNamespaceOk,
    /// `REQUEST_UPDATE` for a subscription. Updates of the other request kinds
    /// take other parameters and are not decoded by this crate.
    RequestUpdate,
    /// `REQUEST_UPDATE` for a FETCH. The decoder cannot tell it from the one
    /// for a subscription; the session checks it where it knows the request.
    FetchUpdate,
    /// `REQUEST_UPDATE` for a `SUBSCRIBE_NAMESPACE`, checked by the session as
    /// `FetchUpdate` is.
    NamespaceUpdate,
    /// FETCH.
    Fetch,
    /// `FETCH_OK`. No parameter of the draft lists it.
    FetchOk,
    /// `SUBSCRIBE_NAMESPACE`: `AUTHORIZATION_TOKEN` alone (§10.2.2).
    SubscribeNamespace,
    /// `TRACK_STATUS`: `AUTHORIZATION_TOKEN` alone (§10.2.2). §10.14 has its
    /// format "identical to the SUBSCRIBE message", "but subscriber
    /// parameters related to Track delivery ... are not included", and no
    /// other parameter of §10.2 names it.
    TrackStatus,
    /// `REQUEST_OK` answering `TRACK_STATUS`: `LARGEST_OBJECT` alone
    /// (§10.2.11).
    TrackStatusOk,
}

impl Parameter {
    fn ty(&self) -> u64 {
        match self {
            Self::ObjectDeliveryTimeout(_) => OBJECT_DELIVERY_TIMEOUT,
            Self::AuthorizationToken(_) => AUTHORIZATION_TOKEN,
            Self::RendezvousTimeout(_) => RENDEZVOUS_TIMEOUT,
            Self::SubgroupDeliveryTimeout(_) => SUBGROUP_DELIVERY_TIMEOUT,
            Self::Expires(_) => EXPIRES,
            Self::LargestObject(_) => LARGEST_OBJECT,
            Self::FillTimeout(_) => FILL_TIMEOUT,
            Self::Forward(_) => FORWARD,
            Self::SubscriberPriority(_) => SUBSCRIBER_PRIORITY,
            Self::SubscriptionFilter(_) => SUBSCRIPTION_FILTER,
            Self::GroupOrder(_) => GROUP_ORDER,
            Self::NewGroupRequest(_) => NEW_GROUP_REQUEST,
            Self::TrackNamespacePrefix(_) => TRACK_NAMESPACE_PREFIX,
        }
    }

    /// Whether the parameter's definition lists `kind` among the messages it MAY
    /// appear in (§10.2.2 to §10.2.13).
    fn allowed_in(&self, kind: MessageKind) -> bool {
        use MessageKind::{
            Fetch, FetchUpdate, NamespaceUpdate, Publish, PublishNamespace, PublishOk,
            RequestUpdate, Subscribe, SubscribeNamespace, SubscribeOk, TrackStatus, TrackStatusOk,
        };
        // WHY `FetchUpdate` in three lists only: §10.2.2, §10.2.3 and §10.2.4
        // name `REQUEST_UPDATE` without saying of which request, and §10.2.7
        // has "REQUEST_UPDATE (for a subscription or FETCH)". FORWARD, the
        // Subscription Filter and `NEW_GROUP_REQUEST` have "REQUEST_UPDATE
        // (for a subscription)" (§10.2.12, §10.2.9, §10.2.13).
        let allowed: &[MessageKind] = match self {
            Self::ObjectDeliveryTimeout(_) | Self::SubgroupDeliveryTimeout(_) => {
                &[PublishOk, Subscribe, RequestUpdate, FetchUpdate]
            }
            Self::AuthorizationToken(_) => &[
                Publish,
                Subscribe,
                RequestUpdate,
                FetchUpdate,
                NamespaceUpdate,
                PublishNamespace,
                Fetch,
                SubscribeNamespace,
                TrackStatus,
            ],
            Self::RendezvousTimeout(_) => &[Subscribe],
            Self::Expires(_) => &[SubscribeOk, Publish, PublishOk],
            Self::LargestObject(_) => &[SubscribeOk, Publish, TrackStatusOk],
            Self::FillTimeout(_) => &[Fetch],
            Self::Forward(_) => &[Subscribe, RequestUpdate, Publish, PublishOk],
            Self::SubscriberPriority(_) => {
                &[Subscribe, RequestUpdate, FetchUpdate, PublishOk, Fetch]
            }
            Self::SubscriptionFilter(_) => &[Subscribe, RequestUpdate, PublishOk],
            // §10.2.8 lists SUBSCRIBE, PUBLISH_OK and FETCH, and no update.
            Self::GroupOrder(_) => &[Subscribe, PublishOk, Fetch],
            Self::NewGroupRequest(_) => &[PublishOk, Subscribe, RequestUpdate],
            // §10.2.14: "MAY appear in REQUEST_UPDATE for a SUBSCRIBE_NAMESPACE
            // or SUBSCRIBE_TRACKS request".
            Self::TrackNamespacePrefix(_) => &[NamespaceUpdate],
        };
        allowed.contains(&kind)
    }
}

/// §10.2: a type may repeat in a message only where its definition allows it,
/// and only `AUTHORIZATION_TOKEN` does (§10.2.2).
fn may_repeat(ty: u64) -> bool {
    ty == AUTHORIZATION_TOKEN
}

/// Whether every parameter may appear in a message of `kind` (§10.2.1). For a
/// received message a `false` is a `PROTOCOL_VIOLATION`.
#[must_use]
pub fn all_allowed(parameters: &[Parameter], kind: MessageKind) -> bool {
    parameters
        .iter()
        .all(|parameter| parameter.allowed_in(kind))
}

/// Whether every parameter of a `REQUEST_UPDATE` may appear in the update of
/// some request: what the decoder can check without knowing which request
/// the update is for. The session checks the rest (`MessageKind::FetchUpdate`,
/// `MessageKind::NamespaceUpdate`).
#[must_use]
pub fn allowed_in_some_update(parameters: &[Parameter]) -> bool {
    parameters.iter().all(|parameter| {
        [
            MessageKind::RequestUpdate,
            MessageKind::FetchUpdate,
            MessageKind::NamespaceUpdate,
        ]
        .into_iter()
        .any(|kind| parameter.allowed_in(kind))
    })
}

/// The Forward State a parameter list sets. FORWARD omitted means 1 in every
/// message this crate handles (§10.2.12).
#[must_use]
pub fn forward(parameters: &[Parameter]) -> bool {
    parameters
        .iter()
        .find_map(|parameter| match parameter {
            Parameter::Forward(forward) => Some(*forward),
            _ => None,
        })
        .unwrap_or(true)
}

/// The `RENDEZVOUS_TIMEOUT` a parameter list has, in milliseconds. `None`
/// without one and with 0: §10.2.6, "If `RENDEZVOUS_TIMEOUT` is absent, the
/// default is 0", and "A value of 0 indicates the subscriber does not want
/// to wait".
#[must_use]
pub fn rendezvous_timeout(parameters: &[Parameter]) -> Option<u64> {
    parameters
        .iter()
        .find_map(|parameter| match parameter {
            Parameter::RendezvousTimeout(milliseconds) => Some(*milliseconds),
            _ => None,
        })
        .filter(|milliseconds| *milliseconds != 0)
}

/// The `SUBGROUP_DELIVERY_TIMEOUT` a parameter list sets, in milliseconds.
/// None without the parameter and for 0 (§8: "a value of 0 means that there
/// is no timeout set").
#[must_use]
pub fn subgroup_delivery_timeout(parameters: &[Parameter]) -> Option<u64> {
    parameters
        .iter()
        .find_map(|parameter| match parameter {
            Parameter::SubgroupDeliveryTimeout(milliseconds) => Some(*milliseconds),
            _ => None,
        })
        .filter(|milliseconds| *milliseconds != 0)
}

/// The Subscription Filter a parameter list has; none means that the
/// subscription is unfiltered (§10.2.9).
#[must_use]
pub fn subscription_filter(parameters: &[Parameter]) -> Option<SubscriptionFilter> {
    parameters.iter().find_map(|parameter| match parameter {
        Parameter::SubscriptionFilter(filter) => Some(*filter),
        _ => None,
    })
}

/// Reads `Number of Parameters` and that many parameters.
///
/// More than [`MAX_PARAMETERS`] are [`Error::TooManyParameters`], which ends the
/// session.
pub(crate) fn decode_all(reader: &mut ByteReader<'_>) -> ReadResult<Vec<Parameter>> {
    let count = reader.varint()?;
    if usize::try_from(count).map_or(true, |count| count > MAX_PARAMETERS) {
        return Err(Error::TooManyParameters {
            limit: MAX_PARAMETERS,
        }
        .into());
    }
    // Not pre-allocated from `count`: the peer chooses it, and the loop stops as
    // soon as the bytes run out.
    let mut parameters = Vec::new();
    let mut previous_type = 0u64;
    for index in 0..count {
        let delta = reader.varint()?;
        let ty = previous_type
            .checked_add(delta)
            .ok_or(Error::ProtocolViolation("parameter type exceeds 64 bits"))?;
        if index > 0 && delta == 0 && !may_repeat(ty) {
            return Err(Error::ProtocolViolation("duplicate parameter").into());
        }
        parameters.push(decode_value(ty, reader)?);
        previous_type = ty;
    }
    Ok(parameters)
}

fn decode_value(ty: u64, reader: &mut ByteReader<'_>) -> ReadResult<Parameter> {
    Ok(match ty {
        OBJECT_DELIVERY_TIMEOUT => Parameter::ObjectDeliveryTimeout(reader.varint()?),
        AUTHORIZATION_TOKEN => {
            let len = reader.length()?;
            Parameter::AuthorizationToken(reader.bytes(len)?.to_vec())
        }
        // §10.2.6 does not name an encoding. Every other duration in §10.2 is a
        // varint, so this one is read as a varint too. The moqtail crate 0.15.0
        // writes it as a varint as well (`serialize_message_parameters` in
        // `libs/moqtail-rs/src/model/parameter/message_parameter.rs` at tag
        // `relay@0.15.0`, read 2026-10-10).
        RENDEZVOUS_TIMEOUT => Parameter::RendezvousTimeout(reader.varint()?),
        SUBGROUP_DELIVERY_TIMEOUT => Parameter::SubgroupDeliveryTimeout(reader.varint()?),
        EXPIRES => Parameter::Expires(reader.varint()?),
        LARGEST_OBJECT => Parameter::LargestObject(Location::decode(reader)?),
        FORWARD => Parameter::Forward(match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::ProtocolViolation("FORWARD is neither 0 nor 1").into()),
        }),
        SUBSCRIBER_PRIORITY => Parameter::SubscriberPriority(reader.u8()?),
        SUBSCRIPTION_FILTER => {
            let len = reader.length()?;
            let filter = parse_exact(reader.bytes(len)?, decode_filter)?;
            Parameter::SubscriptionFilter(filter)
        }
        GROUP_ORDER => Parameter::GroupOrder(
            GroupOrder::from_code(reader.u8()?)
                .ok_or(Error::ProtocolViolation("GROUP_ORDER is neither 1 nor 2"))?,
        ),
        NEW_GROUP_REQUEST => Parameter::NewGroupRequest(reader.varint()?),
        // §10.2.5 does not name an encoding, like §10.2.6 above; read as a
        // varint for the same reason. The moqtail crate 0.15.0 does the same
        // (`FillTimeout` in `src/model/parameter/message_parameter.rs` of the
        // crate, read 2026-10-11).
        FILL_TIMEOUT => Parameter::FillTimeout(reader.varint()?),
        // §10.2.14: "uses the Track Namespace encoding described in Section
        // 2.4.1", with no length before it. WHY NOT a length first, as the
        // moqtail crate 0.15.0 writes it (`KeyValuePair::Bytes` in
        // `libs/moqtail-rs/src/model/parameter/message_parameter.rs` at tag
        // `relay@0.15.0`, read 2026-10-24): §10.2 names "Length-prefixed" as an
        // encoding of its own, and this definition does not use it.
        TRACK_NAMESPACE_PREFIX => Parameter::TrackNamespacePrefix(TrackNamespace::decode(reader)?),
        unknown => return Err(Error::UnknownParameter(unknown).into()),
    })
}

fn decode_filter(reader: &mut ByteReader<'_>) -> ReadResult<SubscriptionFilter> {
    Ok(match reader.varint()? {
        FILTER_NEXT_GROUP_START => SubscriptionFilter::NextGroupStart,
        FILTER_LARGEST_OBJECT => SubscriptionFilter::LargestObject,
        FILTER_ABSOLUTE_START => SubscriptionFilter::AbsoluteStart {
            start: Location::decode(reader)?,
        },
        FILTER_ABSOLUTE_RANGE => {
            let start = Location::decode(reader)?;
            let end_group_delta = reader.varint()?;
            if start.group.checked_add(end_group_delta).is_none() {
                return Err(Error::ProtocolViolation("end group exceeds 64 bits").into());
            }
            SubscriptionFilter::AbsoluteRange {
                start,
                end_group_delta,
            }
        }
        _ => return Err(Error::ProtocolViolation("unknown subscription filter type").into()),
    })
}

/// How long the answer to a request is waited for at the least, in
/// milliseconds, before the parameters of the request are looked at: the
/// time the peer's SETUP gets. **Provisional.**
pub const ANSWER_WAIT_MS: u64 = 10_000;

/// How long the answer to a request with these parameters is waited for,
/// in milliseconds: `base_ms`, and as long again as the request itself
/// tells a relay to wait.
///
/// WHY the two parameters are added: with `RENDEZVOUS_TIMEOUT` "the relay
/// SHOULD hold the subscription and wait for a publisher to appear, up to
/// the specified duration" (§10.2.6), and `FILL_TIMEOUT` is "the maximum
/// total duration in milliseconds a relay SHOULD spend waiting for
/// upstream sources"
/// (§10.2.5). A requester that gave up sooner would give up what it asked
/// for. The draft names no time (§13.6: "Each implementation is expected
/// to set its own timeouts"); `fixtures/request-answer-wait/` has the rows
/// both implementations go by.
#[must_use]
pub fn answer_wait_ms(base_ms: u64, parameters: &[Parameter]) -> u64 {
    let asked = parameters
        .iter()
        .map(|parameter| match parameter {
            Parameter::RendezvousTimeout(ms) | Parameter::FillTimeout(ms) => *ms,
            _ => 0,
        })
        .fold(0u64, u64::saturating_add);
    // No longer than `@xmoq/moqt` can wait.
    base_ms.saturating_add(asked).min(crate::LONGEST_TIMER_MS)
}

/// Appends `Number of Parameters` and the parameters, in ascending type order as
/// §10.2 requires.
pub(crate) fn encode_all(parameters: &[Parameter], out: &mut Vec<u8>) -> Result<(), UsageError> {
    let mut ordered: Vec<&Parameter> = parameters.iter().collect();
    ordered.sort_by_key(|parameter| parameter.ty());
    let repeated = ordered
        .iter()
        .zip(ordered.iter().skip(1))
        .any(|(first, second)| first.ty() == second.ty() && !may_repeat(first.ty()));
    if repeated {
        return Err(UsageError("duplicate parameter"));
    }
    // What this crate would refuse to receive, it does not send.
    if ordered.len() > MAX_PARAMETERS {
        return Err(UsageError("more than 64 parameters"));
    }

    put_count(ordered.len(), out);
    let mut previous_type = 0u64;
    for parameter in ordered {
        // Sorted ascending above, so the difference never goes below zero.
        varint::encode(parameter.ty().saturating_sub(previous_type), out);
        previous_type = parameter.ty();
        match parameter {
            Parameter::ObjectDeliveryTimeout(value)
            | Parameter::RendezvousTimeout(value)
            | Parameter::SubgroupDeliveryTimeout(value)
            | Parameter::Expires(value)
            | Parameter::FillTimeout(value)
            | Parameter::NewGroupRequest(value) => varint::encode(*value, out),
            Parameter::AuthorizationToken(token) => put_length_prefixed(token, out),
            Parameter::LargestObject(location) => location.encode(out),
            Parameter::Forward(forward) => out.push(u8::from(*forward)),
            Parameter::SubscriberPriority(priority) => out.push(*priority),
            Parameter::SubscriptionFilter(filter) => {
                let mut encoded = Vec::new();
                encode_filter(*filter, &mut encoded)?;
                put_length_prefixed(&encoded, out);
            }
            Parameter::GroupOrder(order) => out.push(order.code()),
            Parameter::TrackNamespacePrefix(prefix) => prefix.encode(out),
        }
    }
    Ok(())
}

fn encode_filter(filter: SubscriptionFilter, out: &mut Vec<u8>) -> Result<(), UsageError> {
    match filter {
        SubscriptionFilter::NextGroupStart => varint::encode(FILTER_NEXT_GROUP_START, out),
        SubscriptionFilter::LargestObject => varint::encode(FILTER_LARGEST_OBJECT, out),
        SubscriptionFilter::AbsoluteStart { start } => {
            varint::encode(FILTER_ABSOLUTE_START, out);
            start.encode(out);
        }
        SubscriptionFilter::AbsoluteRange {
            start,
            end_group_delta,
        } => {
            if start.group.checked_add(end_group_delta).is_none() {
                return Err(UsageError("end group exceeds 64 bits"));
            }
            varint::encode(FILTER_ABSOLUTE_RANGE, out);
            start.encode(out);
            varint::encode(end_group_delta, out);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Result<Vec<Parameter>, Error> {
        parse_exact(bytes, decode_all)
    }

    /// `fixtures/request-answer-wait/waits.tsv`, which `@xmoq/moqt` reads too.
    #[test]
    fn an_answer_is_waited_for_as_long_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/request-answer-wait/waits.tsv");
        let mut rows = 0;
        for row in table.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, _request, parameter, value, wait] = fields[..] else {
                panic!("a row of 5 fields: {row}");
            };
            let ms = |field: &str| field.parse::<u64>().expect(field);
            let parameters = match parameter {
                "-" => vec![],
                "RENDEZVOUS_TIMEOUT" => vec![Parameter::RendezvousTimeout(ms(value))],
                "FILL_TIMEOUT" => vec![Parameter::FillTimeout(ms(value))],
                "SUBGROUP_DELIVERY_TIMEOUT" => {
                    vec![Parameter::SubgroupDeliveryTimeout(ms(value))]
                }
                other => panic!("a parameter this test knows: {other}"),
            };
            assert_eq!(
                answer_wait_ms(ANSWER_WAIT_MS, &parameters),
                ms(wait),
                "{name}"
            );
            rows += 1;
        }
        assert_eq!(rows, 10);
    }

    #[test]
    fn the_wait_for_an_answer_begins_from_what_it_is_given() {
        // (the time without a parameter; the parameters; the wait)
        let cases = [
            (1_500, vec![], 1_500),
            (1_500, vec![Parameter::FillTimeout(500)], 2_000),
            (
                0,
                vec![Parameter::RendezvousTimeout(u64::MAX)],
                2_147_483_647,
            ),
        ];
        for (base, parameters, wait) in cases {
            assert_eq!(answer_wait_ms(base, &parameters), wait, "{base}");
        }
    }

    #[test]
    fn decodes_and_encodes_the_same_bytes() {
        let cases: &[(&[u8], &[Parameter])] = &[
            (&[0x00], &[]),
            // One parameter of each remaining type, in ascending order:
            // 0x02, 0x03, 0x03, 0x04, 0x06, 0x08, 0x09, 0x21, 0x32.
            (
                &[
                    0x09, 0x02, 0x25, 0x01, 0x02, 0xab, 0xcd, 0x00, 0x01, 0xef, 0x01, 0x80, 0x80,
                    0x02, 0x07, 0x02, 0x00, 0x01, 0x05, 0x06, 0x18, 0x04, 0x04, 0x02, 0x03, 0x01,
                    0x11, 0x09,
                ],
                &[
                    Parameter::ObjectDeliveryTimeout(37),
                    Parameter::AuthorizationToken(vec![0xab, 0xcd]),
                    Parameter::AuthorizationToken(vec![0xef]),
                    Parameter::RendezvousTimeout(128),
                    Parameter::SubgroupDeliveryTimeout(7),
                    Parameter::Expires(0),
                    Parameter::LargestObject(Location {
                        group: 5,
                        object: 6,
                    }),
                    Parameter::SubscriptionFilter(SubscriptionFilter::AbsoluteRange {
                        start: Location {
                            group: 2,
                            object: 3,
                        },
                        end_group_delta: 1,
                    }),
                    Parameter::NewGroupRequest(9),
                ],
            ),
            (
                &[0x01, 0x21, 0x03, 0x03, 0x02, 0x03],
                &[Parameter::SubscriptionFilter(
                    SubscriptionFilter::AbsoluteStart {
                        start: Location {
                            group: 2,
                            object: 3,
                        },
                    },
                )],
            ),
            (
                &[0x02, 0x21, 0x01, 0x01, 0x01, 0x02],
                &[
                    Parameter::SubscriptionFilter(SubscriptionFilter::NextGroupStart),
                    Parameter::GroupOrder(GroupOrder::Descending),
                ],
            ),
        ];
        for (bytes, parameters) in cases {
            assert_eq!(decode(bytes).as_deref(), Ok(*parameters), "{bytes:02x?}");
            let mut out = Vec::new();
            encode_all(parameters, &mut out).unwrap();
            assert_eq!(out.as_slice(), *bytes, "{parameters:?}");
        }
    }

    /// §10.2.14: the value of `TRACK_NAMESPACE_PREFIX` is a Track Namespace
    /// (§2.4.1), with no length before it: one parameter, Type 0x34, one
    /// field of 4 bytes, `xmoq`. Written from the draft's text; the moqtail
    /// crate 0.15.0 puts a length before it.
    #[test]
    fn a_track_namespace_prefix_is_a_track_namespace() {
        let bytes = [0x01, 0x34, 0x01, 0x04, b'x', b'm', b'o', b'q'];
        let prefix = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        assert_eq!(
            decode(&bytes),
            Ok(vec![Parameter::TrackNamespacePrefix(prefix.clone())])
        );
        let mut out = Vec::new();
        encode_all(&[Parameter::TrackNamespacePrefix(prefix)], &mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn encodes_in_ascending_type_order() {
        let mut out = Vec::new();
        encode_all(
            &[Parameter::SubscriberPriority(128), Parameter::Forward(true)],
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [0x02, 0x10, 0x01, 0x10, 0x80]);
    }

    #[test]
    fn rejects_malformed_input() {
        let cases: &[(&[u8], Error)] = &[
            (&[0x01, 0x01, 0x00], Error::UnknownParameter(0x01)),
            (
                &[0x01, 0x10, 0x02],
                Error::ProtocolViolation("FORWARD is neither 0 nor 1"),
            ),
            (
                &[0x01, 0x22, 0x00],
                Error::ProtocolViolation("GROUP_ORDER is neither 1 nor 2"),
            ),
            (
                &[0x02, 0x10, 0x01, 0x00, 0x01],
                Error::ProtocolViolation("duplicate parameter"),
            ),
            (
                &[0x01, 0x21, 0x01, 0x05],
                Error::ProtocolViolation("unknown subscription filter type"),
            ),
            // LargestObject filter with a trailing byte inside its length.
            (
                &[0x01, 0x21, 0x02, 0x02, 0x00],
                Error::ProtocolViolation("length is longer than the fields it holds"),
            ),
            // AbsoluteRange from group 2^64-1 with delta 1.
            (
                &[
                    0x01, 0x21, 0x0c, 0x04, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                    0x00, 0x01,
                ],
                Error::ProtocolViolation("end group exceeds 64 bits"),
            ),
            // Two parameters announced, one present.
            (
                &[0x02, 0x10, 0x01],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // 2^62 parameters announced, none present: refused by the count,
            // without allocating.
            (
                &[0xff, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
                Error::TooManyParameters { limit: 64 },
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(decode(bytes), Err(error.clone()), "{bytes:02x?}");
        }
    }

    #[test]
    fn forward_defaults_to_true() {
        let cases: &[(&[Parameter], bool)] = &[
            (&[], true),
            (&[Parameter::SubscriberPriority(1)], true),
            (&[Parameter::Forward(true)], true),
            (
                &[Parameter::SubscriberPriority(1), Parameter::Forward(false)],
                false,
            ),
        ];
        for (parameters, expected) in cases {
            assert_eq!(forward(parameters), *expected, "{parameters:?}");
        }
    }

    #[test]
    fn a_subscription_without_the_parameter_is_unfiltered() {
        let start = Location {
            group: 3,
            object: 2,
        };
        let cases: &[(&[Parameter], Option<SubscriptionFilter>)] = &[
            (&[], None),
            (&[Parameter::Forward(false)], None),
            (
                &[Parameter::SubscriptionFilter(
                    SubscriptionFilter::LargestObject,
                )],
                Some(SubscriptionFilter::LargestObject),
            ),
            (
                &[
                    Parameter::SubscriberPriority(1),
                    Parameter::SubscriptionFilter(SubscriptionFilter::AbsoluteStart { start }),
                    Parameter::Forward(true),
                ],
                Some(SubscriptionFilter::AbsoluteStart { start }),
            ),
        ];
        for (parameters, expected) in cases {
            assert_eq!(subscription_filter(parameters), *expected, "{parameters:?}");
        }
    }

    #[test]
    fn refuses_to_encode_a_range_past_the_last_group() {
        let filter = SubscriptionFilter::AbsoluteRange {
            start: Location {
                group: 18_446_744_073_709_551_615,
                object: 0,
            },
            end_group_delta: 1,
        };
        assert_eq!(
            encode_all(&[Parameter::SubscriptionFilter(filter)], &mut Vec::new()),
            Err(UsageError("end group exceeds 64 bits"))
        );
    }

    #[test]
    fn bounds_the_number_of_parameters() {
        // AUTHORIZATION_TOKEN (0x03) with an empty value, 64 times: the first
        // has the type delta 3, the others 0.
        let mut at_the_limit = vec![0x40, 0x03, 0x00];
        at_the_limit.extend([0x00, 0x00].repeat(63));
        assert_eq!(
            parse_exact(&at_the_limit, decode_all).map(|parameters| parameters.len()),
            Ok(64)
        );
        // A count of 65 is refused before any parameter is read.
        assert_eq!(
            parse_exact(&[0x41], decode_all),
            Err(Error::TooManyParameters { limit: 64 })
        );
        let tokens = vec![Parameter::AuthorizationToken(vec![]); 65];
        assert_eq!(
            encode_all(&tokens, &mut Vec::new()),
            Err(UsageError("more than 64 parameters"))
        );
    }

    #[test]
    fn refuses_to_encode_a_repeated_parameter() {
        let mut out = Vec::new();
        assert_eq!(
            encode_all(
                &[Parameter::Forward(true), Parameter::Forward(false)],
                &mut out
            ),
            Err(UsageError("duplicate parameter"))
        );
    }

    #[test]
    fn checks_which_parameters_a_fetch_and_its_answer_may_carry() {
        let cases: &[(Parameter, MessageKind, bool)] = &[
            // §10.2.5, §10.2.7, §10.2.8 and §10.2.2 list FETCH.
            (Parameter::FillTimeout(0), MessageKind::Fetch, true),
            (Parameter::FillTimeout(0), MessageKind::Subscribe, false),
            (Parameter::SubscriberPriority(1), MessageKind::Fetch, true),
            (
                Parameter::GroupOrder(GroupOrder::Descending),
                MessageKind::Fetch,
                true,
            ),
            (
                Parameter::AuthorizationToken(vec![]),
                MessageKind::Fetch,
                true,
            ),
            (Parameter::Forward(true), MessageKind::Fetch, false),
            // The update of a FETCH: what §10.2 lets a REQUEST_UPDATE carry
            // without saying "for a subscription".
            (
                Parameter::SubscriberPriority(1),
                MessageKind::FetchUpdate,
                true,
            ),
            (
                Parameter::AuthorizationToken(vec![]),
                MessageKind::FetchUpdate,
                true,
            ),
            (
                Parameter::ObjectDeliveryTimeout(1),
                MessageKind::FetchUpdate,
                true,
            ),
            (
                Parameter::SubgroupDeliveryTimeout(1),
                MessageKind::FetchUpdate,
                true,
            ),
            (Parameter::Forward(true), MessageKind::FetchUpdate, false),
            (
                Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                MessageKind::FetchUpdate,
                false,
            ),
            (
                Parameter::NewGroupRequest(0),
                MessageKind::FetchUpdate,
                false,
            ),
            (
                Parameter::GroupOrder(GroupOrder::Ascending),
                MessageKind::FetchUpdate,
                false,
            ),
            (Parameter::FillTimeout(0), MessageKind::FetchUpdate, false),
            // No parameter lists FETCH_OK.
            (Parameter::Expires(0), MessageKind::FetchOk, false),
            (
                Parameter::LargestObject(Location {
                    group: 1,
                    object: 0,
                }),
                MessageKind::FetchOk,
                false,
            ),
        ];
        for (parameter, kind, allowed) in cases {
            assert_eq!(
                all_allowed(std::slice::from_ref(parameter), *kind),
                *allowed,
                "{parameter:?} in {kind:?}"
            );
        }
    }

    #[test]
    fn checks_which_message_a_parameter_may_appear_in() {
        let cases: &[(Parameter, MessageKind, bool)] = &[
            (Parameter::Forward(true), MessageKind::Subscribe, true),
            (Parameter::Forward(true), MessageKind::PublishOk, true),
            (Parameter::Forward(true), MessageKind::SubscribeOk, false),
            (
                Parameter::GroupOrder(GroupOrder::Ascending),
                MessageKind::PublishOk,
                true,
            ),
            (
                Parameter::GroupOrder(GroupOrder::Ascending),
                MessageKind::Publish,
                false,
            ),
            (Parameter::Expires(0), MessageKind::SubscribeOk, true),
            (Parameter::Expires(0), MessageKind::Subscribe, false),
            (
                Parameter::AuthorizationToken(vec![]),
                MessageKind::PublishNamespace,
                true,
            ),
            (
                Parameter::Forward(true),
                MessageKind::PublishNamespaceOk,
                false,
            ),
            // What an update of a subscription may change, and what it may not.
            (Parameter::Forward(false), MessageKind::RequestUpdate, true),
            (
                Parameter::SubscriberPriority(1),
                MessageKind::RequestUpdate,
                true,
            ),
            (
                Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                MessageKind::RequestUpdate,
                true,
            ),
            (
                Parameter::ObjectDeliveryTimeout(1),
                MessageKind::RequestUpdate,
                true,
            ),
            (
                Parameter::SubgroupDeliveryTimeout(1),
                MessageKind::RequestUpdate,
                true,
            ),
            (
                Parameter::NewGroupRequest(1),
                MessageKind::RequestUpdate,
                true,
            ),
            (
                Parameter::AuthorizationToken(vec![]),
                MessageKind::RequestUpdate,
                true,
            ),
            (
                Parameter::GroupOrder(GroupOrder::Ascending),
                MessageKind::RequestUpdate,
                false,
            ),
            (Parameter::Expires(0), MessageKind::RequestUpdate, false),
            (
                Parameter::LargestObject(Location {
                    group: 0,
                    object: 0,
                }),
                MessageKind::RequestUpdate,
                false,
            ),
            (
                Parameter::RendezvousTimeout(1),
                MessageKind::RequestUpdate,
                false,
            ),
        ];
        for (parameter, kind, allowed) in cases {
            assert_eq!(
                all_allowed(std::slice::from_ref(parameter), *kind),
                *allowed,
                "{parameter:?} in {kind:?}"
            );
        }
    }
}
