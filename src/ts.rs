//! Minimal MPEG-TS demuxer.
//!
//! Piped's HLS segments for YouTube Live are 188-byte MPEG-TS packets
//! muxing H.264 video and ADTS-framed AAC audio. The `symphonia` crate
//! used for decoding does not support MPEG-TS containers, only raw
//! elementary streams, so we strip the TS framing here and return a
//! contiguous AAC ADTS byte stream.
//!
//! Scope is intentionally narrow: single-program TS, first audio stream
//! of type 0x0F (ADTS AAC). This is what YouTube Live delivers; anything
//! else is an error.

use anyhow::{anyhow, Result};

const TS_PACKET_SIZE: usize = 188;
const SYNC_BYTE: u8 = 0x47;

// Stream type 0x0F = ISO/IEC 13818-7 AAC (ADTS transport).
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;

/// Demultiplex an MPEG-TS bytestream and return the concatenated payload
/// bytes of the first AAC-ADTS audio stream.
pub fn extract_aac_from_ts(data: &[u8]) -> Result<Vec<u8>> {
    let mut pmt_pid: Option<u16> = None;
    let mut audio_pid: Option<u16> = None;
    let mut out = Vec::with_capacity(data.len() / 4);

    for packet in data.chunks_exact(TS_PACKET_SIZE) {
        if packet[0] != SYNC_BYTE {
            continue;
        }
        let pusi = (packet[1] & 0x40) != 0;
        let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;
        let payload = match payload_of(packet) {
            Some(p) => p,
            None => continue,
        };

        if pid == 0 {
            if let Some(pmt) = parse_pat_first_pmt(payload, pusi) {
                pmt_pid = Some(pmt);
            }
        } else if Some(pid) == pmt_pid && audio_pid.is_none() {
            if let Some(aud) = parse_pmt_first_audio(payload, pusi) {
                audio_pid = Some(aud);
            }
        } else if Some(pid) == audio_pid {
            let es = if pusi {
                match strip_pes_header(payload) {
                    Ok(es) => es,
                    Err(_) => continue,
                }
            } else {
                payload
            };
            out.extend_from_slice(es);
        }
    }

    if audio_pid.is_none() {
        return Err(anyhow!("no AAC audio PID found in TS stream"));
    }
    if out.is_empty() {
        return Err(anyhow!("audio PID carried no payload"));
    }
    Ok(out)
}

/// Return the payload slice of a TS packet, skipping any adaptation field.
/// Returns `None` if the packet carries no payload or is malformed.
fn payload_of(packet: &[u8]) -> Option<&[u8]> {
    let af_ctl = (packet[3] >> 4) & 0x03;
    let has_af = af_ctl & 0b10 != 0;
    let has_payload = af_ctl & 0b01 != 0;
    if !has_payload {
        return None;
    }
    let mut idx = 4;
    if has_af {
        let af_len = packet[4] as usize;
        idx = 5 + af_len;
        if idx > TS_PACKET_SIZE {
            return None;
        }
    }
    Some(&packet[idx..])
}

/// Parse a PAT payload and return the PMT PID of the first non-zero
/// program. Returns `None` if the section is malformed.
fn parse_pat_first_pmt(payload: &[u8], pusi: bool) -> Option<u16> {
    let section = section_start(payload, pusi)?;
    // PAT section header is 8 bytes; CRC32 is last 4 bytes of the section.
    let section_length = (((section.get(1)? & 0x0F) as usize) << 8) | *section.get(2)? as usize;
    let entries_end = 3 + section_length.checked_sub(4)?;
    let mut i = 8;
    while i + 4 <= entries_end && i + 4 <= section.len() {
        let program = ((section[i] as u16) << 8) | section[i + 1] as u16;
        let pmt = (((section[i + 2] & 0x1F) as u16) << 8) | section[i + 3] as u16;
        if program != 0 {
            return Some(pmt);
        }
        i += 4;
    }
    None
}

