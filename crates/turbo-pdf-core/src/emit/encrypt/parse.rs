//! Parse the finished `pdf-writer` output into the objects the encryptor mutates.
//!
//! The grammar is exactly what this crate emits: a header, a run of top-level
//! indirect objects (`N G obj … endobj`, optionally carrying one stream), then a
//! classic `xref` table and a `trailer`. We do not parse arbitrary PDF — only
//! that shape. Each object's body is split into the literal/hex string spans
//! that must be encrypted and (if present) its raw stream bytes.

/// One parsed indirect object, ready to be encrypted and re-serialised.
pub struct Object {
    /// The object number `N` (generation is always 0 here).
    pub number: i32,
    /// The dictionary/value body between `obj` and `stream`/`endobj`, verbatim.
    /// String spans inside it are spliced as they are encrypted.
    pub body: Vec<u8>,
    /// The plaintext stream payload, if this object has a `stream … endstream`.
    pub stream: Option<Vec<u8>>,
    /// The encrypted stream payload, filled in by the encryptor.
    pub encrypted_stream: Option<Vec<u8>>,
    /// The literal/hex string spans inside `body`, in source order.
    pub strings: Vec<StringSpan>,
}

/// A string literal located inside an object body: its byte range in `body` and
/// the decoded plaintext bytes that range represents.
pub struct StringSpan {
    pub start: usize,
    pub end: usize,
    pub bytes: Vec<u8>,
}

/// A whole parsed document: the objects plus the verbatim original trailer dict
/// (so the serializer can copy `/Root`/`/Info`/`/Size`).
pub struct Document {
    pub objects: Vec<Object>,
    pub trailer: Trailer,
}

/// The references the trailer must reproduce.
pub struct Trailer {
    pub size: i32,
    pub root: i32,
    pub info: Option<i32>,
}

impl Document {
    /// Parse `input` (the finished PDF) into objects + trailer.
    pub fn parse(input: &[u8]) -> Document {
        let objects = parse_objects(input);
        let trailer = parse_trailer(input);
        Document { objects, trailer }
    }
}

/// Discover and parse every object using the authoritative `xref` table offsets,
/// so binary stream content that happens to contain `obj`/`endobj` cannot be
/// mistaken for an object boundary. Each object spans from its xref offset to the
/// next object's offset (or the xref table), within which the real `endobj` is
/// the last one.
fn parse_objects(input: &[u8]) -> Vec<Object> {
    let xref_at = startxref_offset(input);
    let mut starts = xref_offsets(input, xref_at);
    starts.sort_unstable();
    bounded_windows(&starts, xref_at)
        .map(|(start, end)| parse_object(input, start, end))
        .collect()
}

/// The byte offset of the xref table, read from the `startxref` pointer at the
/// file's tail (the authoritative location).
fn startxref_offset(input: &[u8]) -> usize {
    let kw = find_last(input, b"startxref").expect("a PDF tail has startxref");
    let after = &input[kw + b"startxref".len()..];
    let digits: Vec<u8> = after
        .iter()
        .skip_while(|b| !b.is_ascii_digit())
        .take_while(|b| b.is_ascii_digit())
        .copied()
        .collect();
    ascii_int(&digits) as usize
}

/// Pair each object start with the following start (the last uses `limit`).
fn bounded_windows(starts: &[usize], limit: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    starts.iter().enumerate().map(move |(i, &start)| {
        let end = starts.get(i + 1).copied().unwrap_or(limit);
        (start, end)
    })
}

/// Read the in-use object byte offsets from the classic xref table at `xref_at`.
fn xref_offsets(input: &[u8], xref_at: usize) -> Vec<usize> {
    let body = &input[xref_at + b"xref".len()..];
    parse_xref_entries(body)
}

/// Parse the `OOOOOOOOOO GGGGG n` rows of the xref table into byte offsets,
/// skipping the subsection header line and free (`f`) entries.
fn parse_xref_entries(body: &[u8]) -> Vec<usize> {
    body.split(|&b| b == b'\n')
        .filter_map(xref_entry_offset)
        .collect()
}

/// The byte offset of one xref row, if it is an in-use (`n`) entry. Tolerant of
/// the trailing `\r` in the spec's 20-byte `\r\n`-terminated rows.
fn xref_entry_offset(line: &[u8]) -> Option<usize> {
    let parts: Vec<&[u8]> = line
        .split(|&b| b == b' ' || b == b'\r')
        .filter(|s| !s.is_empty())
        .collect();
    match parts.as_slice() {
        [off, _gen, kind] if *kind == b"n" => Some(ascii_int(off) as usize),
        _ => None,
    }
}

