//! Decodes the Opus packets of a file with each Opus decoder in Rust and with
//! libopus (the crate `opus`), and sets the samples against libopus's.
//!
//!   opus-candidates <packets> [channels, 1] [runs, 3]
//!
//! `<packets>` has the lines of `xmoq-moqt-client --mode subscribe` (what
//! `spikes/opus-compare/capture.sh` writes): the payload of an
//! `OBJECT ... payload_hex=...` line is one packet.
//!
//! Every decoder decodes the whole file with one decoder, `runs` times; the
//! CPU time of the thread is taken around each run, and the samples of the
//! first run are compared. A packet at which a decoder panics is counted with
//! where it panicked, and the decoder is made anew. Built with
//! `--profile checked` (overflow checks on) a panic is mostly an overflow;
//! with `--release` arithmetic wraps and only other panics remain.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::Mutex;

const RATE: u32 = 48_000;
/// 120 ms at 48 kHz, the longest an Opus packet decodes to.
const MOST: usize = 5760;

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

fn thread_cpu_seconds() -> f64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid pointer to a timespec on the stack.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    now.tv_sec as f64 + now.tv_nsec as f64 / 1e9
}

/// One decoder for the whole file: decodes a packet into `out`
/// (interleaved), and says how many samples of all channels it wrote.
trait Decode {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String>;
}

struct Libopus(opus::Decoder, usize);
impl Decode for Libopus {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        self.0.decode_float(packet, out, false).map(|n| n * self.1).map_err(|e| e.to_string())
    }
}

struct OpusDecoder(opus_decoder::OpusDecoder, usize);
impl Decode for OpusDecoder {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        self.0.decode_float(packet, out, false).map(|n| n * self.1).map_err(|e| e.to_string())
    }
}

struct OpusRs(opus_rs::OpusDecoder, usize);
impl Decode for OpusRs {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        self.0.decode(packet, MOST, out).map(|n| n * self.1).map_err(str::to_string)
    }
}

struct RustyOpus(rusty_opus::OpusDecoder, usize);
impl Decode for RustyOpus {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        self.0.decode(packet, MOST, out).map(|n| n * self.1).map_err(|e| format!("{e:?}"))
    }
}

struct OpusPure(opus_pure::OpusDecoder, usize);
impl Decode for OpusPure {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        self.0.decode(packet, MOST, out).map(|n| n * self.1).map_err(|e| format!("{e:?}"))
    }
}

struct Ruopus(ruopus::OpusDecoder);
impl Decode for Ruopus {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        let pcm = self.0.decode_packet(packet).map_err(|e| format!("{e:?}"))?;
        let n = pcm.len().min(out.len());
        out[..n].copy_from_slice(&pcm[..n]);
        Ok(pcm.len())
    }
}

struct Ropus(ropus::Decoder, usize);
impl Decode for Ropus {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        self.0
            .decode_float(packet, &mut out[..MOST * self.1], ropus::DecodeMode::Normal)
            .map(|n| n * self.1)
            .map_err(|e| format!("{e:?}"))
    }
}

struct UnsafeLibopus(*mut unsafe_libopus::OpusDecoder, usize);
impl Decode for UnsafeLibopus {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        // SAFETY: the decoder was made by opus_decoder_create; `out` holds MOST samples a channel.
        let n = unsafe {
            unsafe_libopus::opus_decode_float(self.0, packet.as_ptr(), packet.len() as i32, out.as_mut_ptr(), MOST as i32, 0)
        };
        if n < 0 { Err(format!("error {n}")) } else { Ok(n as usize * self.1) }
    }
}

