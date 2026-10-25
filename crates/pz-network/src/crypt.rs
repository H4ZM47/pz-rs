//! Account password hashing: `PZcrypt.hash(ServerWorldDatabase.encrypt(pwd))`,
//! a bcrypt (`$2a$`, cost 12, the game's fixed salt) of the password's MD5
//! in lower-case hex.

use std::sync::OnceLock;

/// `PZcrypt.salt`.
const SALT: &str = "$2a$12$O/BFHoDFPrfFaNPAACmWpu";

/// `Account.encryptPwd(pwd)` / `Server.setPwd(pwd, true)`.
pub fn hash_password(pwd: &str) -> String {
    pz_hash(&md5_hex(pwd))
}

/// `ServerWorldDatabase.encrypt`: "" for an empty password.
pub fn md5_hex(pwd: &str) -> String {
    if pwd.is_empty() {
        return String::new();
    }
    md5(pwd.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `PZcrypt.hash(s)`: an empty string stays empty.
pub fn pz_hash(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    bcrypt_hashpw(s, SALT).expect("valid salt")
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let mut h: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
            .collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(k[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 16];
    for (i, w) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// Blowfish's initial P-array and S-boxes: the first 1042 32-bit words of
/// the fractional part of pi, computed once (Machin's formula in fixed
/// point).
fn pi_words() -> &'static [u32] {
    static WORDS: OnceLock<Vec<u32>> = OnceLock::new();
    WORDS.get_or_init(|| {
        const WANT: usize = 18 + 4 * 256;
        let n = WANT + 4; // limbs after the integer limb, with guard limbs
                          // x / d in place (big-endian limbs, limb 0 the integer part).
        fn div(x: &mut [u32], d: u32) {
            let mut r = 0u64;
            for l in x.iter_mut() {
                let v = (r << 32) | *l as u64;
                *l = (v / d as u64) as u32;
                r = v % d as u64;
            }
        }
        fn add(x: &mut [u32], y: &[u32]) {
            let mut carry = 0u64;
            for (a, b) in x.iter_mut().zip(y).rev() {
                let v = *a as u64 + *b as u64 + carry;
                *a = v as u32;
                carry = v >> 32;
            }
        }
        fn sub(x: &mut [u32], y: &[u32]) {
            let mut borrow = 0i64;
            for (a, b) in x.iter_mut().zip(y).rev() {
                let v = *a as i64 - *b as i64 - borrow;
                *a = v as u32;
                borrow = (v < 0) as i64;
            }
        }
        // mul * atan(1/x), added into acc (sign by `neg`).
        let atan = |acc: &mut Vec<u32>, x: u32, mul: u32, neg: bool| {
            let mut p = vec![0u32; n + 1];
            p[0] = mul;
            div(&mut p, x);
            let mut k = 0u32;
            loop {
                if p.iter().all(|&l| l == 0) {
                    break;
                }
                let mut t = p.clone();
                div(&mut t, 2 * k + 1);
                if (k % 2 == 1) != neg {
                    sub(acc, &t);
                } else {
                    add(acc, &t);
                }
                div(&mut p, x);
                div(&mut p, x);
                k += 1;
            }
        };
        let mut pi = vec![0u32; n + 1];
        atan(&mut pi, 5, 16, false);
        atan(&mut pi, 239, 4, true);
        pi[1..=WANT].to_vec()
    })
}

struct Blowfish {
    p: [u32; 18],
    s: [[u32; 256]; 4],
}

impl Blowfish {
    fn new() -> Blowfish {
        let w = pi_words();
        let mut bf = Blowfish {
            p: [0; 18],
            s: [[0; 256]; 4],
        };
        bf.p.copy_from_slice(&w[..18]);
        for (i, sbox) in bf.s.iter_mut().enumerate() {
            sbox.copy_from_slice(&w[18 + 256 * i..18 + 256 * (i + 1)]);
        }
        bf
    }

    fn f(&self, x: u32) -> u32 {
        let [a, b, c, d] = x.to_be_bytes();
        (self.s[0][a as usize].wrapping_add(self.s[1][b as usize]) ^ self.s[2][c as usize])
            .wrapping_add(self.s[3][d as usize])
    }

    fn encipher(&self, l: &mut u32, r: &mut u32) {
        let (mut xl, mut xr) = (*l, *r);
        xl ^= self.p[0];
        for i in (1..=16).step_by(2) {
            xr ^= self.f(xl) ^ self.p[i];
            xl ^= self.f(xr) ^ self.p[i + 1];
        }
        *l = xr ^ self.p[17];
        *r = xl;
    }

    /// jBCrypt's `streamtoword`: four bytes, cycling over `data`.
    fn word(data: &[u8], off: &mut usize) -> u32 {
        let mut w = 0u32;
        for _ in 0..4 {
            w = (w << 8) | data[*off] as u32;
            *off = (*off + 1) % data.len();
        }
        w
    }

    fn expand(&mut self, salt: Option<&[u8]>, key: &[u8]) {
        let mut ko = 0;
        for i in 0..18 {
            self.p[i] ^= Self::word(key, &mut ko);
        }
        let (mut l, mut r) = (0u32, 0u32);
        let mut so = 0;
        let mut next = |l: &mut u32, r: &mut u32| {
            if let Some(s) = salt {
                *l ^= Self::word(s, &mut so);
                *r ^= Self::word(s, &mut so);
            }
        };
        for i in (0..18).step_by(2) {
            next(&mut l, &mut r);
            self.encipher(&mut l, &mut r);
            self.p[i] = l;
            self.p[i + 1] = r;
        }
        for b in 0..4 {
            for i in (0..256).step_by(2) {
                next(&mut l, &mut r);
                self.encipher(&mut l, &mut r);
                self.s[b][i] = l;
                self.s[b][i + 1] = r;
            }
        }
    }
}

const B64: &[u8; 64] = b"./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn b64_encode(d: &[u8], len: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < len {
        let c1 = d[i] as u32;
        i += 1;
        out.push(B64[(c1 >> 2) as usize & 0x3f] as char);
        let mut c = (c1 & 0x03) << 4;
        if i >= len {
            out.push(B64[c as usize] as char);
            break;
        }
        let c2 = d[i] as u32;
        i += 1;
        c |= (c2 >> 4) & 0x0f;
        out.push(B64[c as usize] as char);
        c = (c2 & 0x0f) << 2;
        if i >= len {
            out.push(B64[c as usize] as char);
            break;
        }
        let c3 = d[i] as u32;
        i += 1;
        c |= (c3 >> 6) & 0x03;
        out.push(B64[c as usize] as char);
        out.push(B64[(c3 & 0x3f) as usize] as char);
    }
    out
}

fn b64_decode(s: &str, max: usize) -> Option<Vec<u8>> {
    let v = |c: u8| B64.iter().position(|&x| x == c).map(|p| p as u8);
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() && out.len() < max {
        let c1 = v(b[i])?;
        let c2 = v(b[i + 1])?;
        out.push((c1 << 2) | ((c2 & 0x30) >> 4));
        if out.len() >= max || i + 2 >= b.len() {
            break;
        }
        let c3 = v(b[i + 2])?;
        out.push(((c2 & 0x0f) << 4) | ((c3 & 0x3c) >> 2));
        if out.len() >= max || i + 3 >= b.len() {
            break;
        }
        let c4 = v(b[i + 3])?;
        out.push(((c3 & 0x03) << 6) | c4);
        i += 4;
    }
    Some(out)
}

/// jBCrypt's `hashpw(password, salt)` for `$2a$`/`$2$` salts.
pub fn bcrypt_hashpw(password: &str, salt: &str) -> Option<String> {
    let rest = salt.strip_prefix("$2")?;
    let (minor, rest) = match rest.as_bytes().first()? {
        b'$' => (None, &rest[1..]),
        &m => (Some(m as char), rest.get(1..)?.strip_prefix('$')?),
    };
    let rounds: u32 = rest.get(..2)?.parse().ok()?;
    if rest.as_bytes().get(2) != Some(&b'$') || !(4..=30).contains(&rounds) {
        return None;
    }
    let real_salt = rest.get(3..25)?;
    let salt_bytes = b64_decode(real_salt, 16)?;
    let mut key = password.as_bytes().to_vec();
    if minor.is_some_and(|m| m >= 'a') {
        key.push(0);
    }
    let mut bf = Blowfish::new();
    bf.expand(Some(&salt_bytes), &key);
    for _ in 0..1u64 << rounds {
        bf.expand(None, &key);
        bf.expand(None, &salt_bytes);
    }
    let mut c: Vec<u32> = b"OrpheanBeholderScryDoubt"
        .chunks(4)
        .map(|w| u32::from_be_bytes(w.try_into().unwrap()))
        .collect();
    for _ in 0..64 {
        for j in (0..6).step_by(2) {
            let (mut l, mut r) = (c[j], c[j + 1]);
            bf.encipher(&mut l, &mut r);
            c[j] = l;
            c[j + 1] = r;
        }
    }
    let bytes: Vec<u8> = c.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut out = String::from("$2");
    if let Some(m) = minor {
        out.push(m);
    }
    out += &format!("${rounds:02}$");
    out += &b64_encode(&salt_bytes, 16);
    out += &b64_encode(&bytes, 23);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_words_are_blowfish_constants() {
        let w = pi_words();
        assert_eq!((w[0], w[1], w[17]), (0x243F6A88, 0x85A308D3, 0x8979FB1B));
        assert_eq!((w[18], w[1041]), (0xD1310BA6, 0x3AC372E6));
    }

    #[test]
    fn md5_and_bcrypt_known_values() {
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(""), "");
        // OpenBSD / jBCrypt test vector.
        assert_eq!(
            bcrypt_hashpw("", "$2a$06$DCq7YPn5Rq63x1Lad4cll.").unwrap(),
            "$2a$06$DCq7YPn5Rq63x1Lad4cll.TV4S6ytwfsfvkgY8jIucDrjc8deX1s."
        );
        assert_eq!(
            bcrypt_hashpw(
                "abcdefghijklmnopqrstuvwxyz",
                "$2a$06$.rCVZVOThsIa97pEDOxvGu"
            )
            .unwrap(),
            "$2a$06$.rCVZVOThsIa97pEDOxvGuRRgzG64bvtJ0938xuqzv18d3ZpQhstC"
        );
        assert_eq!(pz_hash(""), "");
    }

    /// `Account.encryptPwd` in the original.
    #[test]
    fn hash_password_matches_the_original() {
        assert_eq!(
            hash_password("hunter2"),
            "$2a$12$O/BFHoDFPrfFaNPAACmWpuMkn9y4vuoUFVDFkhdgcNoNaqxtwdV0a"
        );
    }

    /// Cost 12 is slow in debug builds: `cargo test --release -- --ignored`.
    #[test]
    #[ignore]
    fn hash_password_matches_the_original_more() {
        assert_eq!(
            hash_password("p\u{e4}ssw\u{f6}rd"),
            "$2a$12$O/BFHoDFPrfFaNPAACmWpunJGl37gDA.mm6agByYpSJxRD90681m2"
        );
        assert_eq!(
            hash_password(" x "),
            "$2a$12$O/BFHoDFPrfFaNPAACmWpuaN.KFo/RafbuBNyh2jjMFQ0TUZGjd6i"
        );
    }
}
