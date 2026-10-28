//! The calls of the decisions API: one POST a batch, in a task of its own,
//! whose answer comes back to the session's loop on a queue. What is asked
//! and what is read of the answer is `xmoq_pet::decisions`.
//!
//! WHY reqwest, and as `xmoq-viewer-io` has it (rustls with ring, without
//! its default features): the reasons at the top of
//! `crates/viewer-io/src/room_client.rs`. This file alone touches it.
//!
//! **The bearer token** is read once, from `DECISIONS_API_TOKEN` alone
//! (`main.rs`), and goes into the `Authorization` header and nowhere else:
//! no flag, no line of the log, no file. What the log says of a call that
//! failed is one word ([`Failed::word`]), never reqwest's error, which can
//! carry the request.

use std::sync::Once;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use xmoq_pet::decide::Decision;
use xmoq_pet::decisions::{NotDecided, read, request, warm_up};
use xmoq_pet::perceive::Happening;
use xmoq_pet::pet::Pet;
use xmoq_pet::rate::PerMinute;
use xmoq_pet_tracks::payloads::{PetState, Place, Thing};

/// How long a call may take, its answer's body read included; after it the
/// batch is decided by the table. WHY 5 s: the API took about 1.7 s for
/// four questions on 2026-10-28 (the PM's measure), and a pet that reacts
/// later than that looks as if it did not hear. **Provisional.**
pub const DECISION_WAIT: Duration = Duration::from_secs(5);

/// How long the call that warms the API up may take. WHY longer than
/// [`DECISION_WAIT`]: the first call of a set of questions took 4 to 11 s
/// while the API compiled its schema (the PM's runs of 2026-10-28).
/// **Provisional.**
pub const WARM_UP_WAIT: Duration = Duration::from_secs(20);

/// The most bytes of an answer. Four answers are under 2 KiB; the bound
/// keeps a server that is not the API from filling the memory.
const MOST_AN_ANSWER: usize = 64 * 1024;

/// Why a call gave no decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failed {
    /// No answer within [`DECISION_WAIT`].
    Timeout,
    /// No connection, or the request could not be sent.
    Unreachable,
    /// An answer over [`MOST_AN_ANSWER`].
    TooLarge,
    /// What `xmoq_pet::decisions::read` found.
    Read(NotDecided),
}

impl Failed {
    /// One word for the log.
    pub fn word(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::TooLarge => "too-large",
            Self::Read(why) => why.word(),
        }
    }
}

/// What came of a call for the batch of a room.
pub struct Answered {
    /// The room.
    pub room: String,
    /// The batch it was for.
    pub batch: Vec<Happening>,
    /// The decision, or why there is none.
    pub decided: Result<Decision, Failed>,
}

/// Has rustls take its ring provider for the process, as
/// `xmoq-viewer-io` does (why, there).
fn install_the_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // An error is a provider installed already, which is used instead.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The decisions API, and how often it may be called.
pub struct Decider {
    http: reqwest::Client,
    url: reqwest::Url,
    token: String,
    per_minute: PerMinute,
    answers: mpsc::Sender<Answered>,
}

impl Decider {
    /// The API at `url`, called with `token` no more than `per_minute` times
    /// in any minute; what it answers goes to `answers`.
    ///
    /// # Errors
    ///
    /// A URL that is none, and a client that cannot be built.
    pub fn new(
        url: &str,
        token: String,
        per_minute: usize,
        answers: mpsc::Sender<Answered>,
    ) -> Result<Self> {
        install_the_crypto_provider();
        let url = reqwest::Url::parse(url).context("--decisions-url")?;
        let http = reqwest::Client::builder()
            .timeout(DECISION_WAIT)
            // WHY no redirect: the token would go to whatever server the
            // redirect names.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("a client of the decisions API")?;
        Ok(Self {
            http,
            url,
            token,
            per_minute: PerMinute::new(per_minute),
            answers,
        })
    }

    /// Calls the API once with a made-up batch, in a task, and throws its
    /// answer away (`warm_up`, where the reason is); prints how long it
    /// took, or why it failed. Not counted against the rate.
    pub fn warm_up(&self) {
        let (floor, batch) = warm_up();
        let pet = Pet::new(1, 0);
        let body = request(pet.state(), pet.place_at(0), &floor, &batch);
        let call = self
            .http
            .post(self.url.clone())
            .timeout(WARM_UP_WAIT)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
        tokio::spawn(async move {
            let began = std::time::Instant::now();
            let answered = match answer_of(call).await {
                Ok((status, body)) => read(status, &body, &floor, &batch).map_err(Failed::Read),
                Err(failed) => Err(failed),
            };
            match answered {
                Ok(_) => println!("DECISIONS_WARMED took_ms={}", began.elapsed().as_millis()),
                Err(failed) => println!(
                    "DECISIONS_NOT_WARMED why={} took_ms={}",
                    failed.word(),
                    began.elapsed().as_millis()
                ),
            }
        });
    }

    /// Asks for the decision of `batch` with `floor` in a task, where the
    /// rate lets it at `now_ms` (a clock that does not go back); `false`
    /// where it does not, and nothing is asked.
    pub fn ask(
        &mut self,
        room: String,
        (floor, batch): (Vec<Thing>, Vec<Happening>),
        (state, place): (&PetState, Place),
        now_ms: u64,
    ) -> bool {
        if !self.per_minute.take(now_ms) {
            return false;
        }
        let body = request(state, place, &floor, &batch);
        let call = self
            .http
            .post(self.url.clone())
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
        let answers = self.answers.clone();
        tokio::spawn(async move {
            let decided = match answer_of(call).await {
                Ok((status, body)) => read(status, &body, &floor, &batch).map_err(Failed::Read),
                Err(failed) => Err(failed),
            };
            // A queue that is gone is a session loop that is gone.
            let _ = answers
                .send(Answered {
                    room,
                    batch,
                    decided,
                })
                .await;
        });
        true
    }
}

/// The status and the body of the answer to `call`.
async fn answer_of(call: reqwest::RequestBuilder) -> Result<(u16, Vec<u8>), Failed> {
    let failed = |error: &reqwest::Error| {
        if error.is_timeout() {
            Failed::Timeout
        } else {
            Failed::Unreachable
        }
    };
    let mut response = call.send().await.map_err(|error| failed(&error))?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| failed(&error))? {
        if body.len() + chunk.len() > MOST_AN_ANSWER {
            return Err(Failed::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}
