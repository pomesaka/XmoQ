//! What the transcriber decides of each broadcast it listens to, without
//! input or output: whose namespace it takes, which audio track of its
//! catalog it listens to, and what becomes of its catalog and its audio
//! track after each thing that happens to them (`kept::decide` of
//! `xmoq-subscriber`, as for a track a viewer keeps). The session sends the
//! requests, by the tokens of their tasks, and reads the sound.

use xmoq_moqt::name::TrackNamespace;
use xmoq_msf::NoCatalog;
use xmoq_subscriber::kept::{Becomes, Happened, Keeping, Tries, decide};

/// A track of a broadcast that the transcriber subscribes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BroadcastTrack {
    /// Its catalog (MSF §5).
    Catalog,
    /// The audio track its catalog names.
    Audio,
}

impl BroadcastTrack {
    /// The word of the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Audio => "audio",
        }
    }
}

/// How `track` of a broadcast is kept: asked for again whatever ended it,
/// as long as the broadcast stays announced, the catalog up to `times` in a
/// row (`--ask-again-times`), after which the broadcast is listened to no
/// more until its NAMESPACE comes again.
///
/// WHY the catalog asked again, and WHY NOT the broadcast let go of at
/// once: a NAMESPACE comes again only "when there are changes to the
/// namespaces being published" (§10.18), so a broadcast whose catalog was
/// refused before its publisher had it (a relay without a rendezvous) would
/// not be heard until the publisher announced again. WHY a number of times
/// for the catalog: a relay is not asked for ever, every second, for the
/// catalogs of broadcasts that keep sending a malformed one. WHY none for
/// the audio track: a broadcast without sound names no audio track in its
/// catalog, and is not asked; one that names it is meant to be heard, and
/// is let go of with its catalog.
#[must_use]
pub fn keeping(track: BroadcastTrack, times: u32) -> Keeping {
    Keeping {
        again_after_failure: true,
        times: match track {
            BroadcastTrack::Catalog => Some(times),
            BroadcastTrack::Audio => None,
        },
        optional: false,
    }
}

/// The fields of a broadcast's namespace as UTF-8, `from` of its pieces;
/// `None` where one is not. WHY UTF-8: the fields go into the JSON of a
/// piece as strings.
#[must_use]
pub fn from_of(namespace: &TrackNamespace) -> Option<Vec<String>> {
    namespace
        .fields()
        .iter()
        .map(|field| String::from_utf8(field.clone()).ok())
        .collect()
}

/// What a catalog of a broadcast says to the transcriber.
#[derive(Debug)]
pub enum CatalogSays {
    /// It could not be read.
    Unread(NoCatalog),
    /// It names no Opus audio track.
    WithoutAudio,
    /// It names the audio track listened to already.
    Same,
    /// It names an audio track to listen to, in place of the subscription of
    /// this token where there is one, which is to be let go of.
    Listen {
        /// The token of the subscription to the track it names no more.
        instead_of: Option<u64>,
    },
}

/// A broadcast that is listened to, with the `L` that hears its sound.
#[derive(Debug)]
pub struct Broadcast<L> {
    /// The prefix it is under, by its place in the flags.
    pub prefix: usize,
    /// Its namespace, each field as UTF-8 ([`from_of`]).
    pub from: Vec<String>,
    /// The token of its catalog's SUBSCRIBE, while it has one.
    pub catalog: Option<u64>,
    /// The token of the FETCH of its catalog, while it has one.
    pub catalog_fetch: Option<u64>,
    /// The token of its audio track's SUBSCRIBE, while it has one.
    pub audio: Option<u64>,
    /// What hears its sound.
    pub listener: L,
    /// `--ask-again-times`, of its catalog.
    times: u32,
    /// The name of the audio track its catalog names.
    audio_name: Option<Vec<u8>>,
    catalog_tries: Tries,
    audio_tries: Tries,
}

