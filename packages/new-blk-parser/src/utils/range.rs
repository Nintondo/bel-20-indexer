use super::*;

#[derive(Copy, Clone)]
pub struct BlockHeightRange {
    pub start: u64,
    pub end: Option<u64>,
}

impl BlockHeightRange {
    pub fn new(start: u64, end: Option<u64>) -> Result<Self> {
        if end.is_some() && start >= end.unwrap() {
            anyhow::bail!("--start value must be lower than --end value",);
        }
        Ok(Self { start, end })
    }
}

impl fmt::Display for BlockHeightRange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let end = match self.end {
            Some(e) => e.to_string(),
            None => String::from("HEAD"),
        };
        write!(f, "{}..{}", self.start, end)
    }
}
