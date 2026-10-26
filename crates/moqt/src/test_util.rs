//! Helpers shared by the unit tests.

/// Reads `fixtures/moqt-18/<name>.hex`, bytes captured from moqtail and, for
/// one stream, from `moqlivemock`. The
/// directory is shared with the TypeScript implementation; its README records
/// where each file comes from.
pub(crate) fn fixture(name: &str) -> Vec<u8> {
    read_hex("moqt-18", name)
}

/// Reads `fixtures/loc-04/<name>.hex`: a payload `moqlivemock`'s publisher
/// sent, which a stream of `fixtures/moqt-18/` carries.
pub(crate) fn fixture_of_loc(name: &str) -> Vec<u8> {
    read_hex("loc-04", name)
}

/// Reads `fixtures/moqt-18-by-hand/<name>.hex`, bytes written by hand from
/// the draft, which the TypeScript implementation reads too. No other
/// implementation wrote them.
pub(crate) fn fixture_by_hand(name: &str) -> Vec<u8> {
    read_hex("moqt-18-by-hand", name)
}

/// Reads `fixtures/moqt-18-from-libraries/<name>.hex`, bytes a peer's
/// library serialized in a script and no wire carried, which the TypeScript
/// implementation reads too.
pub(crate) fn fixture_from_a_library(name: &str) -> Vec<u8> {
    read_hex("moqt-18-from-libraries", name)
}

fn read_hex(directory: &str, name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../../fixtures/{directory}/{name}.hex",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    digits
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap_or_else(|e| panic!("{path}: {e}"));
            u8::from_str_radix(pair, 16).unwrap_or_else(|e| panic!("{path}: {pair}: {e}"))
        })
        .collect()
}

/// The names of the `.hex` files of a directory under `fixtures/`, sorted.
fn names_on_disk(directory: &str) -> Vec<String> {
    let path = format!("{}/../../fixtures/{directory}", env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<String> = std::fs::read_dir(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".hex").map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

#[test]
fn every_fixture_file_is_read_by_a_test() {
    // WHY: the tests name the files they read. A capture that is added to the
    // directory and to no test would look covered and check nothing.
    let mut read: Vec<String> = crate::message::tests::fixture_names()
        .into_iter()
        .chain(crate::data::tests::fixture_names())
        .chain(crate::fetch::tests::fixture_names())
        .map(str::to_string)
        .collect();
    read.sort();

    assert_eq!(names_on_disk("moqt-18"), read);
}

#[test]
fn every_file_a_library_wrote_is_read_by_a_test() {
    let mut read: Vec<String> = crate::message::tests::names_of_files_a_library_wrote()
        .into_iter()
        .map(str::to_string)
        .collect();
    read.sort();

    assert_eq!(names_on_disk("moqt-18-from-libraries"), read);
}

/// The `REQUEST_UPDATE` messages written by hand, the Request ID each has,
/// and its `NEW_GROUP_REQUEST`, where it has one; no other parameters.
/// `fixtures/peer-subscribe/` names the first two as what a server sent
/// before the bytes of a row.
pub(crate) const REQUEST_UPDATES_BY_HAND: [(&str, u64, Option<u64>); 3] = [
    ("request-update-request-id-3", 3, None),
    ("request-update-request-id-5", 5, None),
    ("request-update-new-group-request", 3, Some(7)),
];

#[test]
fn every_file_written_by_hand_is_read_by_a_test() {
    let mut read = crate::fetch::tests::names_of_files_written_by_hand();
    read.extend(REQUEST_UPDATES_BY_HAND.map(|(name, _, _)| name));
    read.sort_unstable();

    assert_eq!(names_on_disk("moqt-18-by-hand"), read);
}

/// The rows of a shared table: its lines, each split at the tabs into
/// `N` fields. The first field is the name of the row.
pub(crate) fn table_rows<const N: usize>(path: &str) -> Vec<[String; N]> {
    let path = format!("{}/../../fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    let table = std::fs::read_to_string(&path).expect("the fixture is there");
    let body = table
        .strip_suffix('\n')
        .unwrap_or_else(|| panic!("{path}: the last row does not end with a line feed"));
    let mut rows: Vec<[String; N]> = Vec::new();
    for line in body.split('\n') {
        let fields: Vec<String> = line.split('\t').map(str::to_string).collect();
        let row: [String; N] = fields
            .try_into()
            .unwrap_or_else(|_| panic!("{path}: not {N} fields: {line:?}"));
        assert!(
            rows.iter().all(|other| other[0] != row[0]),
            "{path}: twice: {:?}",
            row[0]
        );
        rows.push(row);
    }
    rows
}

/// What the session does for an error that closes it, as the shared
/// tables write it.
pub(crate) fn closed(error: &crate::Error) -> String {
    match error.termination_code() {
        Some(code) => format!("closed {:#x}", code.code()),
        None => panic!("an error that closes no session: {error:?}"),
    }
}

/// The rows of `fixtures/update-answers/answers.tsv`: the name, the steps
/// and what comes of each, as its README has them.
pub(crate) fn update_answer_steps() -> Vec<(String, String, String)> {
    let rows = table_rows::<3>("update-answers/answers.tsv");
    // As many as the README has.
    assert_eq!(rows.len(), 10);
    rows.into_iter()
        .map(|[name, steps, events]| (name, steps, events))
        .collect()
}
