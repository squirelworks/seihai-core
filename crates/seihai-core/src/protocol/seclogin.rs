//! The Seihai secure login: version token `SEI00001`.
//!
//! A Seihai-only login in which the account and password never cross the
//! wire in the clear. It is Seihai's own design, not something any Graal
//! build sends; real clients and servers never see it. The full spec, with
//! test vectors' shape and the reasons for each choice, is
//! `documentation/secure-login.md`.
//!
//! The token goes where a version token goes (the gserver login line, or
//! the listserver's id 7 handshake). After it come the *inner* token, the
//! real build the session then speaks as (`GNW28015` for 2.31, say), and
//! the client's one-time X25519 key. The server answers with its own
//! one-time X25519 key and its long-term Ed25519 key, and signs the whole
//! exchange. The client checks the signature and that the long-term key is
//! the one it pinned for that server, then sends the rest of the classic
//! login (account, password and any build-specific tail) sealed with
//! ChaCha20-Poly1305 under a key from HKDF-SHA256 over the X25519 secret
//! and the transcript.
//!
//! ```text
//! C1  prefix "SEI00001" inner[8] b64(client_x25519)[43] suffix
//! S1  "SEI00001" b64(server_x25519)[43] b64(server_ed25519)[43] b64(sig)[86]
//! C2  "SEI00001" b64(chacha20poly1305(k, nonce 0, aad th, pad(tail)))
//! ```
//!
//! Every line is the body of a plain zlib frame and ends with `\n`; base64
//! has no line feed and no byte below 0x20, so each line is one ordinary
//! message. After C2 both sides run the inner build's session exactly as
//! after its classic login frame: the gen-3 or gen-5 cipher starts from
//! its seed with the key byte from C1.
//!
//! This module holds only the message formats and the cryptography, so the
//! client and the servers share one definition. The sockets are the
//! caller's.

use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::ChaCha20Poly1305;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::fmt;
use x25519_dalek::{EphemeralSecret, PublicKey};

/// The version token of the secure login, revision 1.
pub const TOKEN: &str = "SEI00001";
const TOKEN_B: &[u8] = TOKEN.as_bytes();

/// Base64 length of a 32-byte key, without padding.
const KEY_B64: usize = 43;
/// Base64 length of a 64-byte signature, without padding.
const SIG_B64: usize = 86;
/// The sealed plaintext is padded to a multiple of this, so the length of
/// the ciphertext says little about the length of the password.
pub const PAD_TO: usize = 64;
/// The longest tail [`Sealer::seal`] takes: a 2-byte length plus the tail
/// must fit the padding of a few blocks. A classic tail is two GSTRINGs of
/// at most 95 bytes each and, for 5.x and 6.x, the platform fields.
pub const MAX_TAIL: usize = 1024;

const SIG_CONTEXT: &[u8] = b"seihai-login-v1/server-hello\0";
const KEY_INFO: &[u8] = b"seihai-login-v1/c2s key";
const CHANNEL_C2S: &[u8] = b"seihai-channel-v1/c2s key";
const CHANNEL_S2C: &[u8] = b"seihai-channel-v1/s2c key";

/// Why a handshake step failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The line is not the expected secure-login message.
    Malformed(&'static str),
    /// The server's signature does not verify.
    BadSignature,
    /// The X25519 exchange gave the all-zero secret (a low-order key).
    WeakKey,
    /// The sealed credentials did not open (wrong key, or changed in transit).
    BadSeal,
    /// The tail to seal is longer than [`MAX_TAIL`].
    TooLong,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Malformed(what) => write!(f, "malformed secure-login message: {what}"),
            Error::BadSignature => f.write_str("the server's signature on the secure login does not verify"),
            Error::WeakKey => f.write_str("the key exchange gave a weak shared secret"),
            Error::BadSeal => f.write_str("the sealed login did not open"),
            Error::TooLong => write!(f, "the login is over {MAX_TAIL} bytes"),
        }
    }
}

impl std::error::Error for Error {}

/// `true` when `token` (the eight bytes where a version token goes) is the
/// secure-login token.
pub fn is_token(token: &[u8]) -> bool {
    token == TOKEN_B
}

