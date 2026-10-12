//! Decodes the Opus packets of a file with the crate `opus-decoder` and with
//! libopus (the crate `opus`), and sets the samples against each other.
//!
//!   opus-compare <packets> [channels, 1]
//!
//! `<packets>` has the lines of `xmoq-moqt-client --mode subscribe`: the
//! payload of an `OBJECT ... payload_hex=...` line is one packet.
//!
//! Built with `--profile checked` (overflow checks on) it does something
//! else: it decodes with `opus-decoder` alone, catches the panic of every
//! packet at which the decoder overflows, and writes the numbers of those
//! packets to `<packets>.overflow`, one a line. The decoder is made anew
//! after a panic, so what it decodes after one is not what a decoder that
//! went on would; whether a packet overflows is the packet's own doing
//! (the number of blocks of a band), which is what is asked here.
//!
//! Built with `--release` it decodes with both and prints `OPUS_COMPARED`:
//! how many packets differ and by how much, and how that falls together
//! with the packets of `<packets>.overflow`.

use std::io::BufRead;

const RATE: u32 = 48_000;

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).filter_map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok()).collect()
}

fn packets(path: &str) -> Vec<Vec<u8>> {
    let file = std::fs::File::open(path).expect("the file of packets");
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.starts_with("OBJECT "))
        .filter_map(|line| line.rsplit("payload_hex=").next().map(|hex| unhex(hex.trim())))
        .filter(|packet| !packet.is_empty())
        .collect()
}

/// Whether this build checks arithmetic for overflow.
fn checks_overflow() -> bool {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let checked = std::panic::catch_unwind(|| {
        let one: u8 = std::hint::black_box(1);
        one << std::hint::black_box(9u32)
    })
    .is_err();
    std::panic::set_hook(hook);
    checked
}

fn overflowing(path: &str, packets: &[Vec<u8>], channels: usize) {
    let places = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<String, u64>::new()));
    let noted = std::sync::Arc::clone(&places);
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location().map_or("?".to_string(), |at| format!("{}:{}", at.file().rsplit("opus-decoder-").next().unwrap_or("?"), at.line()));
        *noted.lock().expect("the lock").entry(place).or_default() += 1;
    }));
    let new = || opus_decoder::OpusDecoder::new(RATE, channels).expect("a decoder");
    let mut decoder = new();
    let mut pcm = vec![0f32; 5760 * channels];
    let mut numbers = Vec::new();
    for (number, packet) in packets.iter().enumerate() {
        let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decoder.decode_float(packet, &mut pcm, false).is_ok()));
        if decoded.is_err() {
            numbers.push(number);
            decoder = new();
        }
    }
    let out: String = numbers.iter().map(|number| format!("{number}\n")).collect();
    std::fs::write(format!("{path}.overflow"), out).expect("the file of numbers");
    println!(
        "OPUS_OVERFLOWS packets={} overflowing={} where={:?}",
        packets.len(),
        numbers.len(),
        places.lock().expect("the lock")
    );
}

fn compared(path: &str, packets: &[Vec<u8>], channels: usize) {
    let overflow: std::collections::BTreeSet<usize> = std::fs::read_to_string(format!("{path}.overflow"))
        .map(|text| text.lines().filter_map(|line| line.parse().ok()).collect())
        .unwrap_or_default();
    let mut ours = opus_decoder::OpusDecoder::new(RATE, channels).expect("a decoder");
    let mut theirs =
        opus::Decoder::new(RATE, if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo }).expect("libopus");
    let (mut a, mut b) = (vec![0f32; 5760 * channels], vec![0f32; 5760 * channels]);
    // A packet differs where a sample is further apart than this: -60 dB of full scale.
    const APART: f32 = 0.001;
    let (mut samples, mut squares, mut signal, mut most) = (0u64, 0f64, 0f64, 0f32);
    let (mut differ, mut differ_overflowing, mut refused, mut lengths) = (0u64, 0u64, 0u64, 0u64);
    // How many packets after the last overflowing one each differing packet is.
    let mut after: Vec<usize> = Vec::new();
    let mut last_overflow: Option<usize> = None;
    let mut worst: Vec<(f32, usize)> = Vec::new();
    for (number, packet) in packets.iter().enumerate() {
        if overflow.contains(&number) {
            last_overflow = Some(number);
        }
        let (Ok(n), Ok(m)) = (ours.decode_float(packet, &mut a, false), theirs.decode_float(packet, &mut b, false)) else {
            refused += 1;
            continue;
        };
        if n != m {
            lengths += 1;
            continue;
        }
        let mut apart = 0f32;
        for (x, y) in a[..n * channels].iter().zip(&b[..n * channels]) {
            let d = (x - y).abs();
            apart = apart.max(d);
            squares += f64::from(d) * f64::from(d);
            signal += f64::from(*y) * f64::from(*y);
            samples += 1;
        }
        most = most.max(apart);
        if apart > APART {
            differ += 1;
            if overflow.contains(&number) {
                differ_overflowing += 1;
            }
            after.push(last_overflow.map_or(usize::MAX, |last| number - last));
            worst.push((apart, number));
        }
    }
    worst.sort_by(|x, y| y.0.total_cmp(&x.0));
    after.sort_unstable();
    let within = |packets: usize| after.iter().filter(|distance| **distance <= packets).count();
    println!(
        "OPUS_COMPARED packets={} refused={refused} of_other_length={lengths} samples={samples} \
         differ_by_more_than_0.001={differ} of_them_overflowing={differ_overflowing} overflowing={} \
         differing_within_0/1/5/50_packets_after_an_overflow={}/{}/{}/{} \
         greatest_difference={most:.6} rms_of_difference={:.8} rms_of_libopus={:.6} worst={:?}",
        packets.len(),
        overflow.len(),
        within(0),
        within(1),
        within(5),
        within(50),
        (squares / samples.max(1) as f64).sqrt(),
        (signal / samples.max(1) as f64).sqrt(),
        worst.iter().take(5).collect::<Vec<_>>(),
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("the file of packets");
    let channels: usize = args.next().map_or(1, |text| text.parse().expect("channels"));
    let packets = packets(&path);
    if checks_overflow() {
        overflowing(&path, &packets, channels);
    } else {
        compared(&path, &packets, channels);
    }
}
