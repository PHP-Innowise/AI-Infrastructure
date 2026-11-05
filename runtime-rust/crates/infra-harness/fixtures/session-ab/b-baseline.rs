pub fn parse_duration(value: &str) -> Result<u64, String> {
    value.strip_suffix('s').ok_or("unit")?.parse().map_err(|_| "number".into())
}