/// The text form of a server key: `SHA256:` and the unpadded base64 of the
/// key's SHA-256, as an admin prints it and a player compares it.
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    format!("SHA256:{}", B64.encode(Sha256::digest(public_key)))
}

/// A server key as unpadded base64 (what `--server-key` and the pin file hold).
pub fn encode_key(public_key: &[u8; 32]) -> String {
    B64.encode(public_key)
}

/// A server key from [`encode_key`]'s form.
pub fn decode_key(text: &str) -> Option<[u8; 32]> {
    let bytes = B64.decode(text.trim().trim_end_matches('=')).ok()?;
    bytes.try_into().ok()
}

fn key32(b64: &[u8]) -> Result<[u8; 32], Error> {
    let bytes = B64.decode(b64).map_err(|_| Error::Malformed("bad base64 key"))?;
    bytes.try_into().map_err(|_| Error::Malformed("key is not 32 bytes"))
}

/// The bytes the server signs.
fn signed_bytes(c1: &[u8], server_eph: &[u8; 32], server_id: &[u8; 32]) -> Vec<u8> {
    let mut m = Vec::with_capacity(SIG_CONTEXT.len() + 2 + c1.len() + 64);
    m.extend_from_slice(SIG_CONTEXT);
    m.extend_from_slice(&(c1.len() as u16).to_le_bytes());
    m.extend_from_slice(c1);
    m.extend_from_slice(server_eph);
    m.extend_from_slice(server_id);
    m
}

/// The keys of one handshake.
struct Keys {
    th: [u8; 32],
    /// Seals C2 (the login).
    key: [u8; 32],
    /// Channel keys ([`Channel`]): client to server, server to client.
    c2s: [u8; 32],
    s2c: [u8; 32],
}

/// The transcript hash and the keys.
fn derive(shared: &x25519_dalek::SharedSecret, signed: &[u8], sig: &[u8; 64]) -> Result<Keys, Error> {
    if !shared.was_contributory() {
        return Err(Error::WeakKey);
    }
    let mut h = Sha256::new();
    h.update(signed);
    h.update(sig);
    let th: [u8; 32] = h.finalize().into();
    let hk = Hkdf::<Sha256>::new(Some(&th), shared.as_bytes());
    let expand = |info: &[u8]| {
        let mut k = [0u8; 32];
        hk.expand(info, &mut k).expect("32 bytes is a valid HKDF-SHA256 length");
        k
    };
    Ok(Keys { th, key: expand(KEY_INFO), c2s: expand(CHANNEL_C2S), s2c: expand(CHANNEL_S2C) })
}

/// The long-term key a server signs its hello with.
pub struct ServerIdentity {
    key: SigningKey,
}

impl ServerIdentity {
    /// A new random key.
    pub fn generate() -> Self {
        Self { key: SigningKey::generate(&mut OsRng) }
    }

    /// The key from its 32-byte seed (what a server keeps on disk).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self { key: SigningKey::from_bytes(seed) }
    }

    /// The 32-byte seed, to store.
    pub fn seed(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    /// The public key players pin.
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// The key kept in the file at `path` (one line: the seed as unpadded
    /// base64). A missing file is created with a new key, readable by its
    /// owner only on Unix; the `bool` is true then. Servers print
    /// [`fingerprint`] of the public key at start so the admin can publish it.
    pub fn load_or_create(path: &std::path::Path) -> std::io::Result<(Self, bool)> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let seed = decode_key(&text).ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: not a secure-login key", path.display()))
                })?;
                Ok((Self::from_seed(&seed), false))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                use std::io::Write;
                let id = Self::generate();
                if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir)?;
                }
                let mut opts = std::fs::OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(0o600);
                }
                let mut f = opts.open(path)?;
                writeln!(f, "{}", B64.encode(id.seed()))?;
                Ok((id, true))
            }
            Err(e) => Err(e),
        }
    }
}

/// The client's side, from C1 to C2.
pub struct ClientHandshake {
    secret: EphemeralSecret,
    hello: Vec<u8>,
}

