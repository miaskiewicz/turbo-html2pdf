//! PDF 2.0 password encryption (the `encrypt` feature, gated off by default).
//!
//! Implements the **Standard Security Handler, V=5 / R=6 (AESV3, AES-256)** from
//! ISO 32000-2 §7.6.4.3 — the modern, FIPS-grade scheme also recognised by Adobe
//! Acrobat's "256-bit AES" mode. A caller supplies a user password (to open) and
//! optionally an owner password (full permissions) plus a [`Permissions`] set;
//! every string and stream in the emitted PDF is AES-256-CBC encrypted with a
//! random per-object IV under one random 32-byte *file encryption key*, and an
//! `/Encrypt` dictionary plus a file `/ID` are written so a reader demands the
//! password.
//!
//! **Randomness.** This is the ONE place the engine's "no entropy" rule is
//! waived (and only behind `#[cfg(feature = "encrypt")]`): salts, the file key
//! and the IVs come from the OS CSPRNG via [`getrandom`]. So an encrypted PDF is
//! intentionally NOT byte-deterministic — that is correct for encryption. With
//! `EmitOptions.encryption == None` the emitter never touches this module and
//! stays byte-deterministic.
//!
//! **Shape.** [`encrypt_pdf`] takes the *finished* bytes from `pdf-writer` (a
//! classic PDF with a cross-reference table, every object indirect and
//! top-level) and rewrites them: it parses each `N G obj … endobj`, encrypts the
//! strings/stream inside, re-serialises the body, then writes a fresh xref table
//! and a trailer carrying `/Encrypt` and `/ID`. The grammar it parses is exactly
//! what this crate emits, not arbitrary PDF.

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes256;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use sha2::{Digest, Sha256, Sha384, Sha512};

mod parse;
mod serialize;

/// What an opened reader is allowed to do with the document. Mirrors the
/// permission bits of ISO 32000-2 Table 22. A field that is `false` clears the
/// corresponding bit in `/P`, so a viewer that honours permissions disables that
/// action for a user-password (non-owner) open. `Permissions::all()` (the
/// default) grants everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    /// Print the document (bit 3). When also `high_quality_print`, full-res.
    pub print: bool,
    /// Modify the contents (bit 4).
    pub modify: bool,
    /// Copy or extract text and graphics (bit 5).
    pub copy: bool,
    /// Add or modify annotations and fill form fields (bit 6).
    pub annotate: bool,
    /// Fill in existing interactive form fields (bit 9).
    pub fill_forms: bool,
    /// Extract text/graphics for accessibility (bit 10).
    pub accessibility: bool,
    /// Assemble the document — insert/rotate/delete pages (bit 11).
    pub assemble: bool,
    /// Print at full resolution (bit 12); requires `print`.
    pub high_quality_print: bool,
}

impl Permissions {
    /// Grant every permission (the default for a freshly encrypted document).
    pub fn all() -> Permissions {
        Permissions {
            print: true,
            modify: true,
            copy: true,
            annotate: true,
            fill_forms: true,
            accessibility: true,
            assemble: true,
            high_quality_print: true,
        }
    }

    /// The 32-bit `/P` value: reserved high bits set to 1, each granted action
    /// setting its bit, everything else 0 (ISO 32000-2 Table 22). Returned as the
    /// signed `i32` the `/P` entry stores.
    pub fn to_p(self) -> i32 {
        let mut bits: u32 = 0xFFFF_F0C0; // reserved bits 7,8,13.. + 1,2 = 1.
        self.apply_bit(&mut bits, 3, self.print);
        self.apply_bit(&mut bits, 4, self.modify);
        self.apply_bit(&mut bits, 5, self.copy);
        self.apply_bit(&mut bits, 6, self.annotate);
        self.apply_bit(&mut bits, 9, self.fill_forms);
        self.apply_bit(&mut bits, 10, self.accessibility);
        self.apply_bit(&mut bits, 11, self.assemble);
        self.apply_bit(&mut bits, 12, self.high_quality_print);
        bits as i32
    }