/// Parse the object whose `N 0 obj … endobj` lives in `input[start..end)`.
fn parse_object(input: &[u8], start: usize, end: usize) -> Object {
    let span = &input[start..end];
    let number = ascii_int(span.split(|&b| b == b' ').next().unwrap_or(b""));
    let obj_kw = find_from(span, b"obj", 0).expect("object starts with `obj`");
    let body_end = find_last(span, b"endobj").unwrap_or(span.len());
    let (body, stream) = split_stream(&span[obj_kw + 3..body_end]);
    let strings = find_strings(&body);
    Object {
        number,
        body,
        stream,
        encrypted_stream: None,
        strings,
    }
}

/// Split an object's post-`obj` bytes into the dictionary body and the stream
/// payload (if any). The `/Length` line stays in the body; the serializer
/// rewrites it to the encrypted length.
fn split_stream(rest: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    match find_from(rest, b"stream", 0) {
        Some(kw) => split_at_stream(rest, kw),
        None => (trim_endobj(rest), None),
    }
}

/// Split where a real `stream` keyword starts at `kw`: body is everything up to
/// `kw`, payload is the bytes between the keyword's trailing EOL and `endstream`.
fn split_at_stream(rest: &[u8], kw: usize) -> (Vec<u8>, Option<Vec<u8>>) {
    let body = rest[..kw].to_vec();
    let data_start = after_eol(rest, kw + b"stream".len());
    let data_end = find_from(rest, b"endstream", data_start).expect("stream has endstream");
    let payload = strip_trailing_eol(&rest[data_start..data_end]).to_vec();
    (body, Some(payload))
}

/// Advance past a single EOL (`\r\n`, `\n`) following the `stream` keyword.
fn after_eol(rest: &[u8], pos: usize) -> usize {
    if rest.get(pos) == Some(&b'\r') && rest.get(pos + 1) == Some(&b'\n') {
        pos + 2
    } else if rest.get(pos) == Some(&b'\n') {
        pos + 1
    } else {
        pos
    }
}

/// Drop a single trailing EOL the writer inserts before `endstream`.
fn strip_trailing_eol(data: &[u8]) -> &[u8] {
    if data.ends_with(b"\r\n") {
        &data[..data.len() - 2]
    } else if data.ends_with(b"\n") {
        &data[..data.len() - 1]
    } else {
        data
    }
}

/// Trim the trailing `\nendobj` region from a stream-less body.
fn trim_endobj(rest: &[u8]) -> Vec<u8> {
    let end = find_from(rest, b"endobj", 0).unwrap_or(rest.len());
    rest[..end].to_vec()
}

/// Locate every literal `(...)` and hex `<...>` string in `body`, in order.
fn find_strings(body: &[u8]) -> Vec<StringSpan> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < body.len() {
        i = scan_token(body, i, &mut spans);
    }
    spans
}

/// Inspect the byte at `i`: enter a string scan, skip a dict delimiter, or step.
/// Returns the next index to inspect.
fn scan_token(body: &[u8], i: usize, spans: &mut Vec<StringSpan>) -> usize {
    match body[i] {
        b'(' => scan_literal(body, i, spans),
        b'<' if body.get(i + 1) == Some(&b'<') => i + 2,
        b'<' => scan_hex(body, i, spans),
        _ => i + 1,
    }
}

/// Scan a literal `(...)` from `start`, decode it, push the span, return the
/// index just past the closing `)`.
fn scan_literal(body: &[u8], start: usize, spans: &mut Vec<StringSpan>) -> usize {
    let (end, bytes) = decode_literal(body, start);
    spans.push(StringSpan { start, end, bytes });
    end
}

/// Decode a literal string starting at `start` (`body[start] == b'('`),
/// returning `(end_after_close_paren, decoded_bytes)`.
///
/// `depth` is the nesting level of *unescaped* parens; the opening paren puts it
/// at 1 and is not part of the content. Inner balanced parens are kept verbatim;
/// the `)` that brings `depth` back to 0 ends the string.
fn decode_literal(body: &[u8], start: usize) -> (usize, Vec<u8>) {
    let mut out = Vec::new();
    let mut depth = 1i32;
    let mut i = start + 1;
    while i < body.len() {
        let b = body[i];
        if b == b'\\' {
            i = decode_escape(body, i, &mut out);
            continue;
        }
        i += 1;
        depth = step_paren(b, depth, &mut out);
        if depth == 0 {
            return (i, out);
        }
    }
    (i, out)
}

