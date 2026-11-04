//! CPython integer-seeded MT19937 and Random.sample compatibility.
//! The algorithm identifier is persisted; it must not silently become rand's RNG.
pub const PROFILE: &str = "cpython-mt19937-integer-v1";
pub struct Random {
    state: [u32; 624],
    index: usize,
}
impl Random {
    pub fn new(seed: u64) -> Self {
        let key = if seed > u32::MAX as u64 {
            vec![seed as u32, (seed >> 32) as u32]
        } else {
            vec![seed as u32]
        };
        Self::key(key)
    }
    pub fn key(key: Vec<u32>) -> Self {
        let key = if key.is_empty() { vec![0] } else { key };
        let mut r = Self {
            state: [0; 624],
            index: 624,
        };
        r.state[0] = 19650218;
        for i in 1..624 {
            r.state[i] = 1812433253u32
                .wrapping_mul(r.state[i - 1] ^ (r.state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        let (mut i, mut j) = (1, 0);
        for _ in 0..624.max(key.len()) {
            r.state[i] = (r.state[i]
                ^ (r.state[i - 1] ^ (r.state[i - 1] >> 30)).wrapping_mul(1664525))
            .wrapping_add(key[j])
            .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                r.state[0] = r.state[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            r.state[i] = (r.state[i]
                ^ (r.state[i - 1] ^ (r.state[i - 1] >> 30)).wrapping_mul(1566083941))
            .wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                r.state[0] = r.state[623];
                i = 1;
            }
        }
        r.state[0] = 0x80000000;
        r
    }
    pub fn decimal(text: &str) -> crate::Result<Self> {
        let digits = text.trim().trim_start_matches(['-', '+']);
        if digits.is_empty() || digits.len() > 4096 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(crate::artifact::invalid("seed must be bounded integer"));
        }
        let mut words = vec![0u32];
        for digit in digits.bytes() {
            let mut carry = (digit - b'0') as u64;
            for word in &mut words {
                let n = *word as u64 * 10 + carry;
                *word = n as u32;
                carry = n >> 32;
            }
            if carry > 0 {
                words.push(carry as u32);
            }
        }
        while words.len() > 1 && words.last() == Some(&0) {
            words.pop();
        }
        Ok(Self::key(words))
    }
    pub fn random(&mut self) -> f64 {
        let a = (self.word() >> 5) as u64;
        let b = (self.word() >> 6) as u64;
        ((a << 26) + b) as f64 / 9007199254740992.0
    }
    pub fn text(seed: &str) -> Self {
        use sha2::{Digest, Sha512};
        let mut bytes = seed.as_bytes().to_vec();
        bytes.extend_from_slice(&Sha512::digest(seed.as_bytes()));
        let key = bytes
            .rchunks(4)
            .map(|chunk| {
                let mut word = [0; 4];
                word[4 - chunk.len()..].copy_from_slice(chunk);
                u32::from_be_bytes(word)
            })
            .collect();
        Self::key(key)
    }
    pub fn float(seed: f64) -> Self {
        if seed == 0.0 {
            return Self::new(0);
        }
        let bits = seed.to_bits();
        let exponent = ((bits >> 52) & 2047) as i32 - 1023 - 52;
        let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        let e = exponent.rem_euclid(61) as u32;
        let p = (1u128 << 61) - 1;
        let hash = ((mantissa as u128 * ((1u128) << e)) % p) as u64;
        Self::new(hash)
    }
    pub fn bits(&mut self, n: u32) -> u64 {
        if n == 0 {
            0
        } else if n <= 32 {
            (self.word() >> (32 - n)) as u64
        } else {
            self.word() as u64 | (((self.word() >> (64 - n)) as u64) << 32)
        }
    }
    pub fn word(&mut self) -> u32 {
        if self.index >= 624 {
            for k in 0..624 {
                let y = (self.state[k] & 0x80000000) | (self.state[(k + 1) % 624] & 0x7fffffff);
                self.state[k] = self.state[(k + 397) % 624]
                    ^ (y >> 1)
                    ^ if y & 1 != 0 { 0x9908b0df } else { 0 };
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y
    }
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0);
        let bits = usize::BITS - n.leading_zeros();
        loop {
            let v = if bits <= 32 {
                (self.word() >> (32 - bits)) as u64
            } else {
                self.word() as u64 | ((self.word() >> (64 - bits)) as u64) << 32
            };
            if v < n as u64 {
                return v as usize;
            }
        }
    }
    pub fn sample(&mut self, n: usize, k: usize) -> Vec<usize> {
        let k = k.min(n);
        let mut setsize = 21;
        if k > 5 {
            let mut p = 4;
            while p < k * 3 {
                p *= 4;
            }
            setsize += p;
        }
        if n <= setsize {
            let mut pool = (0..n).collect::<Vec<_>>();
            (0..k)
                .map(|i| {
                    let j = self.below(n - i);
                    let x = pool[j];
                    pool[j] = pool[n - i - 1];
                    x
                })
                .collect()
        } else {
            let mut seen = std::collections::BTreeSet::new();
            let mut out = vec![];
            while out.len() < k {
                let j = self.below(n);
                if seen.insert(j) {
                    out.push(j);
                }
            }
            out
        }
    }
}
pub fn bootstrap(values: &[f64], rounds: usize, seed: u64) -> Option<[f64; 2]> {
    if values.is_empty() || rounds == 0 {
        return None;
    }
    let mut rng = Random::new(seed);
    let mut means = (0..rounds)
        .map(|_| {
            (0..values.len())
                .map(|_| values[rng.below(values.len())])
                .sum::<f64>()
                / values.len() as f64
        })
        .collect::<Vec<_>>();
    means.sort_by(f64::total_cmp);
    Some([
        means[(rounds as f64 * 0.025) as usize],
        means[((rounds as f64 * 0.975) as usize)
            .saturating_sub(1)
            .min(rounds - 1)],
    ])
}
pub fn sign_test(values: &[f64]) -> f64 {
    let positive = values.iter().filter(|v| **v > 0.0).count();
    let negative = values.iter().filter(|v| **v < 0.0).count();
    let n = positive + negative;
    if n == 0 {
        return 1.0;
    }
    let k = positive.min(negative);
    let mut term = 2f64.powi(-(n as i32));
    let mut p = term;
    for j in 1..=k {
        term *= ((n - j + 1) as f64) / (j as f64);
        p += term;
    }
    (2.0 * p).min(1.0)
}
