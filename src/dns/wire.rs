use anyhow::{anyhow, bail, Context, Result};

pub(super) const MAX_MESSAGE_LEN: usize = 65_535;
const MAX_NAME_COMPONENTS: usize = 256;
const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Name(Vec<Vec<u8>>);

pub(super) fn build_query(domain: &str, query_id: u16) -> Result<Vec<u8>> {
    let mut packet = Vec::with_capacity(512);
    packet.extend_from_slice(&query_id.to_be_bytes());
    packet.extend_from_slice(&0x0100_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&[0; 6]);
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    let mut name_length = 1;
    for label in domain.split('.') {
        name_length += label.len() + 1;
        if label.is_empty() || label.len() > 63 || !label.is_ascii() || name_length > 255 {
            bail!("invalid DNS test name: {domain}");
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&TYPE_A.to_be_bytes());
    packet.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(packet)
}

/// A probe succeeds only for a complete response to this A/IN question, with an
/// address for that name or the terminal name in its CNAME chain. Negative DNS
/// answers are well-defined protocol outcomes, but cannot resolve these probes.
pub(super) fn validate_response(response: &[u8], query_id: u16, domain: &str) -> Result<()> {
    if response.len() < 12 || response.len() > MAX_MESSAGE_LEN {
        bail!("DNS response had an invalid message length");
    }
    let mut position = 0;
    if read_u16(response, &mut position)? != query_id {
        bail!("DNS response transaction ID did not match");
    }
    let flags = read_u16(response, &mut position)?;
    if flags & 0x8000 == 0 || flags & 0x7800 != 0 {
        bail!("DNS packet was not a standard query response");
    }
    if flags & 0x0200 != 0 {
        bail!("DNS response was truncated");
    }
    if flags & 0x000f != 0 {
        bail!("DNS resolver returned rcode {}", flags & 0x000f);
    }
    let questions = read_u16(response, &mut position)?;
    let answers = read_u16(response, &mut position)?;
    let authorities = read_u16(response, &mut position)?;
    let additional = read_u16(response, &mut position)?;
    if questions != 1 {
        bail!("DNS response did not contain exactly one question");
    }
    let question = read_name(response, &mut position)?;
    let question_type = read_u16(response, &mut position)?;
    let question_class = read_u16(response, &mut position)?;
    let query = build_query(domain, query_id)?;
    let mut expected_position = 12;
    let expected = read_name(&query, &mut expected_position)?;
    if question != expected || question_type != TYPE_A || question_class != CLASS_IN {
        bail!("DNS response question did not match the query");
    }

    let mut addresses = Vec::new();
    let mut aliases = Vec::new();
    let total_records = usize::from(answers) + usize::from(authorities) + usize::from(additional);
    for index in 0..total_records {
        let owner = read_name(response, &mut position)?;
        let record_type = read_u16(response, &mut position)?;
        let class = read_u16(response, &mut position)?;
        let ttl = take(response, &mut position, 4)?;
        let data_length = usize::from(read_u16(response, &mut position)?);
        let data_start = position;
        take(response, &mut position, data_length)?;
        let data_end = position;

        let alias = match record_type {
            TYPE_A if class == CLASS_IN => {
                if data_length != 4 {
                    bail!("DNS A record did not contain an IPv4 address");
                }
                if index < usize::from(answers) {
                    addresses.push(owner.clone());
                }
                None
            }
            28 if class == CLASS_IN => {
                if data_length != 16 {
                    bail!("DNS AAAA record did not contain an IPv6 address");
                }
                None
            }
            // Names in these common records must stay within their RDATA, even
            // when their compressed representation refers elsewhere in the packet.
            2 | 5 | 12 | 39 => {
                let mut cursor = data_start;
                let name = read_name(response, &mut cursor)?;
                if cursor != data_end {
                    bail!("DNS name record had an invalid data length");
                }
                (record_type == 5 && class == CLASS_IN).then_some(name)
            }
            6 => {
                let mut cursor = data_start;
                read_name(response, &mut cursor)?;
                read_name(response, &mut cursor)?;
                take(response, &mut cursor, 20)?;
                if cursor != data_end {
                    bail!("DNS SOA record had an invalid data length");
                }
                None
            }
            15 | 33 => {
                let mut cursor = data_start;
                take(response, &mut cursor, if record_type == 15 { 2 } else { 6 })?;
                read_name(response, &mut cursor)?;
                if cursor != data_end {
                    bail!("DNS service record had an invalid data length");
                }
                None
            }
            41 if ttl[0] != 0 => bail!("DNS resolver returned an extended error code"),
            _ => None,
        };
        if index < usize::from(answers) {
            if let Some(target) = alias {
                aliases.push((owner, target));
            }
        }
    }
    if position != response.len() {
        bail!("DNS response contained trailing data outside its records");
    }

    let mut current = &question;
    let mut visited = Vec::new();
    loop {
        if visited.contains(&current) {
            bail!("DNS response contained a CNAME loop");
        }
        visited.push(current);
        let mut targets = aliases
            .iter()
            .filter_map(|(owner, target)| (owner == current).then_some(target));
        if let Some(target) = targets.next() {
            if targets.any(|other| other != target) || addresses.contains(current) {
                bail!("DNS response contained conflicting CNAME answers");
            }
            current = target;
        } else if addresses.contains(current) {
            return Ok(());
        } else {
            bail!("DNS resolver returned no usable A answer for the query");
        }
    }
}

fn take<'a>(packet: &'a [u8], position: &mut usize, length: usize) -> Result<&'a [u8]> {
    let end = position
        .checked_add(length)
        .context("DNS record length overflowed")?;
    let value = packet
        .get(*position..end)
        .ok_or_else(|| anyhow!("DNS response ended inside a question or record"))?;
    *position = end;
    Ok(value)
}