    /// Set or clear bit `n` (1-based, per the spec's numbering) in `bits`.
    fn apply_bit(self, bits: &mut u32, n: u32, on: bool) {
        let mask = 1u32 << (n - 1);
        if on {
            *bits |= mask;
        } else {
            *bits &= !mask;
        }
    }
}

impl Default for Permissions {
    fn default() -> Permissions {
        Permissions::all()
    }
}

/// Password-encryption settings for [`EmitOptions`](super::EmitOptions).
///
/// `user_password` is required to *open* the document. `owner_password`, when
/// set, grants full permissions regardless of `permissions`; when `None` the
/// owner password is taken to equal the user password (so the document still has
/// a valid `/O`/`/OE` pair, as the spec requires).
#[derive(Debug, Clone)]
pub struct Encryption {
    pub user_password: String,
    pub owner_password: Option<String>,
    pub permissions: Permissions,
}

/// AES block / key / salt sizes in bytes.
const BLOCK: usize = 16;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 8;

type Aes256CbcEnc = cbc::Encryptor<Aes256>;

/// Encrypt the finished PDF `input` in place of returning new bytes.
///
/// Every literal/hex string and every stream is replaced by its AES-256-CBC
/// ciphertext (IV-prefixed), and an `/Encrypt` dictionary + file `/ID` are added.
/// The object numbering and overall structure are preserved; only bodies and the
/// xref/trailer change.
pub fn encrypt_pdf(input: &[u8], enc: &Encryption) -> Vec<u8> {
    let doc = parse::Document::parse(input);
    let keys = Keys::derive(enc);
    let id = random_bytes::<16>();
    let mut objects = doc.objects;
    for obj in &mut objects {
        encrypt_object(obj, &keys.file_key);
    }
    serialize::write(&objects, doc.trailer, &keys, enc, &id)
}

/// Encrypt every string and the stream of one parsed object under `file_key`.
///
/// Strings are spliced back-to-front so that an earlier span's byte offsets are
/// not invalidated by a later replacement of different length.
fn encrypt_object(obj: &mut parse::Object, file_key: &[u8; KEY_LEN]) {
    let spans = std::mem::take(&mut obj.strings);
    for span in spans.iter().rev() {
        let cipher = aes_cbc_encrypt(&span.bytes, file_key);
        obj.body = parse::splice(
            &obj.body,
            span.start,
            span.end,
            &serialize::hex_string(&cipher),
        );
    }
    if let Some(stream) = obj.stream.take() {
        obj.encrypted_stream = Some(aes_cbc_encrypt(&stream, file_key));
    }
}