impl ClientHandshake {
    /// C1: `prefix`, the token, `inner` (the eight-byte token of the build
    /// the session speaks as), a fresh X25519 key, then `suffix`. No line
    /// feed; the caller adds it.
    ///
    /// For a gserver the prefix is the client type and key byte (or the
    /// type alone for gen 2) and the suffix is empty. For a listserver the
    /// prefix is id 7 and the key byte and the suffix is the list name.
    pub fn new(prefix: &[u8], inner: &str, suffix: &[u8]) -> Result<Self, Error> {
        if inner.len() != 8 {
            return Err(Error::Malformed("inner token is not 8 bytes"));
        }
        let secret = EphemeralSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        let mut hello = prefix.to_vec();
        hello.extend_from_slice(TOKEN_B);
        hello.extend_from_slice(inner.as_bytes());
        hello.extend_from_slice(B64.encode(public.as_bytes()).as_bytes());
        hello.extend_from_slice(suffix);
        Ok(Self { secret, hello })
    }

    /// The C1 line, without its line feed.
    pub fn hello(&self) -> &[u8] {
        &self.hello
    }

    /// Check S1 (one line, with or without its line feed). On success the
    /// result names the server's long-term key, which the caller must
    /// compare with its pin before it seals anything.
    pub fn finish(self, s1: &[u8]) -> Result<Verified, Error> {
        let s1 = s1.strip_suffix(b"\n").unwrap_or(s1);
        let rest = s1.strip_prefix(TOKEN_B).ok_or(Error::Malformed("server hello does not start with the token"))?;
        if rest.len() != KEY_B64 * 2 + SIG_B64 {
            return Err(Error::Malformed("server hello has the wrong length"));
        }
        let eph = key32(&rest[..KEY_B64])?;
        let id = key32(&rest[KEY_B64..KEY_B64 * 2])?;
        let sig: [u8; 64] = B64
            .decode(&rest[KEY_B64 * 2..])
            .map_err(|_| Error::Malformed("bad base64 signature"))?
            .try_into()
            .map_err(|_| Error::Malformed("signature is not 64 bytes"))?;
        let verifying = VerifyingKey::from_bytes(&id).map_err(|_| Error::Malformed("server key is not an Ed25519 key"))?;
        let signed = signed_bytes(&self.hello, &eph, &id);
        verifying.verify_strict(&signed, &Signature::from_bytes(&sig)).map_err(|_| Error::BadSignature)?;
        let shared = self.secret.diffie_hellman(&PublicKey::from(eph));
        let k = derive(&shared, &signed, &sig)?;
        Ok(Verified { server_key: id, sealer: Sealer { th: k.th, key: k.key, c2s: k.c2s, s2c: k.s2c } })
    }
}

/// A server hello that verified.
pub struct Verified {
    /// The server's long-term Ed25519 key. Compare it with the pin.
    pub server_key: [u8; 32],
    /// Seals the login once the key is trusted.
    pub sealer: Sealer,
}

/// Seals C2. Used once: the key is single-use, so the nonce is zero.
pub struct Sealer {
    th: [u8; 32],
    key: [u8; 32],
    c2s: [u8; 32],
    s2c: [u8; 32],
}

fn pad(tail: &[u8]) -> Result<Vec<u8>, Error> {
    if tail.len() > MAX_TAIL {
        return Err(Error::TooLong);
    }
    let mut p = (tail.len() as u16).to_le_bytes().to_vec();
    p.extend_from_slice(tail);
    let padded = p.len().div_ceil(PAD_TO) * PAD_TO;
    p.resize(padded, 0);
    Ok(p)
}

fn unpad(p: &[u8]) -> Result<Vec<u8>, Error> {
    let n = u16::from_le_bytes([*p.first().ok_or(Error::BadSeal)?, *p.get(1).ok_or(Error::BadSeal)?]) as usize;
    let tail = p.get(2..2 + n).ok_or(Error::BadSeal)?;
    if n > MAX_TAIL || p[2 + n..].iter().any(|&b| b != 0) {
        return Err(Error::BadSeal);
    }
    Ok(tail.to_vec())
}

impl Sealer {
    /// The client's end of an encrypted [`Channel`] instead of a sealed
    /// login (the gserver to listserver link, `documentation/secure-login.md`).
    pub fn into_channel(self) -> Channel {
        Channel::new(self.c2s, self.s2c)
    }