struct Mousiki(mousiki::c_style_api::opus_decoder::OpusDecoder<'static>, usize);
impl Decode for Mousiki {
    fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, String> {
        mousiki::c_style_api::opus_decoder::opus_decode_float(&mut self.0, Some(packet), packet.len(), out, MOST, false)
            .map(|n| n * self.1)
            .map_err(|e| format!("{e:?}"))
    }
}

fn make(name: &str, channels: usize) -> Box<dyn Decode> {
    match name {
        "libopus" => Box::new(Libopus(
            opus::Decoder::new(RATE, if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo }).expect("libopus"),
            channels,
        )),
        "opus-decoder" => Box::new(OpusDecoder(opus_decoder::OpusDecoder::new(RATE, channels).expect("a decoder"), channels)),
        "opus-rs" => Box::new(OpusRs(opus_rs::OpusDecoder::new(RATE as i32, channels).expect("a decoder"), channels)),
        "rusty-opus" => Box::new(RustyOpus(rusty_opus::OpusDecoder::new(RATE as i32, channels).expect("a decoder"), channels)),
        "opus-pure" => Box::new(OpusPure(opus_pure::OpusDecoder::new(RATE as i32, channels).expect("a decoder"), channels)),
        "ruopus" => Box::new(Ruopus(ruopus::OpusDecoder::new(channels))),
        "ropus" => Box::new(Ropus(
            ropus::Decoder::new(RATE, if channels == 1 { ropus::Channels::Mono } else { ropus::Channels::Stereo }).expect("a decoder"),
            channels,
        )),
        "unsafe-libopus" => {
            let mut error = 0;
            // SAFETY: a valid pointer for the error.
            let decoder = unsafe { unsafe_libopus::opus_decoder_create(RATE as i32, channels as i32, &mut error) };
            assert!(error == 0 && !decoder.is_null(), "unsafe-libopus: {error}");
            Box::new(UnsafeLibopus(decoder, channels))
        }
        "mousiki" => Box::new(Mousiki(
            mousiki::c_style_api::opus_decoder::opus_decoder_create(RATE as i32, channels as i32).expect("a decoder"),
            channels,
        )),
        other => panic!("no decoder {other}"),
    }
}

const NAMES: [&str; 9] =
    ["libopus", "opus-decoder", "opus-rs", "rusty-opus", "opus-pure", "ruopus", "ropus", "unsafe-libopus", "mousiki"];

static PLACES: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// What one pass over the file gave: per packet the samples, or why none.
enum Outcome {
    Samples(Vec<f32>),
    Refused(String),
    Panicked,
}

fn pass(name: &str, packets: &[Vec<u8>], channels: usize) -> (Vec<Outcome>, f64) {
    let mut decoder = make(name, channels);
    let mut out = vec![0f32; MOST * channels];
    let mut outcomes = Vec::with_capacity(packets.len());
    let start = thread_cpu_seconds();
    for packet in packets {
        let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decoder.decode(packet, &mut out)));
        outcomes.push(match decoded {
            Ok(Ok(n)) => Outcome::Samples(out[..n.min(out.len())].to_vec()),
            Ok(Err(why)) => Outcome::Refused(why),
            Err(_) => {
                decoder = make(name, channels);
                Outcome::Panicked
            }
        });
    }
    (outcomes, thread_cpu_seconds() - start)
}

/// For a decoder far from libopus: the shift (in samples a channel) and
/// sign of its output over the first 10 s that comes nearest libopus's, and
/// the RMS of the difference there.
fn nearest_shift(name: &str, packets: &[Vec<u8>], channels: usize) {
    let joined = |outcomes: Vec<Outcome>| -> Vec<f32> {
        outcomes
            .into_iter()
            .take(500)
            .flat_map(|outcome| match outcome {
                Outcome::Samples(samples) => samples,
                _ => Vec::new(),
            })
            .collect()
    };
    let ours = joined(pass(name, packets, channels).0);
    let theirs = joined(pass("libopus", packets, channels).0);
    let mut best = (f64::MAX, 0i64, 1f32);
    for shift in -2000i64..=2000 {
        for sign in [1f32, -1f32] {
            let (mut squares, mut count) = (0f64, 0u64);
            for at in 4000 * channels..theirs.len().saturating_sub(4000 * channels) {
                let from = at as i64 + shift * channels as i64;
                let Some(x) = usize::try_from(from).ok().and_then(|from| ours.get(from)) else { continue };
                let d = f64::from(sign * x - theirs[at]);
                squares += d * d;
                count += 1;
            }
            let rms = (squares / count.max(1) as f64).sqrt();
            if rms < best.0 {
                best = (rms, shift, sign);
            }
        }
    }
    println!("NEAREST {name} shift={} sign={} rms_of_difference={:.8}", best.1, best.2, best.0);
}

