use encase::{internal::WriteInto, ShaderType, UniformBuffer};

pub(crate) fn uniform_words<T: ShaderType + WriteInto>(value: &T) -> Vec<u32> {
    words(&UniformBuffer::<Vec<u8>>::content_of::<T, Vec<u8>>(value).unwrap())
}

pub(crate) fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

pub(crate) fn float_words(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

pub(crate) fn gpu<T, U: From<T>>(value: T) -> U {
    U::from(value)
}
