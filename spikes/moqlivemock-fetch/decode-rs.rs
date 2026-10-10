use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, RangeEnd};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::params::GroupOrder;

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn main() {
    let path = std::env::args().nth(1).unwrap();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts[0] == "message" {
            let bytes = unhex(parts[2]);
            println!("{} {:?}", parts[1], ControlMessage::decode(&bytes));
            continue;
        }
        let order = if parts[1] == "descending" { GroupOrder::Descending } else { GroupOrder::Ascending };
        let mut decoder = FetchStreamDecoder::new(1 << 20, order);
        let mut out = Vec::new();
        for byte in unhex(parts[2]) {
            decoder.push(&[byte]).unwrap();
            loop {
                match decoder.next_event() {
                    Ok(Some(FetchEvent::Header(h))) => out.push(format!("header({})", h.request_id)),
                    Ok(Some(FetchEvent::EndOfRange { kind, location })) => out.push(format!(
                        "end({},{},{})",
                        match kind { RangeEnd::NonExistent => "NonExistent", RangeEnd::Unknown => "Unknown" },
                        location.group, location.object
                    )),
                    Ok(Some(FetchEvent::Object(o))) => out.push(format!(
                        "obj({},{},sub={},prio={},len={})",
                        o.group_id, o.object_id,
                        o.subgroup_id.map_or("none".to_string(), |s| s.to_string()),
                        o.publisher_priority, o.payload.len()
                    )),
                    Ok(None) => break,
                    Err(error) => { out.push(format!("FAILURE {error:?}")); break; }
                }
            }
        }
        let fin = decoder.finish().err().map_or(String::new(), |e| format!("FIN-ERROR {e:?}"));
        println!("{} {} {}", parts[0], out.join(" "), fin);
    }
}