fn main() {
    if let Some(name) = std::env::var("NEAREST").ok() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let channels = args.get(1).map_or(1, |text| text.parse().expect("channels"));
        nearest_shift(&name, &packets(&args[0]), channels);
        return;
    }
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("the file of packets");
    let channels: usize = args.next().map_or(1, |text| text.parse().expect("channels"));
    let runs: usize = args.next().map_or(3, |text| text.parse().expect("runs"));
    let only: Option<String> = args.next();
    let packets = packets(&path);
    std::panic::set_hook(Box::new(|info| {
        let place = info.location().map_or("?".to_string(), |at| {
            let file = at.file();
            let short = file.rsplit("/registry/src/").next().unwrap_or(file);
            let short = short.split_once('/').map_or(short, |(_, rest)| rest);
            format!("{short}:{}", at.line())
        });
        *PLACES.lock().expect("the lock").entry(place).or_default() += 1;
    }));
    let (reference, _) = pass("libopus", &packets, channels);
    let audio = packets.len() as f64 * 0.02;
    println!("PACKETS {} channels={channels} seconds_of_sound={audio:.0}", packets.len());
    for name in NAMES.iter().filter(|name| only.as_deref().is_none_or(|only| only == **name)) {
        PLACES.lock().expect("the lock").clear();
        let mut times = Vec::new();
        let mut first = None;
        for _ in 0..runs {
            let (outcomes, seconds) = pass(name, &packets, channels);
            times.push(seconds);
            first.get_or_insert(outcomes);
        }
        let places = std::mem::take(&mut *PLACES.lock().expect("the lock"));
        times.sort_by(f64::total_cmp);
        let outcomes = first.expect("one run");
        let (mut refused, mut panicked, mut lengths, mut differ_3, mut differ_2) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let (mut samples, mut squares, mut signal, mut most) = (0u64, 0f64, 0f64, 0f32);
        let mut why: BTreeMap<String, u64> = BTreeMap::new();
        for (ours, theirs) in outcomes.iter().zip(&reference) {
            let (a, b) = match (ours, theirs) {
                (Outcome::Samples(a), Outcome::Samples(b)) => (a, b),
                (Outcome::Refused(reason), _) => {
                    refused += 1;
                    *why.entry(reason.clone()).or_default() += 1;
                    continue;
                }
                (Outcome::Panicked, _) => {
                    panicked += 1;
                    continue;
                }
                _ => continue,
            };
            if a.len() != b.len() {
                lengths += 1;
                continue;
            }
            let mut apart = 0f32;
            for (x, y) in a.iter().zip(b) {
                let d = (x - y).abs();
                apart = apart.max(d);
                squares += f64::from(d) * f64::from(d);
                signal += f64::from(*y) * f64::from(*y);
                samples += 1;
            }
            most = most.max(apart);
            if apart > 0.001 {
                differ_3 += 1;
            }
            if apart > 0.01 {
                differ_2 += 1;
            }
        }
        let rms = (squares / samples.max(1) as f64).sqrt();
        let signal_rms = (signal / samples.max(1) as f64).sqrt();
        println!(
            "DECODER {name} refused={refused} panicked={panicked} of_other_length={lengths} samples={samples} \
             packets_apart_by_more_than_0.001={differ_3} more_than_0.01={differ_2} greatest_difference={most:.6} \
             greatest_db={:.1} rms_of_difference={rms:.8} rms_of_libopus={signal_rms:.6} snr_db={:.1} \
             cpu_seconds_min={:.3} cpu_seconds_all={:?} realtime_factor={:.0} refusals={why:?} panics_at={places:?}",
            20.0 * f64::from(most.max(1e-12)).log10(),
            20.0 * (signal_rms / rms.max(1e-15)).log10(),
            times[0],
            times.iter().map(|t| (t * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
            audio / times[0].max(1e-9),
        );
    }
}