    /// C2: the token and the sealed `tail`, without a line feed. `tail` is
    /// what follows the version token in the classic message (for a
    /// gserver: the account GSTRING, the password GSTRING and any 5.x/6.x
    /// platform fields; for a listserver: id 1's payload).
    pub fn seal(self, tail: &[u8]) -> Result<Vec<u8>, Error> {
        let cipher = ChaCha20Poly1305::new(&self.key.into());
        let ct = cipher
            .encrypt(&[0u8; 12].into(), Payload { msg: &pad(tail)?, aad: &self.th })
            .map_err(|_| Error::TooLong)?;
        let mut line = TOKEN_B.to_vec();
        line.extend_from_slice(B64.encode(ct).as_bytes());
        Ok(line)
    }
}

/// C1 as the server reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHello {
    /// The bytes before the token (client type and key byte, or id 7 and key).
    pub prefix: Vec<u8>,
    /// The inner build's token.
    pub inner: [u8; 8],
    /// The bytes after the client's key (the list name on a listserver).
    pub suffix: Vec<u8>,
    client_key: [u8; 32],
    line: Vec<u8>,
}

impl ClientHello {
    /// Read C1. `prefix_len` is how many bytes come before the token: 2 for
    /// a gserver login with a key byte, 1 for gen 2, 2 for a listserver id 7.
    /// `None` when the token is not there (a classic login).
    pub fn parse(line: &[u8], prefix_len: usize) -> Option<Result<Self, Error>> {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if !is_token(line.get(prefix_len..prefix_len + 8)?) {
            return None;
        }
        let rest = &line[prefix_len + 8..];
        Some((|| {
            let inner: [u8; 8] = rest.get(..8).ok_or(Error::Malformed("no inner token"))?.try_into().unwrap();
            let key = key32(rest.get(8..8 + KEY_B64).ok_or(Error::Malformed("no client key"))?)?;
            Ok(Self { prefix: line[..prefix_len].to_vec(), inner, suffix: rest[8 + KEY_B64..].to_vec(), client_key: key, line: line.to_vec() })
        })())
    }

    /// The inner token as text.
    pub fn inner_token(&self) -> String {
        String::from_utf8_lossy(&self.inner).into_owned()
    }

    /// S1, without a line feed, and the opener for C2.
    pub fn respond(&self, identity: &ServerIdentity) -> Result<(Vec<u8>, Opener), Error> {
        let secret = EphemeralSecret::random_from_rng(OsRng);
        let eph = PublicKey::from(&secret).to_bytes();
        let id = identity.public_key();
        let signed = signed_bytes(&self.line, &eph, &id);
        let sig = identity.key.sign(&signed).to_bytes();
        let shared = secret.diffie_hellman(&PublicKey::from(self.client_key));
        let k = derive(&shared, &signed, &sig)?;
        let mut s1 = TOKEN_B.to_vec();
        for part in [&eph[..], &id[..], &sig[..]] {
            s1.extend_from_slice(B64.encode(part).as_bytes());
        }
        Ok((s1, Opener { th: k.th, key: k.key, c2s: k.c2s, s2c: k.s2c }))
    }
}

/// Opens C2 on the server.
pub struct Opener {
    th: [u8; 32],
    key: [u8; 32],
    c2s: [u8; 32],
    s2c: [u8; 32],
}

impl Opener {
    /// The server's end of an encrypted [`Channel`].
    pub fn into_channel(self) -> Channel {
        Channel::new(self.s2c, self.c2s)
    }

    /// The tail sealed in C2 (one line, with or without its line feed).
    pub fn open(self, c2: &[u8]) -> Result<Vec<u8>, Error> {
        let c2 = c2.strip_suffix(b"\n").unwrap_or(c2);
        let b64 = c2.strip_prefix(TOKEN_B).ok_or(Error::Malformed("sealed login does not start with the token"))?;
        let ct = B64.decode(b64).map_err(|_| Error::Malformed("bad base64 in the sealed login"))?;
        let cipher = ChaCha20Poly1305::new(&self.key.into());
        let p = cipher.decrypt(&[0u8; 12].into(), Payload { msg: &ct, aad: &self.th }).map_err(|_| Error::BadSeal)?;
        unpad(&p)
    }
}

/// The inner token of the gserver to listserver link: C1 carries it where
/// a login carries a build's token.
pub const LINK_TOKEN: &str = "SVLINK01";

