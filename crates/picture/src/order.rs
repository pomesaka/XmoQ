//! Which frames the decoder can be given: those that follow the one before,
//! and the first of a Group.

/// The Object the decoder can take next.
#[derive(Debug, Default)]
pub struct InOrder {
    next: Option<(u64, u64)>,
}

impl InOrder {
    /// Whether the frame of Object `object` of Group `group` can be decoded,
    /// after the frames this was told of. One that can is the last it was
    /// told of; one that cannot leaves it as it was.
    ///
    /// WHY the first of a Group: the publishers this runs with
    /// (`@xmoq/media`'s, `moqlivemock`, `fixtures/loc-04/`) begin every
    /// Group with a key frame. WHY not that of an older Group: it would take the picture
    /// back. WHY a frame that does not follow is left out: the decoder would
    /// make a picture of it from the wrong one before.
    pub fn take(&mut self, group: u64, object: u64) -> bool {
        let follows = self.next == Some((group, object));
        let begins = object == 0 && self.next.is_none_or(|(next, _)| group >= next);
        if !follows && !begins {
            return false;
        }
        self.next = Some((group, object.saturating_add(1)));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_decoded_where_it_follows_or_begins_a_group() {
        // (the frames, as Group and Object, in the order they came; whether
        // each is decoded)
        type Case = (&'static [(u64, u64)], &'static [bool]);
        let cases: [Case; 7] = [
            (&[(5, 0), (5, 1), (5, 2)], &[true, true, true]),
            // Nothing before the first key frame.
            (
                &[(5, 1), (5, 2), (6, 0), (6, 1)],
                &[false, false, true, true],
            ),
            // A frame that did not come: the rest of its Group is left out.
            (
                &[(5, 0), (5, 2), (5, 3), (6, 0)],
                &[true, false, false, true],
            ),
            // A Group that begins before the last ended.
            (
                &[(5, 0), (5, 1), (6, 0), (5, 2)],
                &[true, true, true, false],
            ),
            // The first of an older Group, late.
            (&[(6, 0), (5, 0)], &[true, false]),
            // The same Group again from its beginning (a stream sent again).
            (&[(5, 0), (5, 1), (5, 0), (5, 1)], &[true, true, true, true]),
            (&[(u64::MAX, u64::MAX)], &[false]),
        ];
        for (frames, expected) in cases {
            let mut order = InOrder::default();
            let taken: Vec<bool> = frames
                .iter()
                .map(|(group, object)| order.take(*group, *object))
                .collect();
            assert_eq!(taken, expected, "{frames:?}");
        }
    }
}
