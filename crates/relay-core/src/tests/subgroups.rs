//! Which stream a Subgroup is taken from (§2.2, §11.4, §11.4.2, §11.4.3).

use super::*;

/// A header of Group 3 that gives its Subgroup ID in one of the three ways of
/// §11.4.2.
fn header_with(track_alias: u64, subgroup_id: SubgroupId) -> SubgroupHeader {
    SubgroupHeader {
        subgroup_id,
        ..header(track_alias)
    }
}

/// The publisher opens a stream with exactly this header.
fn stream_opened(relay: &mut Relay, stream: u64, header: SubgroupHeader) -> Vec<Action> {
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(stream),
        header,
    })
}

fn stopped(stream: u64) -> Action {
    Action::StopDataStream {
        session: PUBLISHER,
        stream: StreamKey(stream),
        // CANCELLED.
        code: 0x1,
    }
}

/// The stream is read for the check of §2.4.2 alone.
fn checked(stream: u64) -> Action {
    Action::StartCheckTimer {
        session: PUBLISHER,
        stream: StreamKey(stream),
        wait_ms: crate::CHECKED_STREAM_WAIT_MS,
    }
}

fn reset(out_stream: u64, code: u32) -> Action {
    Action::ResetDataStream {
        out_stream: OutStreamKey(out_stream),
        code,
    }
}

fn finished(out_stream: u64) -> Action {
    Action::FinishDataStream {
        out_stream: OutStreamKey(out_stream),
    }
}

#[test]
fn a_stream_is_opened_downstream_once_its_subgroup_is_known() {
    // (how the header gives the Subgroup ID, what the relay does when the
    //  stream opens, and when its first Object, Object 4, comes)
    let cases = [
        (
            SubgroupId::Zero,
            vec![open_data_stream(ALICE, 0, header_with(0, SubgroupId::Zero))],
            vec![send_object(0, 4)],
        ),
        (
            SubgroupId::Explicit(5),
            vec![open_data_stream(
                ALICE,
                0,
                header_with(0, SubgroupId::Explicit(5)),
            )],
            vec![send_object(0, 4)],
        ),
        // The Subgroup ID is the ID of the first Object: until that comes,
        // the relay cannot tell which Subgroup the stream is of.
        (
            SubgroupId::FirstObjectId,
            vec![],
            vec![
                open_data_stream(ALICE, 0, header_with(0, SubgroupId::FirstObjectId)),
                send_object(0, 4),
            ],
        ),
    ];
    for (subgroup_id, at_the_header, at_the_object) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        let header = header_with(UPSTREAM_ALIAS, subgroup_id);
        assert_eq!(
            stream_opened(&mut relay, 7, header),
            at_the_header,
            "{subgroup_id:?}"
        );
        assert_eq!(
            object_arrived(&mut relay, 7, 4),
            at_the_object,
            "{subgroup_id:?}"
        );
    }
}

#[test]
fn a_stream_that_ends_without_an_object() {
    // (how the header gives the Subgroup ID, how the stream ends, what the
    //  relay does when it ends, the streams Alice was sent in all)
    let cases = [
        // §11.4.3: a header and a reset say that there is a Subgroup the
        // source will not send. The subscriber is told the same. The code is
        // the publisher's, DELIVERY_TIMEOUT (0x2).
        (
            SubgroupId::Zero,
            StreamEnd::Aborted(Some(0x2)),
            vec![reset(0, 0x2)],
            1,
        ),
        // A header and a FIN: §11.4 has a stream hold "one or more" Objects
        // and says nothing of what a receiver does with none. Passed on as it
        // came.
        (SubgroupId::Zero, StreamEnd::Fin, vec![finished(0)], 1),
        (
            SubgroupId::Explicit(5),
            StreamEnd::Fin,
            vec![finished(0)],
            1,
        ),
        // Which Subgroup it was is never known: nothing was opened, and
        // nothing is.
        (SubgroupId::FirstObjectId, StreamEnd::Fin, vec![], 0),
        (
            SubgroupId::FirstObjectId,
            StreamEnd::Aborted(Some(0x2)),
            vec![],
            0,
        ),
    ];
    for (subgroup_id, end, at_the_end, streams) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        stream_opened(&mut relay, 7, header_with(UPSTREAM_ALIAS, subgroup_id));
        assert_eq!(
            data_stream_closed(&mut relay, 7, end),
            at_the_end,
            "{subgroup_id:?} {end:?}"
        );
        assert_eq!(relay.counts().in_streams, 0, "{subgroup_id:?} {end:?}");
        // The publisher leaves, and PUBLISH_DONE counts what Alice was sent.
        assert_eq!(
            relay.step(Event::SessionClosed { session: PUBLISHER }),
            done_actions(ALICE, 0x2, streams),
            "{subgroup_id:?} {end:?}"
        );
    }
}