impl<L> Broadcast<L> {
    /// A broadcast under `prefix`, whose catalog is to be asked for, and
    /// asked again up to `times` in a row ([`keeping`]).
    #[must_use]
    pub fn new(prefix: usize, from: Vec<String>, listener: L, times: u32) -> Self {
        Self {
            prefix,
            from,
            catalog: None,
            catalog_fetch: None,
            audio: None,
            listener,
            times,
            audio_name: None,
            catalog_tries: Tries::default(),
            audio_tries: Tries::default(),
        }
    }

    /// The name of the audio track it is listened to on, where its catalog
    /// named one.
    #[must_use]
    pub fn audio_name(&self) -> Option<&[u8]> {
        self.audio_name.as_deref()
    }

    /// Reads a catalog of it (`payload`): the first Opus audio track is the
    /// one listened to.
    pub fn on_catalog(&mut self, payload: &[u8]) -> CatalogSays {
        let tracks = match xmoq_msf::read_catalog(payload) {
            Ok(tracks) => tracks,
            Err(error) => return CatalogSays::Unread(error),
        };
        let Some(name) = xmoq_msf::first_audio_track(&tracks, "opus")
            .map(|track| track.name.clone().into_bytes())
        else {
            return CatalogSays::WithoutAudio;
        };
        if self.audio_name.as_ref() == Some(&name) {
            return CatalogSays::Same;
        }
        self.audio_name = Some(name);
        CatalogSays::Listen {
            instead_of: self.audio.take(),
        }
    }

    /// What becomes of `track` after `happened`: where it is no longer
    /// subscribed to, its token is gone.
    pub fn on(&mut self, track: BroadcastTrack, happened: Happened) -> Becomes {
        let (token, tries) = match track {
            BroadcastTrack::Catalog => (&mut self.catalog, &mut self.catalog_tries),
            BroadcastTrack::Audio => (&mut self.audio, &mut self.audio_tries),
        };
        if happened != Happened::Subscribed {
            *token = None;
        }
        decide(happened, keeping(track, self.times), tries)
    }

