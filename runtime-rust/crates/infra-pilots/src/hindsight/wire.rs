use serde::Serialize;
pub fn value<T: Serialize + ?Sized>(v: &T) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(v)?)
}
/// Explicit insertion order retains existing frozen Python request/body digests.
pub fn object(fields: Vec<(&str, Vec<u8>)>) -> anyhow::Result<Vec<u8>> {
    let mut out = vec![b'{'];
    for (i, (k, v)) in fields.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend(value(k)?);
        out.push(b':');
        out.extend(v);
    }
    out.push(b'}');
    Ok(out)
}
pub fn array(items: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = vec![b'['];
    for (i, v) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend(v);
    }
    out.push(b']');
    out
}