#[test]
fn a_subgroup_whose_stream_ended_with_a_fin_and_no_object_counts_as_delivered_whole() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    stream_opened(&mut relay, 7, header(UPSTREAM_ALIAS));
    assert_eq!(
        data_stream_closed(&mut relay, 7, StreamEnd::Fin),
        [finished(0)]
    );
    // A second stream for Subgroup 0 is one after a Subgroup delivered
    // whole: read for the check alone.
    assert_eq!(
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
        [checked(8)]
    );
}

/// What happens to the first stream of Subgroup 0 of Group 3, stream 7,
/// before the same publisher opens a second one for the Subgroup, stream 8.
#[derive(Debug, Clone, Copy)]
enum First {
    /// Reset by the publisher, with `DELIVERY_TIMEOUT` (0x2).
    CutShort,
    StillOpen,
    Finished,
}

#[test]
fn a_second_stream_of_a_subgroup_from_the_same_publisher() {
    // (the first stream; what the relay does when the second one opens; when
    //  Object 1 comes on the second; when Object 1 comes on the first; the
    //  streams Alice was sent in all)
    let cases = [
        // §2.2 allows it by name: the publisher sends the Subgroup again.
        (
            First::CutShort,
            vec![open_data_stream(ALICE, 1, header(0))],
            vec![send_object(1, 1)],
            vec![],
            2,
        ),
        // The new one takes the place of the old one: the publisher may have
        // reset the old one already, and the reset may be read later. Alice's
        // stream for the old one is reset (CANCELLED, 0x1) before she gets
        // one for the new, so that she never has two for the Subgroup.
        (
            First::StillOpen,
            vec![
                reset(0, 0x1),
                stopped(7),
                open_data_stream(ALICE, 1, header(0)),
            ],
            vec![send_object(1, 1)],
            vec![],
            2,
        ),
        // The Subgroup was delivered whole: the second stream is read for
        // the check alone, and nothing of it reaches Alice (§2.2). Object 1
        // on it is past the final Object, Object 0, and ends the track
        // (`a_second_stream_of_a_subgroup_delivered_whole_is_read_for_the_check`);
        // here Object 0 comes again.
        (First::Finished, vec![checked(8)], vec![], vec![], 1),
    ];
    for (first, at_the_second, object_on_the_second, object_on_the_first, streams) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        match first {
            First::CutShort => {
                data_stream_closed(&mut relay, 7, StreamEnd::Aborted(Some(0x2)));
            }
            First::StillOpen => {}
            First::Finished => {
                data_stream_closed(&mut relay, 7, StreamEnd::Fin);
            }
        }
        assert_eq!(
            stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
            at_the_second,
            "{first:?}"
        );
        let again = u64::from(!matches!(first, First::Finished));
        assert_eq!(
            object_arrived(&mut relay, 8, again),
            object_on_the_second,
            "{first:?}"
        );
        assert_eq!(
            object_arrived(&mut relay, 7, 1),
            object_on_the_first,
            "{first:?}"
        );
        // Whatever is still open ends, and nothing more is said about a
        // stream that was left.
        for stream in [7, 8] {
            data_stream_closed(&mut relay, stream, StreamEnd::Aborted(None));
        }
        assert_eq!(relay.counts().in_streams, 0, "{first:?}");
        assert_eq!(
            relay.step(Event::SessionClosed { session: PUBLISHER }),
            done_actions(ALICE, 0x2, streams),
            "{first:?}"
        );
    }
}

