//! Re-serialise the encrypted objects into a complete PDF.
//!
//! Writes the header, every (mutated) indirect object — rewriting a stream's
//! `/Length` to the ciphertext length — then a freshly added `/Encrypt`
//! dictionary object, a classic `xref` table over all of them, and a trailer
//! carrying `/Root`, `/Info`, `/Size`, `/Encrypt` and `/ID`. The `/Encrypt` dict
//! and `/ID` are written in clear (they are the key material, never encrypted).

use super::parse::{Object, Trailer};
use super::{Encryption, Keys};

/// Serialise the document. `id` is the random 16-byte file identifier; both
/// halves of `/ID` use it (a single-revision file).
pub fn write(
    objects: &[Object],
    trailer: Trailer,
    keys: &Keys,
    enc: &Encryption,
    id: &[u8; 16],
) -> Vec<u8> {
    let encrypt_num = trailer.size; // the next free object number.
    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n%\x80\x80\x80\x80\n\n");
    let mut offsets = vec![(0i32, 0usize); 0];
    for obj in objects {
        offsets.push((obj.number, out.len()));
        write_object(&mut out, obj);
    }
    offsets.push((encrypt_num, out.len()));
    write_encrypt_object(&mut out, encrypt_num, keys, enc);
    let xref_offset = out.len();
    let size = encrypt_num + 1;
    write_xref(&mut out, &offsets, size);
    write_trailer(&mut out, &trailer, encrypt_num, id, xref_offset, size);
    out
}

/// Write one `N 0 obj … endobj` block, substituting the encrypted stream and its
/// new `/Length` when the object carries one.
fn write_object(out: &mut Vec<u8>, obj: &Object) {
    push_int(out, obj.number);
    out.extend_from_slice(b" 0 obj\n");
    match &obj.encrypted_stream {
        Some(stream) => write_stream_object(out, &obj.body, stream),
        None => out.extend_from_slice(&obj.body),
    }
    out.extend_from_slice(b"\nendobj\n");
}

/// Write a stream object body: the dict with its `/Length` patched to the
/// ciphertext length, then `stream … endstream`.
fn write_stream_object(out: &mut Vec<u8>, body: &[u8], stream: &[u8]) {
    let patched = patch_length(body, stream.len());
    out.extend_from_slice(&patched);
    out.extend_from_slice(b"stream\n");
    out.extend_from_slice(stream);
    out.extend_from_slice(b"\nendstream");
}

/// Replace the integer after `/Length` in `body` with `len`.
fn patch_length(body: &[u8], len: usize) -> Vec<u8> {
    let key = b"/Length";
    let at = find(body, key).expect("a stream dict has /Length");
    let num_start = skip_spaces(body, at + key.len());
    let num_end = skip_digits(body, num_start);
    let mut out = Vec::with_capacity(body.len());
    out.extend_from_slice(&body[..num_start]);
    push_int(&mut out, len as i32);
    out.extend_from_slice(&body[num_end..]);
    out
}

/// Write the `/Encrypt` dictionary as object `num`.
fn write_encrypt_object(out: &mut Vec<u8>, num: i32, keys: &Keys, enc: &Encryption) {
    push_int(out, num);
    out.extend_from_slice(b" 0 obj\n");
    out.extend_from_slice(b"<< /Filter /Standard /V 5 /R 6 /Length 256");
    out.extend_from_slice(b" /CF << /StdCF << /CFM /AESV3 /AuthEvent /DocOpen /Length 32 >> >>");
    out.extend_from_slice(b" /StmF /StdCF /StrF /StdCF /EncryptMetadata true");
    push_named_hex(out, b" /U ", &keys.u);
    push_named_hex(out, b" /UE ", &keys.ue);
    push_named_hex(out, b" /O ", &keys.o);
    push_named_hex(out, b" /OE ", &keys.oe);
    push_named_hex(out, b" /Perms ", &keys.perms);
    out.extend_from_slice(b" /P ");
    push_int(out, enc.permissions.to_p());
    out.extend_from_slice(b" >>\nendobj\n");
}

