pub fn parse_duration(value: &str) -> Result<u64, String> {
    let (number, multiplier) = match value.as_bytes().last() {
        Some(b's') => (&value[..value.len()-1], 1u64),
        Some(b'm') => (&value[..value.len()-1], 60),
        Some(b'h') => (&value[..value.len()-1], 3600),
        _ => return Err("unit".into()),
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("number".into());
    }
    number.parse::<u64>().map_err(|_| "overflow".to_string())?
        .checked_mul(multiplier).ok_or_else(|| "overflow".into())
}