/// The most plaintext one channel record holds.
pub const MAX_RECORD: usize = 16 * 1024;

/// An encrypted, authenticated byte stream after a handshake: each record
/// is `u16be length` and ChaCha20-Poly1305 of up to [`MAX_RECORD`] bytes,
/// under one key per direction, with the record count as the nonce
/// (4 zero bytes, then `u64be`). A record that does not open ends the link.
/// Records are opened in order, so a dropped, repeated or reordered record
/// fails.
pub struct Channel {
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    sent: u64,
    got: u64,
}

fn nonce(n: u64) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[4..].copy_from_slice(&n.to_be_bytes());
    b
}

impl Channel {
    fn new(send: [u8; 32], recv: [u8; 32]) -> Self {
        Self { send: ChaCha20Poly1305::new(&send.into()), recv: ChaCha20Poly1305::new(&recv.into()), sent: 0, got: 0 }
    }

    /// The records (with their length headers) carrying `data`.
    pub fn seal(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 18);
        for chunk in data.chunks(MAX_RECORD) {
            let ct = self.send.encrypt(&nonce(self.sent).into(), chunk).expect("a record is within the AEAD limit");
            self.sent += 1;
            out.extend_from_slice(&(ct.len() as u16).to_be_bytes());
            out.extend_from_slice(&ct);
        }
        out
    }

    /// Open one record's body (the bytes after its length).
    pub fn open(&mut self, record: &[u8]) -> Result<Vec<u8>, Error> {
        let pt = self.recv.decrypt(&nonce(self.got).into(), record).map_err(|_| Error::BadSeal)?;
        self.got += 1;
        Ok(pt)
    }

    /// Wrap a blocking stream: what is written is sealed, what is read is
    /// opened.
    pub fn wrap<S: std::io::Read + std::io::Write>(self, inner: S) -> SecureStream<S> {
        SecureStream { inner, ch: self, buf: Vec::new(), at: 0 }
    }
}

/// A [`Channel`] over a blocking stream.
pub struct SecureStream<S> {
    inner: S,
    ch: Channel,
    buf: Vec<u8>,
    at: usize,
}

impl<S> SecureStream<S> {
    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    #[cfg(test)]
    fn into_channel_for_test(self) -> Channel {
        self.ch
    }
}

impl<S: std::io::Read + std::io::Write> std::io::Read for SecureStream<S> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.at >= self.buf.len() {
            let mut len = [0u8; 2];
            match self.inner.read_exact(&mut len) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(0),
                Err(e) => return Err(e),
            }
            let mut rec = vec![0u8; u16::from_be_bytes(len) as usize];
            self.inner.read_exact(&mut rec)?;
            self.buf = self.ch.open(&rec).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            self.at = 0;
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl<S: std::io::Read + std::io::Write> std::io::Write for SecureStream<S> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let records = self.ch.seal(data);
        self.inner.write_all(&records)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// What the client's pin file says about a server key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinCheck {
    /// The key is the pinned one.
    Matches,
    /// Nothing is pinned for this server yet.
    Unknown,
    /// A different key is pinned: refuse the login.
    Mismatch { pinned: [u8; 32] },
}

/// The client's pins: one `server key` pair per line (`server` as the
/// player typed it, `host:port`; `key` as [`encode_key`] gives it). Lines
/// starting with `#` and blank lines are skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pins {
    entries: Vec<(String, [u8; 32])>,
}

impl Pins {
    /// Read pins from the file's text. Bad lines are skipped.
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.to_string(), decode_key(it.next()?)?))
            })
            .collect();
        Self { entries }
    }

    /// Read the pin file at `path`; a missing file has no pins.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => Ok(Self::parse(&t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn check(&self, server: &str, key: &[u8; 32]) -> PinCheck {
        match self.entries.iter().find(|(s, _)| s == server) {
            None => PinCheck::Unknown,
            Some((_, k)) if k == key => PinCheck::Matches,
            Some((_, k)) => PinCheck::Mismatch { pinned: *k },
        }
    }

    /// Append a pin for `server` to the file at `path` (trust on first use).
    pub fn append(path: &std::path::Path, server: &str, key: &[u8; 32]) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(f, "{server} {}", encode_key(key))
    }
}