/// Write ` /Key <hex>` for a key-material entry.
fn push_named_hex(out: &mut Vec<u8>, key: &[u8], bytes: &[u8]) {
    out.extend_from_slice(key);
    out.extend_from_slice(&hex_string(bytes));
}

/// Write the classic cross-reference table for `size` objects (object 0 is the
/// free head). `offsets` lists `(number, byte_offset)` for the in-use objects.
fn write_xref(out: &mut Vec<u8>, offsets: &[(i32, usize)], size: i32) {
    out.extend_from_slice(b"xref\n0 ");
    push_int(out, size);
    out.push(b'\n');
    out.extend_from_slice(b"0000000000 65535 f \n");
    let mut sorted: Vec<&(i32, usize)> = offsets.iter().collect();
    sorted.sort_by_key(|(n, _)| *n);
    for (_, off) in sorted {
        push_xref_entry(out, *off);
    }
}

/// One 20-byte in-use xref entry: `OOOOOOOOOO 00000 n \n`.
fn push_xref_entry(out: &mut Vec<u8>, offset: usize) {
    let s = format!("{offset:010} 00000 n \n");
    out.extend_from_slice(s.as_bytes());
}

/// Write the trailer dict + `startxref` + `%%EOF`.
fn write_trailer(
    out: &mut Vec<u8>,
    trailer: &Trailer,
    encrypt_num: i32,
    id: &[u8; 16],
    xref_offset: usize,
    size: i32,
) {
    out.extend_from_slice(b"trailer\n<< /Size ");
    push_int(out, size);
    push_ref(out, b" /Root ", trailer.root);
    if let Some(info) = trailer.info {
        push_ref(out, b" /Info ", info);
    }
    push_ref(out, b" /Encrypt ", encrypt_num);
    write_id(out, id);
    out.extend_from_slice(b" >>\nstartxref\n");
    push_int(out, xref_offset as i32);
    out.extend_from_slice(b"\n%%EOF\n");
}

/// Write ` /ID [<hex> <hex>]` (both halves identical for a one-revision file).
fn write_id(out: &mut Vec<u8>, id: &[u8; 16]) {
    out.extend_from_slice(b" /ID [");
    out.extend_from_slice(&hex_string(id));
    out.push(b' ');
    out.extend_from_slice(&hex_string(id));
    out.push(b']');
}

/// Write ` /Key N 0 R`.
fn push_ref(out: &mut Vec<u8>, key: &[u8], num: i32) {
    out.extend_from_slice(key);
    push_int(out, num);
    out.extend_from_slice(b" 0 R");
}

/// Encode `bytes` as a PDF hex string `<...>` (uppercase, no whitespace).
pub fn hex_string(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2 + 2);
    out.push(b'<');
    for &b in bytes {
        out.push(hex_digit(b >> 4));
        out.push(hex_digit(b & 0x0F));
    }
    out.push(b'>');
    out
}

/// One uppercase hex digit for a nibble `0..=15`.
fn hex_digit(n: u8) -> u8 {
    if n < 10 {
        b'0' + n
    } else {
        b'A' + (n - 10)
    }
}

/// Append the decimal ASCII of `n`.
fn push_int(out: &mut Vec<u8>, n: i32) {
    out.extend_from_slice(n.to_string().as_bytes());
}

/// First index of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Index of the first non-space at or after `from`.
fn skip_spaces(body: &[u8], from: usize) -> usize {
    let mut i = from;
    while body.get(i) == Some(&b' ') {
        i += 1;
    }
    i
}

/// Index just past a run of ASCII digits starting at `from`.
fn skip_digits(body: &[u8], from: usize) -> usize {
    let mut i = from;
    while body.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    i
}