#[test]
fn a_second_stream_is_recognised_when_its_first_object_names_the_subgroup() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // Stream 7 is of Subgroup 0, by its header.
    data_stream_opened(&mut relay, 7);
    // Stream 8 leaves the Subgroup ID to its first Object.
    let header = header_with(UPSTREAM_ALIAS, SubgroupId::FirstObjectId);
    assert_eq!(stream_opened(&mut relay, 8, header), []);
    // Object 0 comes first on it: it is of Subgroup 0 as well, and takes the
    // place of stream 7.
    assert_eq!(
        object_arrived(&mut relay, 8, 0),
        [
            reset(0, 0x1),
            stopped(7),
            open_data_stream(ALICE, 1, header_with(0, SubgroupId::FirstObjectId)),
            send_object(1, 0),
        ]
    );
}

#[test]
fn a_track_that_ends_does_not_stop_a_stream_it_has_stopped_already() {
    let mut relay = relay_with_track();
    data_stream_opened(&mut relay, 7);
    data_stream_closed(&mut relay, 7, StreamEnd::Fin);
    // Read for the check, until its time is over; then stopped.
    assert_eq!(
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
        [checked(8)]
    );
    assert_eq!(
        relay.step(Event::CheckStreamTimeout {
            session: PUBLISHER,
            stream: StreamKey(8),
        }),
        [stopped(8)]
    );
    data_stream_opened(&mut relay, 9);
    // The track is malformed (0x12): stream 9 is stopped, stream 8 not again.
    let actions = relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(9),
    });
    let stops: Vec<&Action> = actions
        .iter()
        .filter(|action| matches!(action, Action::StopDataStream { .. }))
        .collect();
    assert_eq!(
        stops,
        [&Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(9),
            code: 0x12,
        }]
    );
}

#[test]
fn subgroups_are_remembered_for_the_newest_groups_only() {
    // The expectations below are spelled out for 8 Groups.
    assert_eq!(crate::MAX_REMEMBERED_GROUPS, 8);
    // (the newest Group seen since Group 0 was delivered whole; what the
    //  relay does with a second stream for Group 0)
    let cases = [
        // Seven Groups on: remembered, so the stream is read for the check
        // alone.
        (7, vec![checked(100)]),
        // Eight Groups on: forgotten. The stream passes, as it has to for a
        // publisher that sends an old Group late.
        (8, vec![open_data_stream(ALICE, 9, header_of_group(0, 0))]),
    ];
    for (newest, second_stream) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        for group in 0..=newest {
            stream_opened(
                &mut relay,
                20 + group,
                header_of_group(UPSTREAM_ALIAS, group),
            );
            data_stream_closed(&mut relay, 20 + group, StreamEnd::Fin);
        }
        assert_eq!(
            stream_opened(&mut relay, 100, header_of_group(UPSTREAM_ALIAS, 0)),
            second_stream,
            "{newest}"
        );
    }
}

#[test]
fn only_so_many_subgroups_are_remembered() {
    // The expectations below are spelled out for 256 Subgroups.
    assert_eq!(crate::MAX_REMEMBERED_SUBGROUPS, 256);
    // (Subgroups of Group 3 that were delivered whole, numbered from 0; the
    //  Subgroup a second stream comes for; whether that stream is left)
    let cases = [
        // All 256 are remembered.
        (256, 0, true),
        // With one more, the one with the smallest ID is forgotten.
        (257, 0, false),
        (257, 1, true),
    ];
    for (delivered, again, left) in cases {
        let mut relay = relay_with_track();
        for subgroup in 0..delivered {
            let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(subgroup));
            stream_opened(&mut relay, 1000 + subgroup, header);
            data_stream_closed(&mut relay, 1000 + subgroup, StreamEnd::Fin);
        }
        // Never more than the limit, however many were delivered.
        assert_eq!(
            relay.counts().remembered_subgroups,
            256,
            "{delivered} {again}"
        );
        let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(again));
        let expected = if left { vec![checked(5000)] } else { vec![] };
        assert_eq!(
            stream_opened(&mut relay, 5000, header),
            expected,
            "{delivered} {again}"
        );
    }
}