    /// The tokens of its requests: what is let go of where it is listened to
    /// no more.
    pub fn tokens(&self) -> impl Iterator<Item = u64> {
        [self.catalog, self.catalog_fetch, self.audio]
            .into_iter()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(audio: &str) -> String {
        format!(
            r#"{{"version":"draft-01","tracks":[{{"name":"video","packaging":"loc","isLive":true,"codec":"avc3.42E01F","bitrate":1000000}},{audio}]}}"#
        )
    }

    fn opus(name: &str) -> String {
        format!(
            r#"{{"name":"{name}","packaging":"loc","isLive":true,"role":"audio","codec":"opus","bitrate":67000,"samplerate":48000,"channelConfig":"1"}}"#
        )
    }

    /// What a catalog says, as a word, with the token it lets go of.
    fn said(says: &CatalogSays) -> (&'static str, Option<u64>) {
        match says {
            CatalogSays::Unread(_) => ("unread", None),
            CatalogSays::WithoutAudio => ("without-audio", None),
            CatalogSays::Same => ("same", None),
            CatalogSays::Listen { instead_of } => ("listen", *instead_of),
        }
    }

    /// The catalogs, each with the token of the audio subscription before
    /// it; what each says; the name listened to after.
    type Case = (
        &'static str,
        Vec<(String, Option<u64>)>,
        &'static [(&'static str, Option<u64>)],
        Option<&'static str>,
    );

    #[test]
    fn a_catalog_has_its_first_opus_track_listened_to_and_another_name_in_place_of_the_old() {
        // (case, the catalogs, each with the token of the audio subscription
        // before it; what each says; the name listened to after)
        let cases: [Case; 5] = [
            (
                "the first",
                vec![(catalog(&opus("audio")), None)],
                &[("listen", None)],
                Some("audio"),
            ),
            (
                "the same again",
                vec![
                    (catalog(&opus("audio")), None),
                    (catalog(&opus("audio")), Some(7)),
                ],
                &[("listen", None), ("same", None)],
                Some("audio"),
            ),
            (
                "another name: the old subscription is let go of",
                vec![
                    (catalog(&opus("audio")), None),
                    (catalog(&opus("audio-2")), Some(7)),
                ],
                &[("listen", None), ("listen", Some(7))],
                Some("audio-2"),
            ),
            (
                "a track that is not Opus is not listened to",
                vec![(
                    catalog(&opus("aac").replace(r#""codec":"opus""#, r#""codec":"mp4a.40.2""#)),
                    None,
                )],
                &[("without-audio", None)],
                None,
            ),
            (
                "a catalog that is not one leaves the track as it was",
                vec![(catalog(&opus("audio")), None), ("{".to_string(), Some(7))],
                &[("listen", None), ("unread", None)],
                Some("audio"),
            ),
        ];
        for (case, catalogs, expected, name) in cases {
            let mut broadcast = Broadcast::new(0, vec![], (), 30);
            let got: Vec<(&str, Option<u64>)> = catalogs
                .iter()
                .map(|(payload, audio)| {
                    broadcast.audio = *audio;
                    said(&broadcast.on_catalog(payload.as_bytes()))
                })
                .collect();
            assert_eq!(got, expected, "{case}");
            assert_eq!(
                broadcast.audio_name(),
                name.map(str::as_bytes),
                "{case}: listened to"
            );
        }
    }

    #[test]
    fn the_catalog_and_the_audio_are_asked_again_each_by_its_own_pauses_and_lose_their_token() {
        let mut broadcast = Broadcast::new(0, vec![], (), 2);
        let refused_at_once = Happened::Refused {
            error_code: 0x5,
            named_ms: Some(0),
        };
        // (track, its token before, what happened, what becomes of it, its
        // token after)
        let (catalog, audio) = (BroadcastTrack::Catalog, BroadcastTrack::Audio);
        let (subscribed, malformed) = (Happened::Subscribed, Happened::Malformed);
        let again = Becomes::Again;
        let steps = [
            (catalog, Some(1), refused_at_once, again(100), None),
            (catalog, None, refused_at_once, again(200), None),
            (audio, Some(2), refused_at_once, again(100), None),
            (audio, Some(3), subscribed, Becomes::Subscribed, Some(3)),
            (audio, Some(3), refused_at_once, again(100), None),
            (catalog, Some(11), subscribed, Becomes::Subscribed, Some(11)),
            (catalog, Some(4), Happened::Ended, again(1000), None),
            (catalog, None, Happened::NotSent, again(1000), None),
            // The catalog is asked again 2 times in a row, also where it is
            // found malformed after each SUBSCRIBE_OK; the audio track for as
            // long as it is malformed.
            (catalog, None, Happened::NotSent, Becomes::GivenUp, None),
            (catalog, Some(5), subscribed, Becomes::Subscribed, Some(5)),
            (catalog, Some(5), malformed, again(1000), None),
            (catalog, Some(6), subscribed, Becomes::Subscribed, Some(6)),
            (catalog, Some(6), malformed, again(1000), None),
            (catalog, Some(7), subscribed, Becomes::Subscribed, Some(7)),
            (catalog, Some(7), malformed, Becomes::GivenUp, None),
            (audio, Some(8), malformed, again(1000), None),
            (audio, Some(9), malformed, again(1000), None),
            (audio, Some(10), malformed, again(1000), None),
        ];
        for (index, (track, before, happened, becomes, after)) in steps.into_iter().enumerate() {
            match track {
                BroadcastTrack::Catalog => broadcast.catalog = before,
                BroadcastTrack::Audio => broadcast.audio = before,
            }
            assert_eq!(broadcast.on(track, happened), becomes, "step {index}");
            let token = match track {
                BroadcastTrack::Catalog => broadcast.catalog,
                BroadcastTrack::Audio => broadcast.audio,
            };
            assert_eq!(token, after, "step {index}");
        }
    }
}
