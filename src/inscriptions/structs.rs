use bellscoin::Script;

use super::*;

#[derive(Debug, PartialEq, Clone)]
pub struct Inscription {
    pub body: Option<Vec<u8>>,
    pub content_type: Option<Vec<u8>>,
    pub content_encoding: Option<Vec<u8>>,
    pub delegate: Option<Vec<u8>>,
    pub duplicate_field: bool,
    pub incomplete_field: bool,
    pub metadata: Option<Vec<u8>>,
    pub metaprotocol: Option<Vec<u8>>,
    pub parents: Vec<Vec<u8>>,
    pub pointer: Option<Vec<u8>>,
    pub rune: Option<Vec<u8>>,
    pub unrecognized_even_field: bool,
}

#[derive(Debug, PartialEq)]
pub enum ParsedInscription {
    None,
    Partial,
    Single(Inscription),
    Many(Vec<Inscription>),
}

impl Inscription {
    pub fn from_parts(partials: &[Part], vout: u32) -> ParsedInscription {
        let mut sig_scripts = Vec::with_capacity(partials.len());

        if partials.len() == 1 {
            let part = partials.first().expect("Part must exist, checked above");
            let script = Script::from_bytes(&part.script_buffer);
            if let Result::Ok(v) = RawEnvelope::from_tapscript(script, vout) {
                let data = v
                    .into_iter()
                    .map(ParsedEnvelope::from)
                    .map(|x| x.payload)
                    .collect();

                return ParsedInscription::Many(data);
            }

            return ParsedInscription::None;
        }

        for partial in partials {
            sig_scripts.push(Script::from_bytes(&partial.script_buffer));
        }

        InscriptionParser::parse(sig_scripts)
    }

    pub fn into_body(self) -> Option<Vec<u8>> {
        self.body
    }

    pub fn content_type(&self) -> Option<&str> {
        core::str::from_utf8(self.content_type.as_ref()?).ok()
    }

    pub fn pointer(&self) -> Option<u64> {
        let value = self.pointer.as_ref()?;

        if value.iter().skip(8).copied().any(|byte| byte != 0) {
            return None;
        }

        let pointer = [
            value.first().copied().unwrap_or(0),
            value.get(1).copied().unwrap_or(0),
            value.get(2).copied().unwrap_or(0),
            value.get(3).copied().unwrap_or(0),
            value.get(4).copied().unwrap_or(0),
            value.get(5).copied().unwrap_or(0),
            value.get(6).copied().unwrap_or(0),
            value.get(7).copied().unwrap_or(0),
        ];

        Some(u64::from_le_bytes(pointer))
    }
}

struct InscriptionParser {}

impl InscriptionParser {
    fn parse(sig_scripts: Vec<&script::Script>) -> ParsedInscription {
        let sig_script = &sig_scripts[0];

        let mut push_datas_vec = match Self::decode_push_datas(sig_script) {
            Some(push_datas) => push_datas,
            None => return ParsedInscription::None,
        };

        let mut push_datas = push_datas_vec.as_slice();

        // read protocol

        if push_datas.len() < 3 {
            return ParsedInscription::None;
        }

        let protocol = &push_datas[0];

        if protocol != PROTOCOL_ID {
            return ParsedInscription::None;
        }

        // read npieces

        let mut npieces = match Self::push_data_to_number(&push_datas[1]) {
            Some(n) => n,
            None => return ParsedInscription::None,
        };

        if npieces == 0 {
            return ParsedInscription::None;
        }

        // read content type

        let content_type = push_datas[2].clone();

        push_datas = &push_datas[3..];

        // read body

        let mut body = vec![];

        let mut sig_scripts = sig_scripts.as_slice();

        // loop over transactions
        loop {
            // loop over chunks
            loop {
                if npieces == 0 {
                    let inscription = Inscription {
                        content_type: Some(content_type),
                        body: Some(body),
                        content_encoding: None,
                        delegate: None,
                        duplicate_field: false,
                        incomplete_field: false,
                        metadata: None,
                        metaprotocol: None,
                        parents: vec![],
                        pointer: None,
                        rune: None,
                        unrecognized_even_field: false,
                    };

                    return ParsedInscription::Single(inscription);
                }

                if push_datas.len() < 2 {
                    break;
                }

                let next = match Self::push_data_to_number(&push_datas[0]) {
                    Some(n) => n,
                    None => break,
                };

                if next != npieces - 1 {
                    break;
                }

                body.append(&mut push_datas[1].clone());

                push_datas = &push_datas[2..];
                npieces -= 1;
            }

            if sig_scripts.len() <= 1 {
                return ParsedInscription::Partial;
            }

            sig_scripts = &sig_scripts[1..];

            push_datas_vec = match Self::decode_push_datas(sig_scripts[0]) {
                Some(push_datas) => push_datas,
                None => return ParsedInscription::None,
            };

            if push_datas_vec.len() < 2 {
                return ParsedInscription::None;
            }

            let next = match Self::push_data_to_number(&push_datas_vec[0]) {
                Some(n) => n,
                None => return ParsedInscription::None,
            };

            if next != npieces - 1 {
                return ParsedInscription::None;
            }

            push_datas = push_datas_vec.as_slice();
        }
    }