#[test]
fn a_publisher_that_keeps_opening_streams_for_one_subgroup_costs_one_stream_at_a_time() {
    // Fifty streams for Subgroup 0 of Group 3, none of them closed. Each
    // takes the place of the one before.
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    subscribe_to_track(&mut relay, BOB);
    for stream in 0..50 {
        stream_opened(&mut relay, 100 + stream, header(UPSTREAM_ALIAS));
    }
    let counts = relay.counts();
    // One stream to each subscriber is open at any time. The publisher's
    // fifty stay in the books until they are reported closed; how many it
    // can have open at once is the limit of the I/O side.
    assert_eq!((counts.out_streams, counts.in_streams), (2, 50));
    // Each subscriber was sent fifty streams, which its PUBLISH_DONE counts
    // (§10.11).
    let mut expected = vec![reset(98, 0x1), reset(99, 0x1)];
    expected.extend(done_actions(ALICE, 0x2, 50));
    expected.extend(done_actions(BOB, 0x2, 50));
    assert_eq!(
        relay.step(Event::SessionClosed { session: PUBLISHER }),
        expected
    );
    // Nothing of the fifty streams, or of what was remembered of their
    // Subgroup, outlives the track.
    let counts = relay.counts();
    assert_eq!(
        (
            counts.in_streams,
            counts.out_streams,
            counts.remembered_subgroups,
            counts.tracks
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn a_first_object_that_names_a_subgroup_that_was_delivered_whole_is_left() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // Subgroup 0 of Group 3 comes whole on stream 7.
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    data_stream_closed(&mut relay, 7, StreamEnd::Fin);
    // Stream 8 leaves the Subgroup ID to its first Object, which is Object 0:
    // only then is it known to be of that Subgroup, and it is read for the
    // check alone. It ends at the same final Object, and nothing of it
    // reaches Alice.
    let header = header_with(UPSTREAM_ALIAS, SubgroupId::FirstObjectId);
    assert_eq!(stream_opened(&mut relay, 8, header), []);
    assert_eq!(object_arrived(&mut relay, 8, 0), [checked(8)]);
    assert_eq!(data_stream_closed(&mut relay, 8, StreamEnd::Fin), []);
    assert_eq!(relay.counts().in_streams, 0);
    assert_eq!(relay.counts().tracks, 1);
}

#[test]
fn a_stream_that_was_held_for_its_alias_starts_out_like_any_other() {
    // The stream and its Object come before the PUBLISH that names the alias
    // (§10.10); the header leaves the Subgroup ID to the first Object.
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    let held = header_with(UPSTREAM_ALIAS, SubgroupId::FirstObjectId);
    stream_opened(&mut relay, 7, held);
    object_arrived(&mut relay, 7, 4);
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: publish(0, b"a"),
    });
    // Nobody subscribed when the held stream was fed back in. Alice comes
    // now, inside the Subgroup, and the next Object reaches her on a stream
    // of her own that names Subgroup 4.
    subscribe_to_track(&mut relay, ALICE);
    assert_eq!(
        object_arrived(&mut relay, 7, 5),
        [
            open_data_stream(
                ALICE,
                0,
                SubgroupHeader {
                    subgroup_id: SubgroupId::Explicit(4),
                    first_object: false,
                    ..header(0)
                }
            ),
            send_object(0, 5),
        ]
    );
    assert_eq!(relay.counts().remembered_subgroups, 1);
}

#[test]
fn a_subscriber_that_joins_inside_a_subgroup_is_told_which_subgroup_it_is() {
    // (how the publisher's header gives the Subgroup ID; the header of the
    //  stream Bob gets when he joins after Object 4). Bob's header names the
    //  Subgroup ID and does not claim FIRST_OBJECT, whatever the publisher's
    //  header looked like (§11.4.2).
    let cases = [
        (SubgroupId::Zero, SubgroupId::Explicit(0)),
        (SubgroupId::Explicit(5), SubgroupId::Explicit(5)),
        (SubgroupId::FirstObjectId, SubgroupId::Explicit(4)),
    ];
    for (from_publisher, to_bob) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        stream_opened(&mut relay, 7, header_with(UPSTREAM_ALIAS, from_publisher));
        object_arrived(&mut relay, 7, 4);
        subscribe_to_track(&mut relay, BOB);
        assert_eq!(
            object_arrived(&mut relay, 7, 6),
            [
                open_data_stream(
                    BOB,
                    1,
                    SubgroupHeader {
                        subgroup_id: to_bob,
                        first_object: false,
                        ..header(0)
                    }
                ),
                send_object(0, 6),
                send_object(1, 6),
            ],
            "{from_publisher:?}"
        );
    }
}

