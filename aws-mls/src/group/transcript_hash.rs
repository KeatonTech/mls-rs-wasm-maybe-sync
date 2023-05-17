use super::*;
use aws_mls_core::error::IntoAnyError;
use core::ops::Deref;

#[derive(Debug, Clone, PartialEq, Eq, MlsSize, MlsEncode, MlsDecode)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ConfirmedTranscriptHash(#[mls_codec(with = "aws_mls_codec::byte_vec")] Vec<u8>);

impl Deref for ConfirmedTranscriptHash {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<u8>> for ConfirmedTranscriptHash {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl ConfirmedTranscriptHash {
    pub(crate) fn create<P: CipherSuiteProvider>(
        cipher_suite_provider: &P,
        interim_transcript_hash: &InterimTranscriptHash,
        content: &AuthenticatedContent,
    ) -> Result<Self, MlsError> {
        #[derive(Debug, MlsSize, MlsEncode)]
        struct ConfirmedTranscriptHashInput<'a> {
            wire_format: WireFormat,
            content: &'a FramedContent,
            signature: &'a MessageSignature,
        }

        let input = ConfirmedTranscriptHashInput {
            wire_format: content.wire_format,
            content: &content.content,
            signature: &content.auth.signature,
        };

        let hash_input = [
            interim_transcript_hash.deref(),
            input.mls_encode_to_vec()?.deref(),
        ]
        .concat();

        cipher_suite_provider
            .hash(&hash_input)
            .map(Into::into)
            .map_err(|e| MlsError::CryptoProviderError(e.into_any_error()))
    }
}

#[derive(Debug, Clone, PartialEq, MlsSize, MlsEncode, MlsDecode)]
pub(crate) struct InterimTranscriptHash(#[mls_codec(with = "aws_mls_codec::byte_vec")] Vec<u8>);

impl Deref for InterimTranscriptHash {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<u8>> for InterimTranscriptHash {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl InterimTranscriptHash {
    pub fn create<P: CipherSuiteProvider>(
        cipher_suite_provider: &P,
        confirmed: &ConfirmedTranscriptHash,
        confirmation_tag: &ConfirmationTag,
    ) -> Result<Self, MlsError> {
        #[derive(Debug, MlsSize, MlsEncode)]
        struct InterimTranscriptHashInput<'a> {
            confirmation_tag: &'a ConfirmationTag,
        }

        let input = InterimTranscriptHashInput { confirmation_tag }.mls_encode_to_vec()?;

        cipher_suite_provider
            .hash(&[confirmed.0.deref(), &input].concat())
            .map(Into::into)
            .map_err(|e| MlsError::CryptoProviderError(e.into_any_error()))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use aws_mls_codec::{MlsDecode, MlsEncode};
    use aws_mls_core::crypto::{CipherSuite, CipherSuiteProvider};

    use crate::{
        crypto::test_utils::{test_cipher_suite_provider, try_test_cipher_suite_provider},
        group::{
            confirmation_tag::ConfirmationTag,
            framing::{Content, ContentType},
            message_signature::AuthenticatedContent,
            proposal::ProposalOrRef,
            proposal_ref::ProposalRef,
            test_utils::get_test_group_context,
            transcript_hashes, Commit, Sender,
        },
        WireFormat,
    };

    use super::{ConfirmedTranscriptHash, InterimTranscriptHash};

    #[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
    struct TestCase {
        pub cipher_suite: u16,

        #[serde(with = "hex::serde")]
        pub confirmation_key: Vec<u8>,
        #[serde(with = "hex::serde")]
        pub authenticated_content: Vec<u8>,
        #[serde(with = "hex::serde")]
        pub interim_transcript_hash_before: Vec<u8>,

        #[serde(with = "hex::serde")]
        pub confirmed_transcript_hash_after: Vec<u8>,
        #[serde(with = "hex::serde")]
        pub interim_transcript_hash_after: Vec<u8>,
    }

    #[maybe_async::test(sync, async(not(sync), futures_test::test))]
    async fn transcript_hash() {
        #[cfg(not(sync))]
        let test_cases: Vec<TestCase> =
            load_test_case_json!(interop_transcript_hashes, generate_test_vector().await);

        #[cfg(sync)]
        let test_cases: Vec<TestCase> =
            load_test_case_json!(interop_transcript_hashes, generate_test_vector());

        for test_case in test_cases.into_iter() {
            let Some(cs) = try_test_cipher_suite_provider(test_case.cipher_suite) else {
                continue;
            };

            let auth_content =
                AuthenticatedContent::mls_decode(&mut &*test_case.authenticated_content).unwrap();

            assert!(auth_content.content.content_type() == ContentType::Commit);

            let conf_key = &test_case.confirmation_key;
            let conf_hash_after = test_case.confirmed_transcript_hash_after.into();
            let conf_tag = auth_content.auth.confirmation_tag.clone().unwrap();

            assert!(conf_tag.matches(conf_key, &conf_hash_after, &cs).unwrap());

            let (expected_interim, expected_conf) = transcript_hashes(
                &cs,
                &test_case.interim_transcript_hash_before.into(),
                &auth_content,
            )
            .unwrap();

            assert_eq!(*expected_interim, test_case.interim_transcript_hash_after);
            assert_eq!(expected_conf, conf_hash_after);
        }
    }

    #[maybe_async::maybe_async]
    async fn generate_test_vector() -> Vec<TestCase> {
        CipherSuite::all().fold(vec![], |mut test_cases, cs| {
            let cs = test_cipher_suite_provider(cs);

            let context = get_test_group_context(0x3456, cs.cipher_suite());

            let proposal_ref = ProposalRef::new_fake(cs.hash(&[9, 9, 9]).unwrap());
            let proposal_ref = ProposalOrRef::Reference(proposal_ref);

            let commit = Commit {
                proposals: vec![proposal_ref],
                path: None,
            };

            let signer = cs.signature_key_generate().unwrap().0;

            let mut auth_content = AuthenticatedContent::new_signed(
                &cs,
                &context,
                Sender::Member(0),
                Content::Commit(commit),
                &signer,
                WireFormat::PublicMessage,
                vec![],
            )
            .unwrap();

            let interim_hash_before = cs.random_bytes_vec(cs.kdf_extract_size()).unwrap().into();

            let conf_hash_after =
                ConfirmedTranscriptHash::create(&cs, &interim_hash_before, &auth_content).unwrap();

            let conf_key = cs.random_bytes_vec(cs.kdf_extract_size()).unwrap();
            let conf_tag = ConfirmationTag::create(&conf_key, &conf_hash_after, &cs).unwrap();

            let interim_hash_after =
                InterimTranscriptHash::create(&cs, &conf_hash_after, &conf_tag).unwrap();

            auth_content.auth.confirmation_tag = Some(conf_tag);

            let test_case = TestCase {
                cipher_suite: cs.cipher_suite().into(),

                confirmation_key: conf_key,
                authenticated_content: auth_content.mls_encode_to_vec().unwrap(),
                interim_transcript_hash_before: interim_hash_before.0,

                confirmed_transcript_hash_after: conf_hash_after.0,
                interim_transcript_hash_after: interim_hash_after.0,
            };

            test_cases.push(test_case);
            test_cases
        })
    }
}