/// Fold one literal-string byte into `out`, returning the new paren depth. A
/// closing paren that drops `depth` to 0 is the terminator and is not pushed.
fn step_paren(b: u8, depth: i32, out: &mut Vec<u8>) -> i32 {
    match b {
        b'(' => {
            out.push(b);
            depth + 1
        }
        b')' if depth == 1 => 0,
        b')' => {
            out.push(b);
            depth - 1
        }
        _ => {
            out.push(b);
            depth
        }
    }
}

/// Decode one backslash escape at `i` (`body[i] == b'\\'`), pushing the resulting
/// byte(s) and returning the next index.
fn decode_escape(body: &[u8], i: usize, out: &mut Vec<u8>) -> usize {
    let next = body.get(i + 1).copied().unwrap_or(b'\\');
    match next {
        b'n' => push_and(out, b'\n', i + 2),
        b'r' => push_and(out, b'\r', i + 2),
        b't' => push_and(out, b'\t', i + 2),
        b'b' => push_and(out, 0x08, i + 2),
        b'f' => push_and(out, 0x0C, i + 2),
        b'0'..=b'7' => decode_octal(body, i + 1, out),
        other => push_and(out, other, i + 2),
    }
}

/// Push `byte` and return `next` (helper to keep `decode_escape` flat).
fn push_and(out: &mut Vec<u8>, byte: u8, next: usize) -> usize {
    out.push(byte);
    next
}

/// Decode a 1–3 digit octal escape beginning at `start`, push the byte, return
/// the index after the last octal digit.
fn decode_octal(body: &[u8], start: usize, out: &mut Vec<u8>) -> usize {
    let mut value: u32 = 0;
    let mut i = start;
    while i < body.len() && i < start + 3 && body[i].is_ascii_digit() && body[i] < b'8' {
        value = value * 8 + (body[i] - b'0') as u32;
        i += 1;
    }
    out.push(value as u8);
    i
}

/// Scan a hex `<...>` from `start`, decode it, push the span, return the index
/// just past the closing `>`.
fn scan_hex(body: &[u8], start: usize, spans: &mut Vec<StringSpan>) -> usize {
    let end = find_from(body, b">", start)
        .map(|e| e + 1)
        .unwrap_or(body.len());
    let bytes = decode_hex(&body[start + 1..end - 1]);
    spans.push(StringSpan { start, end, bytes });
    end
}

/// Decode the inner bytes of a hex string (whitespace ignored, odd trailing
/// nibble padded with 0, per the PDF spec).
fn decode_hex(inner: &[u8]) -> Vec<u8> {
    let digits: Vec<u8> = inner.iter().filter_map(|&b| hex_val(b)).collect();
    digits
        .chunks(2)
        .map(|c| (c[0] << 4) | c.get(1).copied().unwrap_or(0))
        .collect()
}

/// One hex digit's value, or `None` for non-hex bytes (whitespace etc.).
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Replace `body[start..end]` with `replacement`, returning the new buffer.
pub fn splice(body: &[u8], start: usize, end: usize, replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + replacement.len());
    out.extend_from_slice(&body[..start]);
    out.extend_from_slice(replacement);
    out.extend_from_slice(&body[end..]);
    out
}

/// Parse the trailer dictionary's `/Size`, `/Root` and optional `/Info` refs.
fn parse_trailer(input: &[u8]) -> Trailer {
    let kw = find_last(input, b"trailer").expect("a PDF has a trailer");
    let dict = &input[kw..];
    Trailer {
        size: ref_after(dict, b"/Size").expect("trailer has /Size"),
        root: ref_after(dict, b"/Root").expect("trailer has /Root"),
        info: ref_after(dict, b"/Info"),
    }
}

/// The integer (object number for refs, or value for `/Size`) following `key`.
fn ref_after(dict: &[u8], key: &[u8]) -> Option<i32> {
    let at = find_from(dict, key, 0)?;
    let after = &dict[at + key.len()..];
    let digits: Vec<u8> = after
        .iter()
        .skip_while(|&&b| b == b' ')
        .take_while(|&&b| b.is_ascii_digit())
        .copied()
        .collect();
    Some(ascii_int(&digits))
}

/// Parse an ASCII integer (no sign), defaulting to 0 on empty input.
fn ascii_int(bytes: &[u8]) -> i32 {
    bytes
        .iter()
        .filter(|b| b.is_ascii_digit())
        .fold(0i32, |acc, &b| acc * 10 + (b - b'0') as i32)
}

/// First index of `needle` in `hay` at or after `from`.
fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Last index of `needle` in `hay`.
fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}
