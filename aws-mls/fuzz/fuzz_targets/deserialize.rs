#![no_main]
use aws_mls::{aws_mls_codec::MlsDecode, MLSMessage};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = MLSMessage::mls_decode(data);
});
