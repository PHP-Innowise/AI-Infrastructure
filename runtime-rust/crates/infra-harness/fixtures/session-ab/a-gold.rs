pub fn compress_runs(values: &[i32]) -> Vec<(i32, usize)> {
    let mut result: Vec<(i32, usize)> = Vec::new();
    for &value in values {
        match result.last_mut() {
            Some((last, count)) if *last == value => *count += 1,
            _ => result.push((value, 1)),
        }
    }
    result
}

pub fn expand_runs(runs: &[(i32, usize)], limit: usize) -> Result<Vec<i32>, String> {
    let mut size = 0usize;
    for &(_, count) in runs {
        if count == 0 { return Err("zero count".into()); }
        size = size.checked_add(count).ok_or("overflow")?;
        if size > limit { return Err("limit".into()); }
    }
    let mut result = Vec::with_capacity(size);
    for &(value, count) in runs { result.extend(std::iter::repeat_n(value, count)); }
    Ok(result)
}
