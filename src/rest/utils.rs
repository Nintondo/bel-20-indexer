use validator::ValidationError;

pub fn page_size_default() -> usize {
    6
}

pub fn first_page() -> usize {
    1
}

pub fn validate_tick(tick: &str) -> Result<(), ValidationError> {
    if tick.len() != 4 {
        return Err(ValidationError::new("Wrong tick length"));
    }

    Ok(())
}
