use concerto_reflect::ReflectedValue;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A 64-bit FNV-1a hasher whose output is stable across builds and platforms.
pub struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a {
    /// A hasher at the FNV-1a offset basis.
    pub fn new() -> Self {
        Self(FNV_OFFSET)
    }

    /// Feeds `bytes` as they are, without their length.
    pub fn raw(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
        }
    }

    /// Feeds `count` as eight little-endian bytes.
    pub fn count(&mut self, count: usize) {
        self.raw(&(count as u64).to_le_bytes());
    }

    /// Feeds the length of `bytes`, then `bytes`.
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.count(bytes.len());
        self.raw(bytes);
    }

    fn fields<'a>(
        &mut self,
        count: usize,
        fields: impl Iterator<Item = &'a (String, ReflectedValue)>,
    ) {
        self.count(count);
        for (name, value) in fields {
            self.bytes(name.as_bytes());
            self.value(value);
        }
    }

    /// Feeds a reflected value, tagging each node with its kind.
    pub fn value(&mut self, value: &ReflectedValue) {
        match value {
            ReflectedValue::Leaf(bytes) => {
                self.raw(&[0]);
                self.bytes(bytes);
            }
            ReflectedValue::Struct(fields) => {
                self.raw(&[1]);
                self.fields(fields.len(), fields.iter());
            }
            ReflectedValue::Variant { name, fields } => {
                self.raw(&[2]);
                self.bytes(name.as_bytes());
                self.fields(fields.len(), fields.iter());
            }
            ReflectedValue::List(elements) => {
                self.raw(&[3]);
                self.count(elements.len());
                for element in elements {
                    self.value(element);
                }
            }
        }
    }

    /// The hash of everything fed so far.
    pub fn finish(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> u64 {
        let mut hash = Fnv1a::new();
        hash.raw(bytes);
        hash.finish()
    }

    #[test]
    fn the_hash_is_fnv_1a() {
        assert_eq!(digest(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(digest(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(digest(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