    fn decode_push_datas(script: &script::Script) -> Option<Vec<Vec<u8>>> {
        let mut bytes = script.as_bytes();
        let mut push_datas = vec![];

        while !bytes.is_empty() {
            // op_0
            if bytes[0] == 0 {
                push_datas.push(vec![]);
                bytes = &bytes[1..];
                continue;
            }

            // op_1 - op_16
            if bytes[0] >= 81 && bytes[0] <= 96 {
                push_datas.push(vec![bytes[0] - 80]);
                bytes = &bytes[1..];
                continue;
            }

            // op_push 1-75
            if bytes[0] >= 1 && bytes[0] <= 75 {
                let len = bytes[0] as usize;
                if bytes.len() < 1 + len {
                    return None;
                }
                push_datas.push(bytes[1..1 + len].to_vec());
                bytes = &bytes[1 + len..];
                continue;
            }

            // op_pushdata1
            if bytes[0] == 76 {
                if bytes.len() < 2 {
                    return None;
                }
                let len = bytes[1] as usize;
                if bytes.len() < 2 + len {
                    return None;
                }
                push_datas.push(bytes[2..2 + len].to_vec());
                bytes = &bytes[2 + len..];
                continue;
            }

            // op_pushdata2
            if bytes[0] == 77 {
                if bytes.len() < 3 {
                    return None;
                }
                let len = ((bytes[1] as usize) << 8) + (bytes[0] as usize);
                if bytes.len() < 3 + len {
                    return None;
                }
                push_datas.push(bytes[3..3 + len].to_vec());
                bytes = &bytes[3 + len..];
                continue;
            }

            // op_pushdata4
            if bytes[0] == 78 {
                if bytes.len() < 5 {
                    return None;
                }
                let len = ((bytes[3] as usize) << 24)
                    + ((bytes[2] as usize) << 16)
                    + ((bytes[1] as usize) << 8)
                    + (bytes[0] as usize);
                if bytes.len() < 5 + len {
                    return None;
                }
                push_datas.push(bytes[5..5 + len].to_vec());
                bytes = &bytes[5 + len..];
                continue;
            }

            return None;
        }

        Some(push_datas)
    }

    fn push_data_to_number(data: &[u8]) -> Option<u64> {
        if data.is_empty() {
            return Some(0);
        }

        if data.len() > 8 {
            return None;
        }

        let mut n: u64 = 0;
        let mut m: u64 = 0;

        for i in data {
            n += (*i as u64) << m;
            m += 8;
        }

        Some(n)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Copy)]
pub struct Location {
    pub outpoint: OutPoint,
    pub offset: u64,
}

impl Location {
    pub fn zero() -> Self {
        Self {
            offset: 0,
            outpoint: OutPoint {
                txid: Txid::all_zeros(),
                vout: 0,
            },
        }
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}i{}i{}",
            self.outpoint.txid, self.outpoint.vout, self.offset
        ))
    }
}

impl FromStr for Location {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let mut items = s.split(':');

        let error_msg = "Invalid location";

        let txid =
            Txid::from_str(items.next().anyhow_with(error_msg)?).anyhow_with("Invalid txid")?;
        let vout: u32 = items
            .next()
            .anyhow_with(error_msg)?
            .parse()
            .anyhow_with("Invalid vout")?;
        let offset: u64 = items
            .next()
            .anyhow_with(error_msg)?
            .parse()
            .anyhow_with("Invalid offset")?;

        Ok(Self {
            offset,
            outpoint: OutPoint { txid, vout },
        })
    }
}

#[derive(Clone)]
pub struct Part {
    pub is_tapscript: bool,
    pub script_buffer: Vec<u8>,
}

#[derive(Clone)]
pub struct Partials {
    pub inscription_index: u32,
    pub genesis_txid: Txid,
    pub parts: Vec<Part>,
}

impl db::Pebble for Partials {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut buffer = vec![];
        buffer.extend(v.inscription_index.to_be_bytes().to_vec());
        buffer.extend_from_slice(&bellscoin::consensus::serialize(&v.genesis_txid));

        for part in &v.parts {
            buffer.extend([part.is_tapscript as u8]);
            let script_len = part.script_buffer.len() as u32;
            buffer.extend(script_len.to_be_bytes().to_vec());
            buffer.extend(part.script_buffer.clone());
        }

        Cow::Owned(buffer)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let inscription_index = u32::from_be_bytes(v[..4].try_into()?);
        let genesis_txid: Txid = bellscoin::consensus::deserialize(&v[4..36])?;
        let mut parts = vec![];
        let mut offset = 4 + 32;
        while offset != v.len() {
            let is_tapscript = v[offset] == 1;
            offset += 1;
            let script_len = u32::from_be_bytes(v[offset..offset + 4].try_into()?) as usize;
            offset += 4;
            let script_buffer = v[offset..offset + script_len].to_vec();

            parts.push(Part {
                is_tapscript,
                script_buffer,
            });
        }

        Ok(Self {
            genesis_txid,
            inscription_index,
            parts,
        })
    }
}