#[test]
fn a_stream_that_takes_the_place_of_another_does_so_for_every_subscriber() {
    let mut relay = relay_with_track();
    // Alice is there from the start of the Subgroup; Bob joins inside it.
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    subscribe_to_track(&mut relay, BOB);
    object_arrived(&mut relay, 7, 1);
    // Both have a stream for the Subgroup now: 0 and 1. A second stream of
    // the publisher for it resets both, in the order the two subscribed, and
    // gives each a new one from its start.
    assert_eq!(
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
        [
            reset(0, 0x1),
            reset(1, 0x1),
            stopped(7),
            open_data_stream(ALICE, 2, header(0)),
            open_data_stream(BOB, 3, header(0)),
        ]
    );
    assert_eq!(
        object_arrived(&mut relay, 8, 0),
        [send_object(2, 0), send_object(3, 0)]
    );
}

#[test]
fn a_stream_that_takes_the_place_of_another_starts_where_the_subscription_does() {
    let mut relay = relay_with_track();
    // Alice gets everything; Bob's subscription starts at {3, 1}.
    subscribe_to_track(&mut relay, ALICE);
    super::filters::subscribe_with(&mut relay, Some(super::filters::from(3, 1)));
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    object_arrived(&mut relay, 7, 1);
    // Alice has stream 0 and Bob stream 1, from Object 1 on. A second
    // stream of the publisher for the Subgroup resets both. Alice gets a
    // new one with the header; Bob gets his with the first Object that
    // passes again, and not with Object 0.
    assert_eq!(
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
        [
            reset(0, 0x1),
            reset(1, 0x1),
            stopped(7),
            open_data_stream(ALICE, 2, header(0)),
        ]
    );
    assert_eq!(object_arrived(&mut relay, 8, 0), [send_object(2, 0)]);
    assert_eq!(
        object_arrived(&mut relay, 8, 1),
        [
            open_data_stream(
                BOB,
                3,
                SubgroupHeader {
                    subgroup_id: SubgroupId::Explicit(0),
                    first_object: false,
                    ..header(0)
                }
            ),
            send_object(2, 1),
            send_object(3, 1),
        ]
    );
}

#[test]
fn a_stream_that_took_the_place_of_another_and_ends_without_an_object() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS));
    // The new stream is finished with nothing on it. Alice's stream for it
    // is finished the same way, and the Subgroup counts as delivered whole:
    // a third stream for it is read for the check alone.
    assert_eq!(
        data_stream_closed(&mut relay, 8, StreamEnd::Fin),
        [finished(1)]
    );
    assert_eq!(
        stream_opened(&mut relay, 9, header(UPSTREAM_ALIAS)),
        [checked(9)]
    );
}

#[test]
fn a_stream_whose_subgroup_was_forgotten_while_it_was_open() {
    // The expectations below are spelled out for 8 Groups.
    assert_eq!(crate::MAX_REMEMBERED_GROUPS, 8);
    // (what happens after the relay forgot the Subgroup of stream 20, which
    //  is still open; what the relay does)
    let cases = [
        // It goes on, and ends as it would have.
        ("an Object and the FIN", false),
        // A second stream for the Subgroup: there is nothing to compare it
        // with, so it passes, and Alice has two streams for the Subgroup.
        ("a second stream", true),
    ];
    for (name, second) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        // Stream 20 is of Group 0 and stays open; Groups 1 to 8 pass.
        stream_opened(&mut relay, 20, header_of_group(UPSTREAM_ALIAS, 0));
        for group in 1..=8 {
            stream_opened(
                &mut relay,
                20 + group,
                header_of_group(UPSTREAM_ALIAS, group),
            );
            data_stream_closed(&mut relay, 20 + group, StreamEnd::Fin);
        }
        // Eight Groups are remembered, 1 to 8; Group 0 is not among them.
        assert_eq!(relay.counts().remembered_subgroups, 8, "{name}");
        if second {
            assert_eq!(
                stream_opened(&mut relay, 100, header_of_group(UPSTREAM_ALIAS, 0)),
                [open_data_stream(ALICE, 9, header_of_group(0, 0))],
                "{name}"
            );
        }
        assert_eq!(
            object_arrived(&mut relay, 20, 0),
            [send_object(0, 0)],
            "{name}"
        );
        assert_eq!(
            data_stream_closed(&mut relay, 20, StreamEnd::Fin),
            [finished(0)],
            "{name}"
        );
        assert_eq!(relay.counts().remembered_subgroups, 8, "{name}");
    }
}

