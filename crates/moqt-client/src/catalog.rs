//! `--catalog-video`: the payload of the catalog Object that came last, by
//! the subscription to the catalog track or by its Joining FETCH.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// WHY for the whole process, and not a value that is handed along: the
/// Objects of a fetch stream are read in a task of their own and the ones
/// of a subscription by the client, and the client reads one catalog.
static WANTED: AtomicBool = AtomicBool::new(false);
static LATEST: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// From now on the payloads that are offered are kept.
pub fn want() {
    WANTED.store(true, Ordering::Relaxed);
}

/// The payload of an Object of the catalog track. The one that is offered
/// last stays: the Joining FETCH brings the catalog there is, and the
/// subscription the ones that come after it.
pub fn offer(payload: &[u8]) {
    if WANTED.load(Ordering::Relaxed)
        && let Ok(mut latest) = LATEST.lock()
    {
        *latest = Some(payload.to_vec());
    }
}

/// Whether a catalog came.
pub fn came() -> bool {
    LATEST.lock().is_ok_and(|latest| latest.is_some())
}

/// The catalog that came last; none is kept or wanted after this.
pub fn take() -> Option<Vec<u8>> {
    WANTED.store(false, Ordering::Relaxed);
    LATEST.lock().ok().and_then(|mut latest| latest.take())
}
