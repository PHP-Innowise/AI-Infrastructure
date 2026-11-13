pub fn compress_runs(values: &[i32]) -> Vec<(i32, usize)> {
    let mut result = Vec::new();
    let mut run = None;
    for &value in values {
        match run {
            Some((last, count)) if last == value => run = Some((last, count + 1)),
            Some(previous) => { result.push(previous); run = Some((value, 1)); }
            None => run = Some((value, 1)),
        }
    }
    result
}

pub fn expand_runs(_: &[(i32, usize)], _: usize) -> Result<Vec<i32>, String> {
    Err("not implemented".into())
}
