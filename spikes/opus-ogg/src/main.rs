//! Writes the Objects of an Opus track of LOC as an Ogg Opus file (RFC 7845),
//! without decoding them.
//!
//!   xmoq-moqt-client --mode subscribe --objects 250 ... --track <an Opus track> \
//!       | opus-ogg out.opus [channels, 2]
//!
//! It reads the `OBJECT ... payload_hex=...` lines of the client: the payload
//! of an Object of a LOC audio track is one Opus packet. Each packet goes
//! into an Ogg page of its own. How many samples a packet has is read from
//! its first byte (RFC 6716, §3.1). The pre-skip is written as 312 samples,
//! which is a guess: nothing in LOC or in the catalog says what the encoder
//! used.

use std::io::{BufRead, Write};

/// The CRC of Ogg: polynomial 0x04c11db7, not reflected, from 0.
fn crc(bytes: &[u8]) -> u32 {
    let mut crc = 0u32;
    for byte in bytes {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
        }
    }
    crc
}

/// One Ogg page with one packet.
fn page(packet: &[u8], granule: u64, sequence: u32, flags: u8) -> Vec<u8> {
    let mut out = b"OggS\0".to_vec();
    out.push(flags);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&0x786d_6f71u32.to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    // The lacing: 255 for every full 255 bytes, and what is left, also 0.
    let mut lacing = vec![255u8; packet.len() / 255];
    lacing.push((packet.len() % 255) as u8);
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    out.extend_from_slice(packet);
    let sum = crc(&out);
    out[22..26].copy_from_slice(&sum.to_le_bytes());
    out
}

/// The samples of a packet at 48 kHz, from its TOC byte (RFC 6716, §3.1).
fn samples(packet: &[u8]) -> u64 {
    let Some(toc) = packet.first() else { return 0 };
    let config = toc >> 3;
    let a_frame = match config {
        0..=11 => [480, 960, 1920, 2880][usize::from(config % 4)],
        12..=15 => [480, 960][usize::from(config % 2)],
        _ => [120, 240, 480, 960][usize::from(config % 4)],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => u64::from(packet.get(1).copied().unwrap_or(0) & 0x3f),
    };
    a_frame * frames
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).filter_map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok()).collect()
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("the file to write");
    let channels: u8 = args.next().map_or(2, |text| text.parse().expect("channels"));
    let mut out = std::fs::File::create(&path)?;
    let mut head = b"OpusHead\x01".to_vec();
    head.push(channels);
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    out.write_all(&page(&head, 0, 0, 2))?;
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&4u32.to_le_bytes());
    tags.extend_from_slice(b"xmoq");
    tags.extend_from_slice(&0u32.to_le_bytes());
    out.write_all(&page(&tags, 0, 1, 0))?;
    let (mut granule, mut packets, mut bytes) = (0u64, 0u32, 0usize);
    let mut first_timestamp = None;
    let mut last_timestamp = None;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if !line.starts_with("OBJECT ") {
            continue;
        }
        let Some(hex) = line.rsplit("payload_hex=").next() else { continue };
        let packet = unhex(hex.trim());
        if packet.is_empty() {
            continue;
        }
        // properties=16:<Timestamp of LOC>, where the Object has one.
        let timestamp = line
            .split("properties=")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|pairs| pairs.split(',').find_map(|pair| pair.strip_prefix("16:")))
            .and_then(|value| value.parse::<u64>().ok());
        first_timestamp = first_timestamp.or(timestamp);
        last_timestamp = timestamp.or(last_timestamp);
        granule += samples(&packet);
        packets += 1;
        bytes += packet.len();
        out.write_all(&page(&packet, granule, packets + 1, 0))?;
    }
    eprintln!(
        "OPUS_OGG file={path} packets={packets} bytes={bytes} samples={granule} seconds={:.2} timestamps_span_us={}",
        granule as f64 / 48_000.0,
        match (first_timestamp, last_timestamp) {
            (Some(first), Some(last)) => (last - first).to_string(),
            _ => "none".to_string(),
        }
    );
    Ok(())
}