#[test]
fn a_stream_that_takes_the_place_of_another_while_an_old_subgroup_is_forgotten() {
    // The expectation below is spelled out for 256 Subgroups.
    assert_eq!(crate::MAX_REMEMBERED_SUBGROUPS, 256);
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // Subgroups 0 to 255 of Group 3 are open, on streams 1000 to 1255: the
    // table is full.
    for subgroup in 0..256 {
        let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(subgroup));
        stream_opened(&mut relay, 1000 + subgroup, header);
    }
    assert_eq!(relay.counts().remembered_subgroups, 256);
    // A second stream for Subgroup 255 takes the place of stream 1255, and
    // the table holds no more than before: one event that replaces and has
    // nothing to forget.
    let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(255));
    assert_eq!(
        stream_opened(&mut relay, 2000, header),
        [
            reset(255, 0x1),
            stopped(1255),
            open_data_stream(ALICE, 256, header_with(0, SubgroupId::Explicit(255))),
        ]
    );
    assert_eq!(relay.counts().remembered_subgroups, 256);
    // A stream for a new Subgroup, 256, makes it 257: Subgroup 0 is
    // forgotten, and a second stream for Subgroup 0 passes next to the first.
    let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(256));
    stream_opened(&mut relay, 2001, header);
    assert_eq!(relay.counts().remembered_subgroups, 256);
    let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(0));
    assert_eq!(
        stream_opened(&mut relay, 2002, header),
        [open_data_stream(
            ALICE,
            258,
            header_with(0, SubgroupId::Explicit(0))
        )]
    );
}

#[test]
fn the_publishers_stream_count_has_the_streams_the_relay_did_not_pass_on() {
    // (the Stream Count of the publisher's PUBLISH_DONE; tracks left after
    //  it). The publisher sent three streams: one that was passed on, one the
    //  relay read for the check alone, and one that named no Subgroup. §10.11 counts all
    //  three ("including streams that contained no Objects"), so with 3 the
    //  relay waits for nothing, and with 4 it waits for one more.
    let cases = [(3, 0), (4, 1)];
    for (stream_count, tracks_left) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        data_stream_closed(&mut relay, 7, StreamEnd::Fin);
        assert_eq!(
            stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
            [checked(8)]
        );
        data_stream_closed(&mut relay, 8, StreamEnd::Aborted(None));
        let unnamed = header_with(UPSTREAM_ALIAS, SubgroupId::FirstObjectId);
        stream_opened(&mut relay, 9, unnamed);
        data_stream_closed(&mut relay, 9, StreamEnd::Fin);
        relay.step(Event::RequestMessage {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: publish_done(0x2, stream_count),
        });
        assert_eq!(relay.counts().tracks, tracks_left, "{stream_count}");
    }
}

#[test]
fn a_subscriber_that_comes_between_the_header_and_the_first_object() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    let unnamed = header_with(UPSTREAM_ALIAS, SubgroupId::FirstObjectId);
    assert_eq!(stream_opened(&mut relay, 7, unnamed), []);
    // Bob comes before the Object that names the Subgroup. Nothing was sent
    // of it yet, so he gets what Alice gets: the header as it came, and the
    // first Object first, from which both derive Subgroup 4.
    subscribe_to_track(&mut relay, BOB);
    assert_eq!(
        object_arrived(&mut relay, 7, 4),
        [
            open_data_stream(ALICE, 0, header_with(0, SubgroupId::FirstObjectId)),
            open_data_stream(BOB, 1, header_with(0, SubgroupId::FirstObjectId)),
            send_object(0, 4),
            send_object(1, 4),
        ]
    );
}

#[test]
fn a_subscriber_whose_stream_is_closed_gets_another_for_the_same_subgroup() {
    // Closed without a STOP_SENDING of the subscriber; one it stopped is
    // `a_stream_the_subscriber_stopped_is_not_opened_again`.
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    relay.step(Event::OutStreamClosed {
        out_stream: OutStreamKey(0),
    });
    assert_eq!(
        object_arrived(&mut relay, 7, 1),
        [
            open_data_stream(
                ALICE,
                1,
                SubgroupHeader {
                    subgroup_id: SubgroupId::Explicit(0),
                    first_object: false,
                    ..header(0)
                }
            ),
            send_object(1, 1),
        ]
    );
}

