use super::*;

use byteorder::{LittleEndian, ReadBytesExt};

use proto::ToRaw;

/// Variable length integer
/// Also known as CompactSize
#[derive(Debug, Clone)]
pub struct VarUint {
    pub value: u64, // Represents bytes as uint value
    buf: Vec<u8>,   // Raw bytes used for serialization (uint8 .. uint64 possible). (little endian)
}

impl VarUint {
    #[inline]
    fn new(value: u64, buf: Vec<u8>) -> VarUint {
        VarUint { value, buf }
    }

    pub fn read_from<R: Read + ?Sized>(reader: &mut R) -> io::Result<VarUint> {
        use crate::timing::BLOCK_READ_METRICS;
        let start = std::time::Instant::now();
        let first = reader.read_u8()?; // read first length byte
        let vint = match first {
            0x00..=0xfc => VarUint::from(first),
            0xfd => VarUint::from(reader.read_u16::<LittleEndian>()?),
            0xfe => VarUint::from(reader.read_u32::<LittleEndian>()?),
            0xff => VarUint::from(reader.read_u64::<LittleEndian>()?),
        };
        BLOCK_READ_METRICS.record_varint(start.elapsed());
        Ok(vint)
    }
}

impl From<u8> for VarUint {
    #[inline]
    fn from(value: u8) -> Self {
        VarUint::new(value as u64, vec![value])
    }
}

impl From<u16> for VarUint {
    fn from(value: u16) -> Self {
        let mut buf: Vec<u8> = Vec::with_capacity(3);
        buf.push(0xfd);
        buf.extend(&value.to_le_bytes());
        VarUint::new(value as u64, buf)
    }
}

impl From<u32> for VarUint {
    fn from(value: u32) -> Self {
        let mut buf: Vec<u8> = Vec::with_capacity(5);
        buf.push(0xfe);
        buf.extend(&value.to_le_bytes());
        VarUint::new(value as u64, buf)
    }
}

impl From<u64> for VarUint {
    fn from(value: u64) -> Self {
        let mut buf: Vec<u8> = Vec::with_capacity(9);
        buf.push(0xff);
        buf.extend(&value.to_le_bytes());
        VarUint::new(value, buf)
    }
}

impl ToRaw for VarUint {
    #[inline]
    fn to_bytes(&self) -> Vec<u8> {
        self.buf.clone()
    }
}

impl fmt::Display for VarUint {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}