/// The client's pin file: `known_servers.txt` in the settings folder.
pub fn pin_path() -> std::path::PathBuf {
    crate::config::config_dir().join("known_servers.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(tail: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let id = ServerIdentity::generate();
        let c = ClientHandshake::new(&[0x25, 0x41], "GNW28015", b"").unwrap();
        let c1 = c.hello().to_vec();
        let hello = ClientHello::parse(&c1, 2).unwrap().unwrap();
        assert_eq!(hello.inner_token(), "GNW28015");
        assert_eq!(hello.prefix, [0x25, 0x41]);
        let (s1, opener) = hello.respond(&id).unwrap();
        let v = c.finish(&s1).unwrap();
        assert_eq!(v.server_key, id.public_key());
        let c2 = v.sealer.seal(tail).unwrap();
        let opened = opener.open(&c2).unwrap();
        (c1, s1, c2, opened)
    }

    #[test]
    fn round_trip_opens_the_tail() {
        let tail = b"%account(password";
        let (c1, s1, c2, opened) = exchange(tail);
        assert_eq!(opened, tail);
        for line in [&c1, &s1, &c2] {
            assert!(line.iter().all(|&b| b >= 0x20 && b != b'\n'), "every byte is a message byte");
        }
        // Neither the account nor the password shows in any line.
        for line in [&c1, &s1, &c2] {
            assert!(!line.windows(7).any(|w| w == b"account"));
            assert!(!line.windows(8).any(|w| w == b"password"));
        }
    }

    #[test]
    fn ciphertext_is_padded() {
        let (_, _, short, _) = exchange(b"a");
        let (_, _, longer, _) = exchange(b"0123456789abcdef0123456789abcdef");
        assert_eq!(short.len(), longer.len());
    }

    #[test]
    fn classic_login_is_not_a_hello() {
        assert!(ClientHello::parse(b"%AGNW28015$acct$pass", 2).is_none());
    }

    #[test]
    fn listserver_suffix_is_kept() {
        let c = ClientHandshake::new(&[0x27, 0x41], "GNW30123", b"newmain").unwrap();
        let h = ClientHello::parse(c.hello(), 2).unwrap().unwrap();
        assert_eq!(h.suffix, b"newmain");
        assert_eq!(h.inner_token(), "GNW30123");
    }

    #[test]
    fn a_changed_hello_fails_the_signature() {
        // A man in the middle who changes the inner token (a downgrade)
        // or the key byte breaks the signature the client checks.
        let id = ServerIdentity::generate();
        let c = ClientHandshake::new(&[0x25, 0x41], "GNW28015", b"").unwrap();
        let mut tampered = c.hello().to_vec();
        tampered[10] = b'X';
        let hello = ClientHello::parse(&tampered, 2).unwrap().unwrap();
        let (s1, _) = hello.respond(&id).unwrap();
        assert_eq!(c.finish(&s1).err(), Some(Error::BadSignature));
    }

    #[test]
    fn a_relayed_hello_from_another_key_is_caught_by_the_pin() {
        let real = ServerIdentity::generate();
        let fake = ServerIdentity::generate();
        let c = ClientHandshake::new(&[0x25, 0x41], "GNW28015", b"").unwrap();
        let hello = ClientHello::parse(c.hello(), 2).unwrap().unwrap();
        let (s1, _) = hello.respond(&fake).unwrap();
        let v = c.finish(&s1).unwrap();
        let pins = Pins::parse(&format!("127.0.0.1:14802 {}\n", encode_key(&real.public_key())));
        assert_eq!(pins.check("127.0.0.1:14802", &v.server_key), PinCheck::Mismatch { pinned: real.public_key() });
        assert_eq!(pins.check("127.0.0.1:14803", &v.server_key), PinCheck::Unknown);
    }

    #[test]
    fn a_changed_seal_does_not_open() {
        let id = ServerIdentity::generate();
        let c = ClientHandshake::new(&[0x25, 0x41], "GNW28015", b"").unwrap();
        let hello = ClientHello::parse(c.hello(), 2).unwrap().unwrap();
        let (s1, opener) = hello.respond(&id).unwrap();
        let mut c2 = c.finish(&s1).unwrap().sealer.seal(b"secret").unwrap();
        let last = c2.len() - 1;
        c2[last] = if c2[last] == b'A' { b'B' } else { b'A' };
        assert!(opener.open(&c2).is_err());
    }

    #[test]
    fn a_seal_from_one_session_does_not_open_in_another() {
        let id = ServerIdentity::generate();
        let c = ClientHandshake::new(&[0x25, 0x41], "GNW28015", b"").unwrap();
        let c1 = c.hello().to_vec();
        let hello = ClientHello::parse(&c1, 2).unwrap().unwrap();
        let (s1, _) = hello.respond(&id).unwrap();
        let c2 = c.finish(&s1).unwrap().sealer.seal(b"secret").unwrap();
        // Replaying C1 and C2: the server's new one-time key gives a new seal key.
        let (_, opener) = hello.respond(&id).unwrap();
        assert_eq!(opener.open(&c2).err(), Some(Error::BadSeal));
    }

    #[test]
    fn identity_round_trips_through_its_seed() {
        let id = ServerIdentity::generate();
        let again = ServerIdentity::from_seed(&id.seed());
        assert_eq!(id.public_key(), again.public_key());
        assert_eq!(decode_key(&encode_key(&id.public_key())), Some(id.public_key()));
        assert!(fingerprint(&id.public_key()).starts_with("SHA256:"));
    }

    #[test]
    fn channel_carries_bytes_both_ways_and_refuses_tampering() {
        use std::io::{Read, Write};
        let id = ServerIdentity::generate();
        let c = ClientHandshake::new(b"", LINK_TOKEN, b"").unwrap();
        let hello = ClientHello::parse(c.hello(), 0).unwrap().unwrap();
        assert_eq!(hello.inner_token(), LINK_TOKEN);
        let (s1, opener) = hello.respond(&id).unwrap();
        let mut client = c.finish(&s1).unwrap().sealer.into_channel();
        let mut server = opener.into_channel();
        let big = vec![b'x'; MAX_RECORD + 10];
        let wire = client.seal(&big);
        assert!(!wire.windows(4).any(|w| w == b"xxxx"));
        // Two records; read them back through a stream.
        let mut s = server.wrap(std::io::Cursor::new(wire));
        let mut back = Vec::new();
        s.read_to_end(&mut back).unwrap();
        assert_eq!(back, big);
        // A record repeated, or opened by the wrong direction, does not open.
        let r = client.seal(b"secret");
        let mut server = s.into_channel_for_test();
        assert_eq!(server.open(&r[2..]).unwrap(), b"secret");
        assert!(server.open(&r[2..]).is_err(), "a repeated record fails");
        let mut client2 = client;
        let back = server.seal(b"pong");
        assert_eq!(client2.open(&back[2..]).unwrap(), b"pong");
        let mine = client2.seal(b"mine");
        assert!(client2.open(&mine[2..]).is_err(), "a record from the own direction fails");
        // Writing through the stream makes one record per write.
        let mut w = client2.wrap(std::io::Cursor::new(Vec::new()));
        w.write_all(b"hi").unwrap();
        assert_eq!(w.get_ref().get_ref().len(), 2 + 2 + 16);
    }

    #[test]
    fn identity_file_is_created_once() {
        let dir = std::env::temp_dir().join(format!("seihai-idkey-{}", std::process::id()));
        let path = dir.join("seihai-login.key");
        let _ = std::fs::remove_dir_all(&dir);
        let (a, created) = ServerIdentity::load_or_create(&path).unwrap();
        assert!(created);
        let (b, created) = ServerIdentity::load_or_create(&path).unwrap();
        assert!(!created);
        assert_eq!(a.public_key(), b.public_key());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pins_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("seihai-pins-{}", std::process::id()));
        let path = dir.join("known_servers.txt");
        let key = ServerIdentity::generate().public_key();
        assert_eq!(Pins::load(&path).unwrap().check("127.0.0.1:1", &key), PinCheck::Unknown);
        Pins::append(&path, "127.0.0.1:1", &key).unwrap();
        assert_eq!(Pins::load(&path).unwrap().check("127.0.0.1:1", &key), PinCheck::Matches);
        let _ = std::fs::remove_dir_all(dir);
    }
}