/// Parse a PMT payload and return the PID of the first AAC-ADTS
/// elementary stream.
fn parse_pmt_first_audio(payload: &[u8], pusi: bool) -> Option<u16> {
    let section = section_start(payload, pusi)?;
    let section_length = (((section.get(1)? & 0x0F) as usize) << 8) | *section.get(2)? as usize;
    let program_info_length =
        (((section.get(10)? & 0x0F) as usize) << 8) | *section.get(11)? as usize;
    let mut i = 12 + program_info_length;
    let end = 3 + section_length.checked_sub(4)?;
    while i + 5 <= end && i + 5 <= section.len() {
        let stream_type = section[i];
        let pid = (((section[i + 1] & 0x1F) as u16) << 8) | section[i + 2] as u16;
        let es_info_length =
            (((section[i + 3] & 0x0F) as usize) << 8) | section[i + 4] as usize;
        if stream_type == STREAM_TYPE_AAC_ADTS {
            return Some(pid);
        }
        i += 5 + es_info_length;
    }
    None
}

/// Given a TS payload whose packet's PUSI bit is set (section starts in
/// this payload), return the section bytes after the pointer field.
fn section_start(payload: &[u8], pusi: bool) -> Option<&[u8]> {
    if !pusi {
        return Some(payload);
    }
    let ptr = *payload.first()? as usize;
    payload.get(1 + ptr..)
}

