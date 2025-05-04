use super::*;

pub type RawEnvelope = Envelope<Vec<Vec<u8>>>;
pub type ParsedEnvelope = Envelope<Inscription>;
type Result<T> = std::result::Result<T, script::Error>;

use bellscoin::blockdata::script::Instruction::{Op, PushBytes};

#[derive(Default, PartialEq, Clone, Serialize, Deserialize, Debug, Eq)]
pub struct Envelope<T> {
    pub input: u32,
    pub offset: u32,
    pub payload: T,
    pub pushnum: bool,
    pub stutter: bool,
}

impl From<RawEnvelope> for ParsedEnvelope {
    fn from(envelope: RawEnvelope) -> Self {
        let body = envelope
            .payload
            .iter()
            .enumerate()
            .position(|(i, push)| i % 2 == 0 && push.is_empty());

        let mut fields: BTreeMap<&[u8], Vec<&[u8]>> = BTreeMap::new();

        let mut incomplete_field = false;

        for item in envelope.payload[..body.unwrap_or(envelope.payload.len())].chunks(2) {
            match item {
                [key, value] => fields.entry(key).or_default().push(value),
                _ => incomplete_field = true,
            }
        }

        let duplicate_field = fields.iter().any(|(_key, values)| values.len() > 1);

        let content_encoding = Tag::ContentEncoding.take(&mut fields);
        let content_type = Tag::ContentType.take(&mut fields);
        let delegate = Tag::Delegate.take(&mut fields);
        let metadata = Tag::Metadata.take(&mut fields);
        let metaprotocol = Tag::Metaprotocol.take(&mut fields);
        let parents = Tag::Parent.take_array(&mut fields);
        let pointer = Tag::Pointer.take(&mut fields);
        let rune = Tag::Rune.take(&mut fields);

        let unrecognized_even_field = fields
            .keys()
            .any(|tag| tag.first().map(|lsb| lsb % 2 == 0).unwrap_or_default());

        Self {
            payload: Inscription {
                body: body.map(|i| {
                    envelope.payload[i + 1..]
                        .iter()
                        .flatten()
                        .cloned()
                        .collect()
                }),
                content_encoding,
                content_type,
                delegate,
                duplicate_field,
                incomplete_field,
                metadata,
                metaprotocol,
                parents,
                pointer,
                rune,
                unrecognized_even_field,
            },
            input: envelope.input,
            offset: envelope.offset,
            pushnum: envelope.pushnum,
            stutter: envelope.stutter,
        }
    }
}

impl RawEnvelope {
    pub fn from_tapscript(tapscript: &script::Script, input: u32) -> Result<Vec<Self>> {
        let mut envelopes = Vec::new();

        let mut instructions = tapscript.instructions().peekable();

        let mut stuttered = false;
        while let Some(instruction) = instructions.next().transpose()? {
            if instruction == PushBytes((&[]).into()) {
                let (stutter, envelope) = Self::from_instructions(
                    &mut instructions,
                    input,
                    envelopes.len() as u32,
                    stuttered,
                )?;
                if let Some(envelope) = envelope {
                    envelopes.push(envelope);
                } else {
                    stuttered = stutter;
                }
            }
        }

        Ok(envelopes)
    }

    fn accept(
        instructions: &mut Peekable<script::Instructions>,
        instruction: script::Instruction,
    ) -> Result<bool> {
        if instructions.peek() == Some(&Ok(instruction)) {
            instructions.next().transpose()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn from_instructions(
        instructions: &mut Peekable<script::Instructions>,
        input: u32,
        offset: u32,
        stutter: bool,
    ) -> Result<(bool, Option<Self>)> {
        use opcodes::all::*;

        if !Self::accept(instructions, Op(OP_IF))? {
            let stutter = instructions.peek() == Some(&Ok(PushBytes((&[]).into())));
            return Ok((stutter, None));
        }

        if !Self::accept(instructions, PushBytes(PROTOCOL_ID.into()))? {
            let stutter = instructions.peek() == Some(&Ok(PushBytes((&[]).into())));
            return Ok((stutter, None));
        }

        let mut pushnum = false;

        let mut payload = Vec::new();

        while let Some(instruction) = instructions.next().transpose()? {
            let opcode = match instruction {
                Op(opcode) => opcode,
                PushBytes(push) => {
                    payload.push(push.as_bytes().to_vec());
                    continue;
                }
            };

            let opcode_payload = vec![opcode.to_u8()];

            match opcode {
                OP_ENDIF => {
                    return Ok((
                        false,
                        Some(Envelope {
                            input,
                            offset,
                            payload,
                            pushnum,
                            stutter,
                        }),
                    ));
                }
                OP_PUSHNUM_NEG1 | OP_PUSHNUM_1 | OP_PUSHNUM_2 | OP_PUSHNUM_3 | OP_PUSHNUM_4
                | OP_PUSHNUM_5 | OP_PUSHNUM_6 | OP_PUSHNUM_7 | OP_PUSHNUM_8 | OP_PUSHNUM_9
                | OP_PUSHNUM_10 | OP_PUSHNUM_11 | OP_PUSHNUM_12 | OP_PUSHNUM_13 | OP_PUSHNUM_14
                | OP_PUSHNUM_15 | OP_PUSHNUM_16 => {
                    payload.push(opcode_payload);
                }

                _ => return Ok((false, None)),
            }

            pushnum = true;
        }

        Ok((false, None))
    }
}
