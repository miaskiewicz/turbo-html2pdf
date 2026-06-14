//! Document metadata (§7, AC-7.6). Writes the info dictionary (title/author/
//! subject/keywords/creation date + a fixed producer) deterministically. The
//! creation date is a `D:…Z` UTC string derived from the caller's Unix timestamp
//! or the fixed sentinel, so identical inputs round-trip to identical bytes.

use chrono::{DateTime, Datelike, Timelike, Utc};
use pdf_writer::{Date, Pdf, Ref, TextStr};

use super::{EmitOptions, SENTINEL_DATE};

/// The fixed `/Producer` string. Constant (no version interpolation) so output
/// stays byte-stable across builds.
const PRODUCER: &str = "turbo-pdf";

/// Write the document info dictionary at `id`.
pub fn write_info(pdf: &mut Pdf, id: Ref, opts: &EmitOptions) {
    let mut info = pdf.document_info(id);
    info.producer(TextStr(PRODUCER));
    if let Some(t) = &opts.title {
        info.title(TextStr(t));
    }
    if let Some(a) = &opts.author {
        info.author(TextStr(a));
    }
    if let Some(s) = &opts.subject {
        info.subject(TextStr(s));
    }
    if let Some(k) = &opts.keywords {
        info.keywords(TextStr(k));
    }
    info.creation_date(pdf_date(opts.creation_date.unwrap_or(SENTINEL_DATE)));
}

/// Build a UTC PDF [`Date`] from a Unix timestamp, clamping out-of-range stamps
/// to the sentinel so the conversion is total and deterministic.
fn pdf_date(timestamp: i64) -> Date {
    let dt = DateTime::<Utc>::from_timestamp(timestamp, 0).unwrap_or_else(sentinel_datetime);
    date_from(dt)
}

/// The sentinel as a concrete `DateTime` (infallible: the constant is in range).
fn sentinel_datetime() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(SENTINEL_DATE, 0).expect("sentinel timestamp is in range")
}

/// Convert a chrono `DateTime` into a fully-specified UTC PDF date.
fn date_from(dt: DateTime<Utc>) -> Date {
    Date::new(dt.year() as u16)
        .month(dt.month() as u8)
        .day(dt.day() as u8)
        .hour(dt.hour() as u8)
        .minute(dt.minute() as u8)
        .second(dt.second() as u8)
        .utc_offset_hour(0)
        .utc_offset_minute(0)
}