/// Strip a PES packet header from `payload` and return the elementary
/// stream bytes. Expects `payload` to begin with the PES start code.
fn strip_pes_header(payload: &[u8]) -> Result<&[u8]> {
    if payload.len() < 9 || payload[0..3] != [0x00, 0x00, 0x01] {
        return Err(anyhow!("missing PES start code"));
    }
    let pes_header_data_length = payload[8] as usize;
    let header_end = 9 + pes_header_data_length;
    if header_end > payload.len() {
        return Err(anyhow!("PES header length exceeds packet"));
    }
    Ok(&payload[header_end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a single TS packet. Short payloads are padded with an
    /// adaptation-field stuffing area so the emitted packet matches what
    /// real muxers produce — the demuxer must skip over stuffing and
    /// extract exactly the `payload` bytes.
    fn ts_packet(pid: u16, pusi: bool, cc: u8, payload: &[u8]) -> [u8; TS_PACKET_SIZE] {
        const SPACE: usize = TS_PACKET_SIZE - 4;
        assert!(payload.len() <= SPACE, "payload too big for single TS packet");
        let mut p = [0xFFu8; TS_PACKET_SIZE];
        p[0] = SYNC_BYTE;
        p[1] = ((pusi as u8) << 6) | (((pid >> 8) as u8) & 0x1F);
        p[2] = (pid & 0xFF) as u8;
        if payload.len() == SPACE {
            // adaptation_field_control = 01 (payload only)
            p[3] = 0x10 | (cc & 0x0F);
            p[4..].copy_from_slice(payload);
        } else {
            // adaptation_field_control = 11 (AF + payload); stuff with AF.
            p[3] = 0x30 | (cc & 0x0F);
            let af_length = SPACE - 1 - payload.len();
            p[4] = af_length as u8;
            if af_length >= 1 {
                p[5] = 0x00; // AF flags: nothing set; remainder is stuffing (0xFF).
            }
            let payload_start = 5 + af_length;
            p[payload_start..].copy_from_slice(payload);
        }
        p
    }

    fn pat_payload_for(program: u16, pmt_pid: u16) -> Vec<u8> {
        // pointer_field=0, then PAT section:
        //   table_id=0x00, sec_syn+flags+length(2), tsid(2), flags+version(1),
        //   section_number(1), last_section_number(1),
        //   program(2), reserved+pmt_pid(2), CRC32(4)
        // section_length counts bytes AFTER the length field, so:
        //   tsid(2)+flags(1)+secno(1)+lastsec(1)+program(2)+pmt(2)+crc(4) = 13
        let section_length: u16 = 13;
        let mut v = vec![0x00u8]; // pointer_field
        v.push(0x00); // table_id = PAT
        v.push(0xB0 | ((section_length >> 8) & 0x0F) as u8);
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&[0x00, 0x01]); // tsid
        v.push(0xC1); // reserved + version=0 + current_next=1
        v.push(0x00); // section_number
        v.push(0x00); // last_section_number
        v.extend_from_slice(&program.to_be_bytes());
        let pmt_word: u16 = 0xE000 | (pmt_pid & 0x1FFF);
        v.extend_from_slice(&pmt_word.to_be_bytes());
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // dummy CRC
        v
    }

    fn pmt_payload_for(streams: &[(u8, u16)]) -> Vec<u8> {
        // Each stream: stream_type(1) + reserved+pid(2) + reserved+es_info_len(2) = 5 bytes
        let streams_len: u16 = (streams.len() * 5) as u16;
        // section after length field:
        //   program(2) + flags(1) + secno(1) + lastsec(1) + pcr_pid(2)
        //   + program_info_length(2) + streams(N*5) + CRC(4)
        let section_length: u16 = 9 + streams_len + 4;
        let mut v = vec![0x00u8]; // pointer_field
        v.push(0x02); // table_id = PMT
        v.push(0xB0 | ((section_length >> 8) & 0x0F) as u8);
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&[0x00, 0x01]); // program_number
        v.push(0xC5); // reserved + version=2 + current_next=1
        v.push(0x00); // section_number
        v.push(0x00); // last_section_number
        v.extend_from_slice(&[0xFF, 0xFF]); // pcr_pid (none)
        v.extend_from_slice(&[0xF0, 0x00]); // program_info_length = 0
        for (stype, pid) in streams {
            v.push(*stype);
            let pid_word: u16 = 0xE000 | (pid & 0x1FFF);
            v.extend_from_slice(&pid_word.to_be_bytes());
            v.extend_from_slice(&[0xF0, 0x00]); // es_info_length = 0
        }
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // dummy CRC
        v
    }

    /// PES header wrapping a small payload; no optional fields.
    fn pes_wrap(aac: &[u8]) -> Vec<u8> {
        let mut v = vec![0x00, 0x00, 0x01]; // start code
        v.push(0xC0); // stream_id = audio (0xC0..0xDF)
        let pes_packet_length: u16 = (3 + aac.len()) as u16; // header_data_length(1)+flags(2)+data
        v.extend_from_slice(&pes_packet_length.to_be_bytes());
        v.push(0x80); // '10' marker + flags
        v.push(0x00); // no PTS/DTS flags
        v.push(0x00); // PES_header_data_length = 0
        v.extend_from_slice(aac);
        v
    }

    #[test]
    fn payload_of_skips_adaptation_field() {
        // Packet with both AF + payload, AF length = 2, payload = [0xAA, 0xBB]
        let mut p = [0u8; 188];
        p[0] = SYNC_BYTE;
        p[1] = 0x00;
        p[2] = 0x10;
        p[3] = 0x30; // adaptation=11 (both)
        p[4] = 0x02; // AF length
        p[5] = 0x00;
        p[6] = 0x00;
        p[7] = 0xAA;
        p[8] = 0xBB;
        let payload = payload_of(&p).unwrap();
        assert_eq!(&payload[..2], &[0xAA, 0xBB]);
    }

    #[test]
    fn payload_of_returns_none_when_af_only() {
        let mut p = [0u8; 188];
        p[0] = SYNC_BYTE;
        p[3] = 0x20; // adaptation=10 (AF only)
        p[4] = 0x01;
        assert!(payload_of(&p).is_none());
    }

    #[test]
    fn parse_pat_finds_pmt_pid() {
        let pat = pat_payload_for(1, 0x1234);
        assert_eq!(parse_pat_first_pmt(&pat, true), Some(0x1234));
    }

    #[test]
    fn parse_pmt_finds_first_aac_pid() {
        // Mix of video (0x1B) and audio (0x0F) — audio must win.
        let pmt = pmt_payload_for(&[(0x1B, 0x100), (STREAM_TYPE_AAC_ADTS, 0x101)]);
        assert_eq!(parse_pmt_first_audio(&pmt, true), Some(0x101));
    }

    #[test]
    fn parse_pmt_returns_none_when_no_aac() {
        let pmt = pmt_payload_for(&[(0x1B, 0x100)]);
        assert!(parse_pmt_first_audio(&pmt, true).is_none());
    }

    #[test]
    fn strip_pes_header_returns_es_payload() {
        let wrapped = pes_wrap(&[0xFF, 0xF1, 0x50, 0x80, 0x10, 0x1F, 0xFC]);
        let es = strip_pes_header(&wrapped).unwrap();
        assert_eq!(es, &[0xFF, 0xF1, 0x50, 0x80, 0x10, 0x1F, 0xFC]);
    }

    #[test]
    fn strip_pes_header_errors_without_start_code() {
        assert!(strip_pes_header(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09]).is_err());
    }

    // Regression: before this demuxer existed, MPEG-TS segment data was
    // fed directly to symphonia's AAC reader, which choked on the 0x47
    // sync bytes and produced `IoError("end of stream")`. This test
    // constructs a minimal well-formed TS stream (PAT + PMT + audio PES)
    // and verifies that the demuxer recovers the inner ADTS payload.
    #[test]
    fn extract_aac_end_to_end_from_synthetic_ts() {
        let pmt_pid: u16 = 0x100;
        let audio_pid: u16 = 0x101;
        let aac_payload = vec![0xFF, 0xF1, 0x50, 0x80, 0x10, 0x1F, 0xFC, 0xAA, 0xBB];

        let mut ts = Vec::new();
        ts.extend_from_slice(&ts_packet(0, true, 0, &pat_payload_for(1, pmt_pid)));
        ts.extend_from_slice(&ts_packet(
            pmt_pid,
            true,
            0,
            &pmt_payload_for(&[(0x1B, 0x200), (STREAM_TYPE_AAC_ADTS, audio_pid)]),
        ));
        ts.extend_from_slice(&ts_packet(audio_pid, true, 0, &pes_wrap(&aac_payload)));

        let got = extract_aac_from_ts(&ts).unwrap();
        assert_eq!(got, aac_payload);
    }

    #[test]
    fn extract_aac_errors_when_no_audio_pid() {
        let pmt_pid: u16 = 0x100;
        let mut ts = Vec::new();
        ts.extend_from_slice(&ts_packet(0, true, 0, &pat_payload_for(1, pmt_pid)));
        // PMT with only video, no AAC entry:
        ts.extend_from_slice(&ts_packet(pmt_pid, true, 0, &pmt_payload_for(&[(0x1B, 0x200)])));
        assert!(extract_aac_from_ts(&ts).is_err());
    }

    #[test]
    fn extract_aac_errors_on_empty_input() {
        assert!(extract_aac_from_ts(&[]).is_err());
    }

    #[test]
    fn extract_aac_concatenates_multiple_pes_continuations() {
        // Continuation packets (PUSI=0) have no PES header and should be
        // appended verbatim to the prior PES payload.
        let pmt_pid: u16 = 0x100;
        let audio_pid: u16 = 0x101;
        let chunk1 = vec![0xFF, 0xF1, 0x50, 0x80];
        let chunk2 = vec![0x10, 0x1F, 0xFC, 0xAA, 0xBB];

        let mut ts = Vec::new();
        ts.extend_from_slice(&ts_packet(0, true, 0, &pat_payload_for(1, pmt_pid)));
        ts.extend_from_slice(&ts_packet(
            pmt_pid,
            true,
            0,
            &pmt_payload_for(&[(STREAM_TYPE_AAC_ADTS, audio_pid)]),
        ));
        ts.extend_from_slice(&ts_packet(audio_pid, true, 0, &pes_wrap(&chunk1)));
        ts.extend_from_slice(&ts_packet(audio_pid, false, 1, &chunk2));

        let got = extract_aac_from_ts(&ts).unwrap();
        let mut expected = chunk1.clone();
        expected.extend(&chunk2);
        assert_eq!(got, expected);
    }
}
