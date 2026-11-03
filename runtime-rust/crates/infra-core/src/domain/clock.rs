//! Injected knowledge calendar, separate from proof and deadline clocks.
use crate::{Result, RuntimeError};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};
use std::cell::RefCell;
fn invalid_anchor() -> RuntimeError {
    RuntimeError::invalid("MEMORY_EVAL_CLOCK must be an ISO datetime timestamp")
}
fn decimal(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    bytes.iter().try_fold(0u32, |n, b| {
        n.checked_mul(10)?.checked_add((b - b'0') as u32)
    })
}
fn date_end(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 7 {
        return None;
    }
    if bytes.len() == 7 {
        return Some(7);
    }
    if bytes[4] == b'-' {
        if bytes.get(5) == Some(&b'W') {
            if bytes.len() > 8 && bytes[8] == b'-' {
                if bytes.len() == 9 {
                    return None;
                }
                if bytes.len() > 10 && bytes[10].is_ascii_digit() {
                    Some(8)
                } else {
                    Some(10)
                }
            } else {
                Some(8)
            }
        } else {
            Some(10)
        }
    } else if bytes[4] == b'W' {
        let mut at = 7;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1
        }
        Some(if at < 9 {
            at
        } else if at % 2 == 0 {
            7
        } else {
            8
        })
    } else {
        Some(8)
    }
}
fn calendar(text: &str) -> Option<NaiveDate> {
    if text.len() < 7 || !text.as_bytes().get(..4)?.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = decimal(&text.as_bytes()[..4])?;
    if !(1..=9999).contains(&year) {
        return None;
    }
    let date = super::python_date(text)?;
    (1..=9999).contains(&date.year()).then_some(date)
}
fn time_parts(text: &str) -> Option<(u32, u32, u32, u32)> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let mut values = [0u32; 3];
    let mut at = 0usize;
    let mut colon = false;
    let mut fraction = 0u32;
    for index in 0..3 {
        values[index] = decimal(bytes.get(at..at + 2)?)?;
        at += 2;
        if at == bytes.len() {
            return Some((values[0], values[1], values[2], fraction));
        }
        let c = bytes[at];
        if index == 0 {
            colon = c == b':'
        }
        if c == b'.' || c == b',' {
            if index != 2 {
                return None;
            }
            let digits = &bytes[at + 1..];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return None;
            }
            for i in 0..6 {
                fraction = fraction * 10 + digits.get(i).map(|b| (b - b'0') as u32).unwrap_or(0)
            }
            return Some((values[0], values[1], values[2], fraction));
        }
        if colon {
            if index == 2 || c != b':' {
                return None;
            }
            at += 1;
        } else if index == 2 {
            return None;
        }
    }
    None
}
fn iso_parts(text: &str) -> Option<(NaiveDateTime, Option<i64>)> {
    let end = date_end(text.as_bytes())?;
    let date = calendar(text.get(..end)?)?;
    if end == text.len() {
        return Some((date.and_hms_opt(0, 0, 0)?, None));
    }
    let separator = text[end..].chars().next()?;
    let time = &text[end + separator.len_utf8()..];
    if time.is_empty() {
        return None;
    }
    let zone_at = time
        .char_indices()
        .find(|(_, c)| matches!(c, 'Z' | '+' | '-'))
        .map(|(i, _)| i);
    let (main, zone) = zone_at
        .map(|at| (&time[..at], Some(&time[at..])))
        .unwrap_or((time, None));
    let (hour, minute, second, micro) = time_parts(main)?;
    if minute >= 60
        || second >= 60
        || hour > 24
        || (hour == 24 && (minute != 0 || second != 0 || micro != 0))
    {
        return None;
    }
    let date = if hour == 24 { date.succ_opt()? } else { date };
    if !(1..=9999).contains(&date.year()) {
        return None;
    }
    let naive = date.and_hms_micro_opt(if hour == 24 { 0 } else { hour }, minute, second, micro)?;
    let offset = match zone {
        None => None,
        Some("Z") => Some(0),
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (h, m, s, us) = time_parts(zone.get(1..)?)?;
            let whole = h as i64 * 3600 + m as i64 * 60 + s as i64;
            // CPython's C datetime treats zero whole-second offsets as UTC,
            // before considering the separately parsed fractional component.
            let offset = if whole == 0 {
                0
            } else {
                sign * (whole * 1_000_000 + us as i64)
            };
            if offset.abs() >= 86_400_000_000 {
                return None;
            }
            Some(offset)
        }
    };
    Some((naive, offset))
}
fn naive_timestamp(value: NaiveDateTime) -> Option<DateTime<Utc>> {
    match chrono::Local.from_local_datetime(&value) {
        LocalResult::Single(v) => Some(v.with_timezone(&Utc)),
        LocalResult::Ambiguous(a, b) => Some(a.with_timezone(&Utc).min(b.with_timezone(&Utc))),
        LocalResult::None => {
            let mut before = None;
            let mut after = None;
            for hours in 1..=48 {
                if before.is_none() {
                    before = value
                        .checked_sub_signed(chrono::Duration::hours(hours))
                        .and_then(|v| chrono::Local.from_local_datetime(&v).earliest())
                        .map(|v| v.offset().fix().local_minus_utc());
                }
                if after.is_none() {
                    after = value
                        .checked_add_signed(chrono::Duration::hours(hours))
                        .and_then(|v| chrono::Local.from_local_datetime(&v).earliest())
                        .map(|v| v.offset().fix().local_minus_utc());
                }
                if before.is_some() && after.is_some() {
                    break;
                }
            }
            let utc = value.and_utc();
            let a = utc.checked_sub_signed(chrono::Duration::seconds(before? as i64))?;
            let b = utc.checked_sub_signed(chrono::Duration::seconds(after? as i64))?;
            Some(a.max(b))
        }
    }
}
/// datetime.fromisoformat(...).timestamp(): naive anchors use local fold=0.
pub fn parse_python_anchor(text: &str) -> Result<DateTime<Utc>> {
    let (naive, offset) = iso_parts(text).ok_or_else(invalid_anchor)?;
    let value = match offset {
        Some(micros) => naive
            .and_utc()
            .checked_sub_signed(chrono::Duration::microseconds(micros)),
        None => naive_timestamp(naive),
    };
    value.ok_or_else(invalid_anchor)
}
pub fn python_iso_datetime_valid(text: &str) -> bool {
    iso_parts(text).is_some()
}
#[derive(Clone, Debug)]
pub enum WallClock {
    System,
    Offset(chrono::Duration),
    Frozen(DateTime<Utc>),
}
impl WallClock {
    pub fn from_eval_environment() -> Result<Self> {
        match std::env::var("MEMORY_EVAL_CLOCK") {
            Err(std::env::VarError::NotPresent) => Ok(Self::System),
            Err(_) => Err(invalid_anchor()),
            Ok(value) if value.is_empty() => Ok(Self::System),
            Ok(value) => Ok(Self::Offset(parse_python_anchor(&value)? - Utc::now())),
        }
    }
    pub fn now(&self) -> DateTime<Utc> {
        match self {
            Self::System => Utc::now(),
            Self::Offset(offset) => Utc::now() + *offset,
            Self::Frozen(value) => *value,
        }
    }
}
thread_local! {static CURRENT:RefCell<Option<WallClock>>=const{RefCell::new(None)};}
/// True only while this thread has an explicit historical wall clock.
pub fn is_controlled() -> bool {
    CURRENT.with(|current| {
        matches!(
            current.borrow().as_ref(),
            Some(WallClock::Offset(_) | WallClock::Frozen(_))
        )
    })
}
pub struct ClockScope {
    previous: Option<WallClock>,
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
pub fn enter(clock: WallClock) -> ClockScope {
    let previous = CURRENT.with(|current| current.replace(Some(clock)));
    ClockScope {
        previous,
        _same_thread: std::marker::PhantomData,
    }
}
impl Drop for ClockScope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.previous.take());
        });
    }
}
pub fn now() -> DateTime<Utc> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .map(WallClock::now)
            .unwrap_or_else(Utc::now)
    })
}
pub fn today() -> NaiveDate {
    now().date_naive()
}
pub fn local_today() -> NaiveDate {
    now().with_timezone(&chrono::Local).date_naive()
}
pub fn latest_today() -> NaiveDate {
    today().max(local_today())
}
pub fn proof_now() -> DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod iso_anchor_contract_tests {
    use super::{parse_python_anchor, python_iso_datetime_valid};

    fn utc_equals(input: &str, expected: &str) {
        let actual = parse_python_anchor(input).expect("valid CPython 3.14 ISO anchor");
        let expected = chrono::DateTime::parse_from_rfc3339(expected).expect("fixed UTC oracle");
        assert_eq!(
            actual.timestamp_micros(),
            expected.timestamp_micros(),
            "{input}"
        );
    }

    #[test]
    fn midnight_rollover_matches_python314_fromisoformat() {
        // datetime.fromisoformat has a rollover branch distinct from the
        // datetime constructor's hour range in CPython 3.14.
        for (input, expected) in [
            ("2026-01-01T24:00:00Z", "2026-01-02T00:00:00Z"),
            ("2024-02-29T24:00:00Z", "2024-03-01T00:00:00Z"),
            ("2026-12-31T24:00:00Z", "2027-01-01T00:00:00Z"),
            ("2026-01-01T24:00:00+03:30", "2026-01-01T20:30:00Z"),
        ] {
            utc_equals(input, expected);
        }
    }

    #[test]
    fn zero_whole_second_timezone_matches_cpython_utc_special_case() {
        // CPython's C datetime returns its UTC singleton when tzoffset is
        // zero seconds, even when the parsed tz_useconds is nonzero.
        for input in [
            "2026-01-01T00:00:00+00:00:00.5",
            "2026-01-01T00:00:00-00:00:00.5",
        ] {
            utc_equals(input, "2026-01-01T00:00:00Z");
        }
        utc_equals("2026-01-01T00:00:01+00:00:01.5", "2025-12-31T23:59:59.5Z");
        utc_equals("2026-01-01T00:00:01-00:00:01.5", "2026-01-01T00:00:02.5Z");
    }

    #[test]
    fn supported_iso_forms_retain_microseconds_and_calendar_identity() {
        for (input, expected) in [
            ("20260101T235959Z", "2026-01-01T23:59:59Z"),
            ("2026-W01-4T00:00:00Z", "2026-01-01T00:00:00Z"),
            ("2026-01-01x12:00:00Z", "2026-01-01T12:00:00Z"),
            (
                "2026-01-01T12:34:56,123456789+00:00",
                "2026-01-01T12:34:56.123456Z",
            ),
        ] {
            utc_equals(input, expected);
        }
        for input in [
            "2026-01-01T24:00:00.000001Z",
            "2026-01-01T24:00:01Z",
            "2026-01-01T24:01:00Z",
            "2026-01-01T25:00:00Z",
            "9999-12-31T24:00:00Z",
            "2026-01-01T23:60:00Z",
            "2026-01-01T23:59:60Z",
            "2026-02-29T00:00:00Z",
            "2026-01-01T00:00:00+24:00",
            "2026-01-01T00:00:00-24:00",
            "2026-01-01T00:00:00+00:00:00.",
        ] {
            assert!(!python_iso_datetime_valid(input), "{input}");
            assert!(parse_python_anchor(input).is_err(), "{input}");
        }
    }
}
