//! The requests a client opens itself, each written by a task of its own
//! ([`crate::streams::open_request`]), by the token the task reports with:
//! what each is for (the caller's `P`), the session's handle of it, and how
//! the loop tells its task something or lets it go. The gatherer of the chat
//! and the transcriber keep theirs so.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::params::{ANSWER_WAIT_MS, Parameter, answer_wait_ms};
use xmoq_moqt::session::{RequestId, Session};
use xmoq_moqt_io::Connection;
use xmoq_moqt_io::wt::WtConnection;

use crate::streams::{Command, Heard, RequestTask, Waits, open_request};

/// One request of the client.
pub struct Own<P> {
    /// What it is for.
    pub purpose: P,
    /// The session's handle of it.
    pub request: RequestId,
    task: RequestTask,
}

/// The requests of the client that are open, by token, on the session of
/// `connection`.
pub struct OwnRequests<P, C: Connection = WtConnection> {
    connection: Arc<C>,
    own: HashMap<u64, Own<P>>,
    next_token: u64,
    heard: mpsc::Sender<(u64, Heard)>,
    open_wait: Duration,
}

/// Where the queue of a request's task is full ([`OwnRequests::tell`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

impl<P, C: Connection> OwnRequests<P, C> {
    /// None yet, on `connection`. Their tasks tell `heard` what happens,
    /// with the token; a request's stream is waited for `open_wait`.
    #[must_use]
    pub fn new(connection: Arc<C>, heard: mpsc::Sender<(u64, Heard)>, open_wait: Duration) -> Self {
        Self {
            connection,
            own: HashMap::new(),
            next_token: 0,
            heard,
            open_wait,
        }
    }

    /// Opens the stream of `message`, which the session made as `request`,
    /// for `purpose`; its answer is waited for as long as `parameters` ask
    /// (`answer_wait_ms` of `xmoq-moqt`). Returns its token.
    pub fn open(
        &mut self,
        purpose: P,
        request: RequestId,
        message: ControlMessage,
        parameters: &[Parameter],
    ) -> u64 {
        let token = self.next_token;
        self.next_token += 1;
        let waits = Waits {
            open: self.open_wait,
            answer: Duration::from_millis(answer_wait_ms(ANSWER_WAIT_MS, parameters)),
        };
        let task = open_request(&self.connection, token, message, waits, self.heard.clone());
        self.own.insert(
            token,
            Own {
                purpose,
                request,
                task,
            },
        );
        token
    }

    /// The request of `token`, where it is open.
    #[must_use]
    pub fn get(&self, token: u64) -> Option<&Own<P>> {
        self.own.get(&token)
    }

    /// The token of the request the session knows as `request`.
    #[must_use]
    pub fn token_of(&self, request: RequestId) -> Option<u64> {
        self.own
            .iter()
            .find_map(|(token, own)| (own.request == request).then_some(*token))
    }

    /// Tells the task of `token` something. Where its queue is full, the
    /// caller cancels the request ([`Self::let_go`] with `EXCESSIVE_LOAD`,
    /// §3.3.3) and goes on without it.
    ///
    /// WHY `try_send` and not a wait: the loop must not wait on one stream.
    /// WHY no command is ever dropped: among them are the answers to the
    /// relay's `REQUEST_UPDATE`s, and §10.9 has "The receiver of a
    /// `REQUEST_UPDATE` MUST respond with exactly one `REQUEST_OK` or
    /// `REQUEST_ERROR`". Where the queue is full
    /// ([`crate::streams::COMMAND_QUEUE`]), the relay takes nothing on the
    /// stream and sends more than it reads, as the own relay ends a FETCH
    /// with too many updates waiting.
    ///
    /// # Errors
    ///
    /// [`QueueFull`].
    pub fn tell(&self, token: u64, command: Command) -> Result<(), QueueFull> {
        match self.own.get(&token) {
            Some(own) if own.task.commands.try_send(command).is_err() => Err(QueueFull),
            _ => Ok(()),
        }
    }

    /// Ends the request of `token` in `session` and on its stream, and
    /// forgets it: with `Command::Cancel` it is cancelled (§3.3.2) before
    /// what waits in its queue; any other command is the last its task
    /// does. Returns what it was for.
    pub fn let_go(&mut self, session: &mut Session, token: u64, command: Command) -> Option<P> {
        let mut own = self.own.remove(&token)?;
        match command {
            Command::Cancel(code) => {
                session.cancel(own.request);
                if let Some(abort) = own.task.abort.take() {
                    let _ = abort.send(code);
                }
            }
            // Queued before the sender is dropped, it is read before the
            // task sees the end of the queue; where it does not fit, the
            // dropped sender cancels the stream.
            command => {
                let _ = own.task.commands.try_send(command);
            }
        }
        Some(own.purpose)
    }

    /// Forgets the request of `token` whose stream was never opened: the
    /// session's handle is the caller's to cancel.
    pub fn forget(&mut self, token: u64) -> Option<Own<P>> {
        self.own.remove(&token)
    }
}
