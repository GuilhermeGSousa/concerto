use encase::{internal::WriteInto, ShaderType, UniformBuffer};

pub(crate) fn uniform_words<T: ShaderType + WriteInto>(value: &T) -> Vec<u32> {
    words(&UniformBuffer::<Vec<u8>>::content_of::<T, Vec<u8>>(value).unwrap())
}

pub(crate) fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect()
}

pub(crate) fn float_words(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}