/// AES-256-CBC encrypt `data` with a fresh random 16-byte IV, returning the IV
/// followed by the ciphertext (V5 omits per-object key derivation — the file key
/// is used directly, ISO 32000-2 §7.6.4.3).
fn aes_cbc_encrypt(data: &[u8], key: &[u8; KEY_LEN]) -> Vec<u8> {
    let iv = random_bytes::<BLOCK>();
    let mut buf = pkcs_pad(data);
    let enc = Aes256CbcEnc::new(key.into(), (&iv).into());
    let n = data.len();
    let pad = buf.len() - n;
    let ct = enc
        .encrypt_padded_mut::<NoPadding>(&mut buf, n + pad)
        .expect("CBC NoPadding on a block-aligned buffer never fails");
    let mut out = Vec::with_capacity(BLOCK + ct.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(ct);
    out
}

/// Pad `data` to a whole number of AES blocks with PKCS#7 padding (the padding
/// PDF AESV3 strings/streams use). A block-aligned input still gains a full block
/// of padding, per PKCS#7.
fn pkcs_pad(data: &[u8]) -> Vec<u8> {
    let pad = BLOCK - (data.len() % BLOCK);
    let mut buf = Vec::with_capacity(data.len() + pad);
    buf.extend_from_slice(data);
    buf.resize(data.len() + pad, pad as u8);
    buf
}

/// The derived key material for the `/Encrypt` dictionary plus the file key used
/// to encrypt object bodies.
pub struct Keys {
    /// The random 32-byte file encryption key.
    pub file_key: [u8; KEY_LEN],
    /// `/U` (48 bytes: 32 hash + 8 validation salt + 8 key salt).
    pub u: [u8; 48],
    /// `/UE` (32 bytes: file key wrapped under the user-key hash).
    pub ue: [u8; KEY_LEN],
    /// `/O` (48 bytes).
    pub o: [u8; 48],
    /// `/OE` (32 bytes).
    pub oe: [u8; KEY_LEN],
    /// `/Perms` (16 bytes).
    pub perms: [u8; BLOCK],
}

impl Keys {
    /// Run the V5/R6 key-derivation algorithms (ISO 32000-2 Algorithm 2.B and
    /// the U/UE/O/OE/Perms derivations) for `enc`.
    fn derive(enc: &Encryption) -> Keys {
        let file_key = random_bytes::<KEY_LEN>();
        let user_pw = enc.user_password.as_bytes();
        let owner_pw = enc
            .owner_password
            .as_deref()
            .unwrap_or(&enc.user_password)
            .as_bytes();
        let (u, ue) = derive_user(user_pw, &file_key);
        let (o, oe) = derive_owner(owner_pw, &file_key, &u);
        let perms = derive_perms(enc.permissions, &file_key);
        Keys {
            file_key,
            u,
            ue,
            o,
            oe,
            perms,
        }
    }
}

/// Derive `/U` and `/UE` from the user password (ISO 32000-2 §7.6.4.4.7).
fn derive_user(pw: &[u8], file_key: &[u8; KEY_LEN]) -> ([u8; 48], [u8; KEY_LEN]) {
    let validation_salt = random_bytes::<SALT_LEN>();
    let key_salt = random_bytes::<SALT_LEN>();
    let hash = hash_2b(pw, &validation_salt, &[]);
    let u = pack_u_or_o(&hash, &validation_salt, &key_salt);
    let intermediate = hash_2b(pw, &key_salt, &[]);
    let ue = aes_cbc_nopad_noiv(file_key, &intermediate);
    (u, ue)
}

/// Derive `/O` and `/OE` from the owner password (the owner hashes also fold in
/// the 48-byte `/U`, ISO 32000-2 §7.6.4.4.8).
fn derive_owner(pw: &[u8], file_key: &[u8; KEY_LEN], u: &[u8; 48]) -> ([u8; 48], [u8; KEY_LEN]) {
    let validation_salt = random_bytes::<SALT_LEN>();
    let key_salt = random_bytes::<SALT_LEN>();
    let hash = hash_2b(pw, &validation_salt, u);
    let o = pack_u_or_o(&hash, &validation_salt, &key_salt);
    let intermediate = hash_2b(pw, &key_salt, u);
    let oe = aes_cbc_nopad_noiv(file_key, &intermediate);
    (o, oe)
}

/// Pack a 32-byte hash + two 8-byte salts into the 48-byte `/U` or `/O` value.
fn pack_u_or_o(
    hash: &[u8; KEY_LEN],
    validation_salt: &[u8; SALT_LEN],
    key_salt: &[u8; SALT_LEN],
) -> [u8; 48] {
    let mut out = [0u8; 48];
    out[..KEY_LEN].copy_from_slice(hash);
    out[KEY_LEN..KEY_LEN + SALT_LEN].copy_from_slice(validation_salt);
    out[KEY_LEN + SALT_LEN..].copy_from_slice(key_salt);
    out
}

/// AES-256-CBC, no padding, zero IV, single use — wraps the 32-byte file key
/// under the 32-byte intermediate key for `/UE` / `/OE`.
fn aes_cbc_nopad_noiv(file_key: &[u8; KEY_LEN], intermediate: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    let iv = [0u8; BLOCK];
    let mut buf = *file_key;
    let enc = Aes256CbcEnc::new(intermediate.into(), (&iv).into());
    enc.encrypt_padded_mut::<NoPadding>(&mut buf, KEY_LEN)
        .expect("32-byte buffer is block-aligned");
    buf
}

/// Build the 16-byte `/Perms` value: AES-256-ECB (no padding) of the permission
/// block (ISO 32000-2 §7.6.4.4.9).
fn derive_perms(perms: Permissions, file_key: &[u8; KEY_LEN]) -> [u8; BLOCK] {
    let block = perms_block(perms);
    let cipher = Aes256::new(file_key.into());
    let mut ga = GenericArray::clone_from_slice(&block);
    cipher.encrypt_block(&mut ga);
    let mut out = [0u8; BLOCK];
    out.copy_from_slice(&ga);
    out
}

/// The plaintext 16-byte permission block fed to AES-ECB for `/Perms`: the
/// little-endian `/P`, `0xFFFFFFFF`, the `EncryptMetadata` flag (`T`), the magic
/// `adb`, then four random filler bytes.
fn perms_block(perms: Permissions) -> [u8; BLOCK] {
    let p = perms.to_p() as u32;
    let mut b = [0u8; BLOCK];
    b[..4].copy_from_slice(&p.to_le_bytes());
    b[4..8].copy_from_slice(&[0xFF; 4]);
    b[8] = b'T'; // metadata is encrypted.
    b[9..12].copy_from_slice(b"adb");
    let filler = random_bytes::<4>();
    b[12..].copy_from_slice(&filler);
    b
}

/// The hardened Algorithm 2.B hash (ISO 32000-2 §7.6.4.3.4): SHA-256 of
/// `password ‖ salt ‖ extra`, then ≥64 rounds of an AES-128-CBC mix whose
/// per-round modulus selects SHA-256/384/512.
fn hash_2b(password: &[u8], salt: &[u8], extra: &[u8]) -> [u8; KEY_LEN] {
    let mut k = sha256_concat(password, salt, extra);
    let mut round = 0;
    loop {
        let k1 = build_k1(password, &k, extra);
        let e = aes128_cbc_round(&k, &k1);
        k = next_hash(&e);
        round += 1;
        if round >= 64 && (*e.last().unwrap() as usize) <= round - 32 {
            break;
        }
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&k[..KEY_LEN]);
    out
}

/// Round step: `K1` is `(password ‖ K ‖ extra)` repeated 64 times.
fn build_k1(password: &[u8], k: &[u8], extra: &[u8]) -> Vec<u8> {
    let mut unit = Vec::with_capacity(password.len() + k.len() + extra.len());
    unit.extend_from_slice(password);
    unit.extend_from_slice(k);
    unit.extend_from_slice(extra);
    unit.repeat(64)
}

/// Round step: AES-128-CBC encrypt `k1` using the first 16 bytes of `k` as key
/// and the next 16 as IV (no padding; `k1` is a multiple of the block size).
fn aes128_cbc_round(k: &[u8], k1: &[u8]) -> Vec<u8> {
    type Enc = cbc::Encryptor<aes::Aes128>;
    let key = GenericArray::from_slice(&k[..BLOCK]);
    let iv = GenericArray::from_slice(&k[BLOCK..BLOCK * 2]);
    let mut buf = k1.to_vec();
    let n = buf.len();
    let enc = Enc::new(key, iv);
    enc.encrypt_padded_mut::<NoPadding>(&mut buf, n)
        .expect("k1 is block-aligned");
    buf
}

/// Round step: pick the hash by `E[..16]` as a big number mod 3 and digest `E`.
fn next_hash(e: &[u8]) -> Vec<u8> {
    let m = mod3(&e[..BLOCK]);
    match m {
        0 => Sha256::digest(e).to_vec(),
        1 => Sha384::digest(e).to_vec(),
        _ => Sha512::digest(e).to_vec(),
    }
}

/// The first 16 bytes of `e` interpreted as a big-endian integer, mod 3.
fn mod3(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| (acc * 256 + b as u32) % 3)
}

/// Initial SHA-256 of `a ‖ b ‖ c` (the Algorithm 2.B seed).
fn sha256_concat(a: &[u8], b: &[u8], c: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(a);
    h.update(b);
    h.update(c);
    h.finalize().to_vec()
}

/// `N` random bytes from the OS CSPRNG. The only entropy source in the engine,
/// reachable only through this feature.
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).expect("OS CSPRNG must be available to encrypt");
    buf
}
