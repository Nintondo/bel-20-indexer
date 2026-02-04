use std::fmt;

pub struct RedactedStr<'a>(pub &'a str);

impl fmt::Debug for RedactedStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = if self.0.is_empty() { "" } else { "****" };
        fmt::Debug::fmt(value, f)
    }
}