#[test]
fn known_gap_a_subgroup_a_subscriber_stopped_reaches_it_again_on_another_stream_of_its_source() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    relay.step(Event::OutStreamStopped {
        out_stream: OutStreamKey(0),
    });
    relay.step(Event::OutStreamClosed {
        out_stream: OutStreamKey(0),
    });
    // The publisher sends the Subgroup again on stream 8, which takes the
    // place of stream 7: Alice gets the Subgroup again, where §11.4.3 has
    // "SHOULD NOT attempt to open a new stream to deliver additional
    // Objects in that Subgroup".
    assert_eq!(
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
        [stopped(7), open_data_stream(ALICE, 1, header(0))]
    );
}

#[test]
fn known_gap_the_stream_whose_header_is_read_later_is_kept_though_it_was_reset_first() {
    // The publisher reset stream 7 and sent the Subgroup again on stream 8,
    // and the relay reads the header of 8 first: QUIC keeps no order
    // between streams.
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    assert_eq!(
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
        [open_data_stream(ALICE, 0, header(0))]
    );
    object_arrived(&mut relay, 8, 0);
    // Stream 7 takes the place of stream 8, which the publisher goes on with.
    assert_eq!(
        stream_opened(&mut relay, 7, header(UPSTREAM_ALIAS)),
        [
            reset(0, 0x1),
            stopped(8),
            open_data_stream(ALICE, 1, header(0))
        ]
    );
    // Stream 7 ends with its reset, and so does Alice's. A publisher that
    // was sent STOP_SENDING for stream 8 "SHOULD NOT attempt to open a new
    // stream" for the Subgroup (§11.4.3): it is lost for every subscriber.
    assert_eq!(
        data_stream_closed(&mut relay, 7, StreamEnd::Aborted(Some(0x2))),
        [reset(1, 0x2)]
    );
    // What still comes on stream 8 is dropped: no subscriber gets an Object
    // it should not.
    assert_eq!(object_arrived(&mut relay, 8, 1), []);
}

/// What a second stream of the publisher carries for Subgroup 0 of Group 3,
/// which it delivered whole before.
#[derive(Debug, Clone, Copy)]
enum Second {
    Object(u64),
    Fin,
    Reset,
}

/// A Subgroup that ends twice, or goes past its end (relay).
#[test]
fn a_second_stream_of_a_subgroup_delivered_whole_is_read_for_the_check() {
    use Second::{Fin, Object, Reset};
    // (the Objects of the first stream, which ends with a FIN; what comes on
    //  the second; whether the track is malformed after it, §2.4.2)
    let cases: [(&str, &[u64], &[Second], bool); 7] = [
        (
            "the same Objects again",
            &[0, 1],
            &[Object(0), Object(1), Fin],
            false,
        ),
        (
            "an Object past the final one (the second example)",
            &[0, 1],
            &[Object(0), Object(1), Object(2)],
            true,
        ),
        (
            "a FIN after another last Object (the third example)",
            &[0, 1],
            &[Object(0), Fin],
            true,
        ),
        (
            "cut short before the final Object",
            &[0, 1],
            &[Object(0), Reset],
            false,
        ),
        ("a FIN with no Object on it", &[0, 1], &[Fin], false),
        (
            "a first stream with no Object has no final Object",
            &[],
            &[Object(0), Object(5), Fin],
            false,
        ),
        (
            "a smaller last Object",
            &[0, 1, 2],
            &[Object(0), Object(1), Fin],
            true,
        ),
    ];
    for (name, first, second, malformed) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        for object in first {
            object_arrived(&mut relay, 7, *object);
        }
        data_stream_closed(&mut relay, 7, StreamEnd::Fin);
        assert_eq!(
            stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS)),
            [checked(8)],
            "{name}"
        );
        let mut actions = Vec::new();
        for step in second {
            actions.extend(match *step {
                Object(object) => object_arrived(&mut relay, 8, object),
                Fin => data_stream_closed(&mut relay, 8, StreamEnd::Fin),
                Reset => data_stream_closed(&mut relay, 8, StreamEnd::Aborted(Some(0x2))),
            });
        }
        // Nothing of the second stream goes to Alice.
        let towards_alice = actions.iter().any(|action| {
            matches!(
                action,
                Action::SendObject { .. } | Action::OpenDataStream { .. }
            )
        });
        assert!(!towards_alice, "{name}: {actions:?}");
        assert_eq!(relay.counts().tracks, usize::from(!malformed), "{name}");
        if malformed {
            assert!(
                actions.contains(&done_actions(ALICE, 0x12, 1)[0]),
                "{name}: {actions:?}"
            );
        }
    }
}