fn read_u16(packet: &[u8], position: &mut usize) -> Result<u16> {
    let bytes = take(packet, position, 2)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_name(packet: &[u8], position: &mut usize) -> Result<Name> {
    let mut cursor = *position;
    let mut jumped = false;
    let mut length = 1;
    let mut labels = Vec::new();
    for _ in 0..MAX_NAME_COMPONENTS {
        let offset = cursor;
        let first = take(packet, &mut cursor, 1)?[0];
        match first & 0xc0 {
            0xc0 => {
                let second = take(packet, &mut cursor, 1)?[0];
                let target = (usize::from(first & 0x3f) << 8) | usize::from(second);
                if target < 12 || target >= offset {
                    bail!("DNS compression pointer did not reference an earlier name");
                }
                if !jumped {
                    *position = cursor;
                    jumped = true;
                }
                cursor = target;
            }
            0 => {
                if first == 0 {
                    if !jumped {
                        *position = cursor;
                    }
                    return Ok(Name(labels));
                }
                length += usize::from(first) + 1;
                if length > 255 {
                    bail!("DNS name exceeded the protocol length limit");
                }
                let label = take(packet, &mut cursor, usize::from(first))?;
                labels.push(label.to_ascii_lowercase());
            }
            _ => bail!("DNS name used an unsupported label encoding"),
        }
    }
    bail!("DNS name exceeded the compression traversal limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: u16 = 0x1234;
    const DOMAIN: &str = "example.com";
    const ADDRESS: &[u8] = &[192, 0, 2, 1];

    fn response(domain: &str, answers: u16) -> Vec<u8> {
        let mut packet = build_query(domain, ID).unwrap();
        packet[2..4].copy_from_slice(&0x8180_u16.to_be_bytes());
        packet[6..8].copy_from_slice(&answers.to_be_bytes());
        packet
    }

    fn pointer(offset: usize) -> [u8; 2] {
        assert!(offset < 0x4000);
        (0xc000 | offset as u16).to_be_bytes()
    }

    fn record(packet: &mut Vec<u8>, owner: &[u8], kind: u16, class: u16, data: &[u8]) -> usize {
        packet.extend_from_slice(owner);
        packet.extend_from_slice(&kind.to_be_bytes());
        packet.extend_from_slice(&class.to_be_bytes());
        packet.extend_from_slice(&60_u32.to_be_bytes());
        packet.extend_from_slice(&(data.len() as u16).to_be_bytes());
        let data_offset = packet.len();
        packet.extend_from_slice(data);
        data_offset
    }

    fn address_response() -> Vec<u8> {
        let mut packet = response(DOMAIN, 1);
        record(&mut packet, &pointer(12), 1, 1, ADDRESS);
        packet
    }

    fn cname_response() -> Vec<u8> {
        let mut packet = response(DOMAIN, 3);
        let alias = record(&mut packet, &pointer(12), 5, 1, b"\x03cdn\xc0\x0c");
        let target = record(&mut packet, &pointer(alias), 5, 1, b"\x04edge\xc0\x0c");
        record(&mut packet, &pointer(target), 1, 1, ADDRESS);
        packet
    }

    #[test]
    fn query_encoding_matches_a_standard_a_question() {
        assert_eq!(
            build_query(DOMAIN, ID).unwrap(),
            b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01"
        );
        assert_eq!(
            build_query("example.com.", ID).unwrap(),
            build_query(DOMAIN, ID).unwrap()
        );
        for domain in ["", ".", "example..com", "example.com..", "tést.example"] {
            assert!(build_query(domain, ID).is_err(), "{domain}");
        }
        assert!(build_query(&"x".repeat(64), ID).is_err());
        let maximum = [
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61),
        ]
        .join(".");
        assert!(build_query(&maximum, ID).is_ok());
        assert!(build_query(&format!("{maximum}d"), ID).is_err());
    }

    #[test]
    fn accepts_compressed_and_uncompressed_address_answers_case_insensitively() {
        assert!(validate_response(&address_response(), ID, DOMAIN).is_ok());
        let mut packet = response("EXAMPLE.COM", 1);
        record(&mut packet, b"\x07example\x03com\x00", 1, 1, ADDRESS);
        assert!(validate_response(&packet, ID, "Example.Com.").is_ok());
    }

    #[test]
    fn accepts_compressed_cname_chains_and_answer_order_independence() {
        assert!(validate_response(&cname_response(), ID, DOMAIN).is_ok());
        let mut packet = response(DOMAIN, 2);
        let owner = packet.len();
        record(&mut packet, b"\x03cdn\xc0\x0c", 1, 1, ADDRESS);
        record(&mut packet, &pointer(12), 5, 1, &pointer(owner));
        assert!(validate_response(&packet, ID, DOMAIN).is_ok());
    }

    #[test]
    fn rejects_every_truncated_prefix_including_a_header_claiming_answers() {
        let packet = cname_response();
        for length in 0..packet.len() {
            assert!(
                validate_response(&packet[..length], ID, DOMAIN).is_err(),
                "prefix {length}"
            );
        }
        let mut header = [0_u8; 12];
        header[..2].copy_from_slice(&ID.to_be_bytes());
        header[2..4].copy_from_slice(&0x8180_u16.to_be_bytes());
        header[6..8].copy_from_slice(&1_u16.to_be_bytes());
        assert!(validate_response(&header, ID, DOMAIN).is_err());
    }

    #[test]
    fn rejects_wrong_transaction_question_type_class_and_response_flags() {
        let original = address_response();
        let question_end = build_query(DOMAIN, ID).unwrap().len();
        let cases = [
            (0, 0xff),
            (13, b'x'),
            (question_end - 3, 28),
            (question_end - 1, 3),
            (2, 0x01), // Query rather than response.
            (2, 0x89), // Nonzero opcode.
            (2, 0x83), // Truncated response, even with complete records.
            (3, 0x82), // SERVFAIL.
            (4, 1),    // Excessive question count.
            (5, 0),    // No question.
            (5, 2),    // Two questions.
        ];
        for (offset, value) in cases {
            let mut packet = original.clone();
            packet[offset] = value;
            assert!(
                validate_response(&packet, ID, DOMAIN).is_err(),
                "byte {offset}={value}"
            );
        }
    }

    #[test]
    fn negative_dns_answers_are_not_successful_address_probes() {
        let mut no_data = response(DOMAIN, 0);
        no_data[8..10].copy_from_slice(&1_u16.to_be_bytes());
        let mut soa = b"\x02ns\xc0\x0c\x0ahostmaster\xc0\x0c".to_vec();
        soa.extend_from_slice(&[0; 20]);
        record(&mut no_data, &pointer(12), 6, 1, &soa);
        let error = validate_response(&no_data, ID, DOMAIN)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no usable A answer"), "{error}");
        no_data[3] = 0x83; // NXDOMAIN with a legitimate SOA authority record.
        assert!(validate_response(&no_data, ID, DOMAIN)
            .unwrap_err()
            .to_string()
            .contains("rcode 3"));
    }

    #[test]
    fn unrelated_answers_and_additional_glue_do_not_resolve_the_question() {
        for (owner, kind, class, data) in [
            (&b"\x05other\x03com\x00"[..], 1, 1, ADDRESS),
            (&b"\xc0\x0c"[..], 28, 1, &[0; 16][..]),
            (&b"\xc0\x0c"[..], 1, 3, ADDRESS),
            (&b"\xc0\x0c"[..], 5, 1, &b"\x03cdn\xc0\x0c"[..]),
        ] {
            let mut packet = response(DOMAIN, 1);
            record(&mut packet, owner, kind, class, data);
            assert!(validate_response(&packet, ID, DOMAIN).is_err());
        }
        let mut glue = response(DOMAIN, 0);
        glue[10..12].copy_from_slice(&1_u16.to_be_bytes());
        record(&mut glue, &pointer(12), 1, 1, ADDRESS);
        assert!(validate_response(&glue, ID, DOMAIN).is_err());
    }

    #[test]
    fn rejects_invalid_address_lengths_and_cname_data_boundaries() {
        for data in [&ADDRESS[..3], &[192, 0, 2, 1, 9][..]] {
            let mut packet = response(DOMAIN, 1);
            record(&mut packet, &pointer(12), 1, 1, data);
            assert!(validate_response(&packet, ID, DOMAIN).is_err());
        }
        let mut packet = cname_response();
        let cname_length = build_query(DOMAIN, ID).unwrap().len() + 10;
        for length in [0_u16, 1, 5, 7, u16::MAX] {
            packet[cname_length..cname_length + 2].copy_from_slice(&length.to_be_bytes());
            assert!(
                validate_response(&packet, ID, DOMAIN).is_err(),
                "RDLENGTH {length}"
            );
        }
    }

    #[test]
    fn rejects_compression_loops_forward_pointers_and_invalid_labels() {
        let original = address_response();
        let owner = build_query(DOMAIN, ID).unwrap().len();
        for name in [
            pointer(owner),
            pointer(owner + 2),
            pointer(0x3fff),
            pointer(1),
            [0x40, 0],
        ] {
            let mut packet = original.clone();
            packet[owner..owner + 2].copy_from_slice(&name);
            assert!(
                validate_response(&packet, ID, DOMAIN).is_err(),
                "name {name:?}"
            );
        }
        let mut loop_packet = response(DOMAIN, 1);
        let mut looping_name = b"\x01a".to_vec();
        looping_name.extend_from_slice(&pointer(owner));
        record(&mut loop_packet, &looping_name, 1, 1, ADDRESS);
        assert!(validate_response(&loop_packet, ID, DOMAIN).is_err());
    }

    #[test]
    fn bounds_compression_work_and_rejects_oversized_names() {
        let mut packet = vec![0; 13];
        let mut previous = 12;
        for _ in 0..MAX_NAME_COMPONENTS {
            let next = packet.len();
            packet.extend_from_slice(&pointer(previous));
            previous = next;
        }
        assert!(read_name(&packet, &mut previous).is_err());
        let mut long_name = Vec::new();
        for _ in 0..4 {
            long_name.push(63);
            long_name.extend_from_slice(&[b'x'; 63]);
        }
        long_name.push(0);
        let mut packet = response(DOMAIN, 1);
        record(&mut packet, &long_name, 1, 1, ADDRESS);
        assert!(validate_response(&packet, ID, DOMAIN).is_err());
    }

    #[test]
    fn rejects_cname_cycles_and_conflicting_targets() {
        let mut cycle = response(DOMAIN, 2);
        let alias = record(&mut cycle, &pointer(12), 5, 1, b"\x03cdn\xc0\x0c");
        record(&mut cycle, &pointer(alias), 5, 1, &pointer(12));
        assert!(validate_response(&cycle, ID, DOMAIN)
            .unwrap_err()
            .to_string()
            .contains("CNAME loop"));
        let mut conflict = response(DOMAIN, 2);
        record(&mut conflict, &pointer(12), 5, 1, b"\x03cdn\xc0\x0c");
        record(&mut conflict, &pointer(12), 5, 1, b"\x04edge\xc0\x0c");
        assert!(validate_response(&conflict, ID, DOMAIN)
            .unwrap_err()
            .to_string()
            .contains("conflicting CNAME"));
    }

    #[test]
    fn validates_authority_additional_and_complete_message_boundaries() {
        let mut packet = address_response();
        packet[8..10].copy_from_slice(&1_u16.to_be_bytes());
        let nameserver = record(&mut packet, &pointer(12), 2, 1, b"\x02ns\xc0\x0c");
        packet[10..12].copy_from_slice(&1_u16.to_be_bytes());
        record(&mut packet, &pointer(nameserver), 1, 1, ADDRESS);
        assert!(validate_response(&packet, ID, DOMAIN).is_ok());
        packet.pop();
        assert!(validate_response(&packet, ID, DOMAIN).is_err());
        let mut trailing = address_response();
        trailing.push(0);
        assert!(validate_response(&trailing, ID, DOMAIN).is_err());
        let mut excessive_count = address_response();
        excessive_count[6..8].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(validate_response(&excessive_count, ID, DOMAIN).is_err());
        assert!(validate_response(&vec![0; MAX_MESSAGE_LEN + 1], ID, DOMAIN).is_err());
    }

    #[test]
    fn accepts_edns_metadata_but_rejects_extended_error_codes() {
        let mut packet = address_response();
        packet[10..12].copy_from_slice(&1_u16.to_be_bytes());
        let data = record(&mut packet, &[0], 41, 1232, &[]);
        assert!(validate_response(&packet, ID, DOMAIN).is_ok());
        packet[data - 6] = 1;
        assert!(validate_response(&packet, ID, DOMAIN).is_err());
    }
}
