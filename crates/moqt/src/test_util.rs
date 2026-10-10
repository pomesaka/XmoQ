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