#[test]
fn an_object_past_the_final_one_stops_its_stream_with_malformed_track() {
    let mut relay = relay_with_track();
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    data_stream_closed(&mut relay, 7, StreamEnd::Fin);
    stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS));
    let actions = object_arrived(&mut relay, 8, 1);
    assert_eq!(
        actions.first(),
        Some(&Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(8),
            code: 0x12,
        })
    );
    assert_eq!(relay.counts().tracks, 0);
}

#[test]
fn only_so_many_streams_of_a_track_are_read_for_the_check_at_once() {
    // The expectations below are spelled out for 4 streams.
    assert_eq!(crate::MAX_CHECKED_STREAMS_PER_TRACK, 4);
    let mut relay = relay_with_track();
    for subgroup in 0..5 {
        let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(subgroup));
        stream_opened(&mut relay, 10 + subgroup, header);
        data_stream_closed(&mut relay, 10 + subgroup, StreamEnd::Fin);
    }
    let again: Vec<Vec<Action>> = (0..5)
        .map(|subgroup| {
            let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(subgroup));
            stream_opened(&mut relay, 20 + subgroup, header)
        })
        .collect();
    assert_eq!(
        again,
        [
            vec![checked(20)],
            vec![checked(21)],
            vec![checked(22)],
            vec![checked(23)],
            vec![stopped(24)],
        ]
    );
    // One that ends makes room for the next.
    data_stream_closed(&mut relay, 20, StreamEnd::Aborted(None));
    let header = header_with(UPSTREAM_ALIAS, SubgroupId::Explicit(0));
    assert_eq!(stream_opened(&mut relay, 30, header), [checked(30)]);
}

#[test]
fn a_stream_read_for_the_check_is_stopped_when_its_time_is_over() {
    let mut relay = relay_with_track();
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    data_stream_closed(&mut relay, 7, StreamEnd::Fin);
    stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS));
    let timeout = Event::CheckStreamTimeout {
        session: PUBLISHER,
        stream: StreamKey(8),
    };
    assert_eq!(relay.step(timeout.clone()), [stopped(8)]);
    // What still arrives on it is not looked at, and a second timeout of
    // the same stream does nothing.
    assert_eq!(object_arrived(&mut relay, 8, 1), []);
    assert_eq!(relay.step(timeout), []);
    assert_eq!(relay.counts().tracks, 1);
}

/// A Subgroup that ends twice, or goes past its end (relay), where the final
/// Object is a status.
#[test]
fn a_status_is_the_final_object_it_ends_on() {
    let end_of_group = SubgroupObject {
        status: ObjectStatus::EndOfGroup,
        payload: vec![],
        ..object(2)
    };
    // (what the second stream carries before its FIN; whether the track is
    //  malformed). The first stream has Object 0 and an End of Group at 2.
    let cases: [(&[u64], bool); 2] = [(&[0], true), (&[0, 2], false)];
    for (second, malformed) in cases {
        let mut relay = relay_with_track();
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        relay.step(Event::Object {
            session: PUBLISHER,
            stream: StreamKey(7),
            object: end_of_group.clone(),
        });
        data_stream_closed(&mut relay, 7, StreamEnd::Fin);
        stream_opened(&mut relay, 8, header(UPSTREAM_ALIAS));
        for object_id in second {
            let object = if *object_id == 2 {
                end_of_group.clone()
            } else {
                object(*object_id)
            };
            relay.step(Event::Object {
                session: PUBLISHER,
                stream: StreamKey(8),
                object,
            });
        }
        data_stream_closed(&mut relay, 8, StreamEnd::Fin);
        assert_eq!(relay.counts().tracks, usize::from(!malformed), "{second:?}");
    }
}
