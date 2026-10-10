//! Helpers shared by the unit tests.

/// Reads `fixtures/moqt-18/<name>.hex`, bytes captured from moqtail. The
/// directory is shared with the TypeScript implementation; its README records
/// where each file comes from.
pub(crate) fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../../fixtures/moqt-18/{name}.hex",
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

#[test]
fn every_fixture_file_is_read_by_a_test() {
    // WHY: the tests name the files they read. A capture that is added to the
    // directory and to no test would look covered and check nothing.
    let directory = format!("{}/../../fixtures/moqt-18", env!("CARGO_MANIFEST_DIR"));
    let mut on_disk: Vec<String> = std::fs::read_dir(&directory)
        .unwrap_or_else(|e| panic!("{directory}: {e}"))
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".hex").map(str::to_string)
        })
        .collect();
    on_disk.sort();

    let mut read: Vec<String> = crate::message::tests::fixture_names()
        .into_iter()
        .chain(crate::data::tests::fixture_names())
        .map(str::to_string)
        .collect();
    read.sort();

    assert_eq!(on_disk, read);
}
