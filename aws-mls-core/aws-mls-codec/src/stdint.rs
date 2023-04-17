use crate::{MlsDecode, MlsEncode, MlsSize};

macro_rules! impl_stdint {
    ($t:ty) => {
        impl MlsSize for $t {
            fn mls_encoded_len(&self) -> usize {
                core::mem::size_of::<$t>()
            }
        }

        impl MlsEncode for $t {
            fn mls_encode<W: crate::Writer>(&self, mut writer: W) -> Result<(), crate::Error> {
                writer.write(&self.to_be_bytes())
            }
        }

        impl MlsDecode for $t {
            fn mls_decode(reader: &mut &[u8]) -> Result<Self, crate::Error> {
                MlsDecode::mls_decode(reader).map(<$t>::from_be_bytes)
            }
        }
    };
}

impl_stdint!(u8);
impl_stdint!(u16);
impl_stdint!(u32);
impl_stdint!(u64);

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use crate::{MlsDecode, MlsEncode};

    use alloc::vec;

    #[test]
    fn u8_round_trip() {
        let serialized = 42u8.mls_encode_to_vec().unwrap();
        assert_eq!(serialized, vec![42u8]);

        let recovered = u8::mls_decode(&mut &*serialized).unwrap();

        assert_eq!(recovered, 42u8);
    }

    #[test]
    fn u16_round_trip() {
        let serialized = 1024u16.mls_encode_to_vec().unwrap();
        assert_eq!(serialized, vec![4, 0]);

        let recovered = u16::mls_decode(&mut &*serialized).unwrap();

        assert_eq!(recovered, 1024u16);
    }

    #[test]
    fn u32_round_trip() {
        let serialized = 1000000u32.mls_encode_to_vec().unwrap();
        assert_eq!(serialized, vec![0, 15, 66, 64]);

        let recovered = u32::mls_decode(&mut &*serialized).unwrap();

        assert_eq!(recovered, 1000000u32);
    }

    #[test]
    fn u64_round_trip() {
        let serialized = 100000000000u64.mls_encode_to_vec().unwrap();
        assert_eq!(serialized, vec![0, 0, 0, 23, 72, 118, 232, 0]);

        let recovered = u64::mls_decode(&mut &*serialized).unwrap();

        assert_eq!(recovered, 100000000000u64);
    }
}
