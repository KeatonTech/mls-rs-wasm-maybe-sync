use core::ops::Deref;

use alloc::vec::Vec;
use aws_mls_codec::{MlsDecode, MlsEncode, MlsSize};

use super::BasicCredential;

#[cfg(feature = "x509")]
use super::CertificateChain;

#[derive(
    Debug, PartialEq, Eq, Hash, Clone, Copy, PartialOrd, Ord, MlsSize, MlsEncode, MlsDecode,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Wrapper type representing a credential type identifier along with default
/// values defined by the MLS RFC.
pub struct CredentialType(u16);

impl CredentialType {
    /// Basic identity.
    pub const BASIC: CredentialType = CredentialType(1);

    #[cfg(feature = "x509")]
    /// X509 Certificate Identity.
    pub const X509: CredentialType = CredentialType(2);

    pub const fn new(raw_value: u16) -> Self {
        CredentialType(raw_value)
    }

    pub const fn raw_value(&self) -> u16 {
        self.0
    }
}

impl From<u16> for CredentialType {
    fn from(value: u16) -> Self {
        CredentialType(value)
    }
}

impl Deref for CredentialType {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, MlsSize, MlsEncode, MlsDecode, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Custom user created credential type.
///
/// # Warning
///
/// In order to use a custom credential within an MLS group, a supporting
/// [`IdentityProvider`](crate::IdentityProvider) must be created that can
/// authenticate the credential.
pub struct CustomCredential {
    pub(crate) credential_type: CredentialType,
    #[mls_codec(with = "aws_mls_codec::byte_vec")]
    pub(crate) data: Vec<u8>,
}

impl CustomCredential {
    /// Create a new custom credential with opaque data.
    ///
    /// # Warning
    ///
    /// Using any of the constants defined within [`CredentialType`] will
    /// result in unspecified behavior.
    pub fn new(credential_type: CredentialType, data: Vec<u8>) -> CustomCredential {
        CustomCredential {
            credential_type,
            data,
        }
    }

    /// Unique credential type to identify this custom credential.
    pub fn credential_type(&self) -> CredentialType {
        self.credential_type
    }

    /// Opaque data representing this custom credential.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Clone, Debug, PartialEq, Ord, PartialOrd, Eq, Hash)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// A MLS credential used to authenticate a group member.
#[non_exhaustive]
pub enum Credential {
    /// Basic identifier-only credential.
    ///
    /// # Warning
    ///
    /// Basic credentials are inherently insecure since they can not be
    /// properly validated. It is not recommended to use [`BasicCredential`]
    /// in production applications.
    Basic(BasicCredential),
    #[cfg(feature = "x509")]
    /// X.509 Certificate chain.
    X509(CertificateChain),
    /// User provided custom credential.
    Custom(CustomCredential),
}

impl Credential {
    /// Credential type of the underlying credential.
    pub fn credential_type(&self) -> CredentialType {
        match self {
            Credential::Basic(_) => CredentialType::BASIC,
            #[cfg(feature = "x509")]
            Credential::X509(_) => CredentialType::X509,
            Credential::Custom(c) => c.credential_type,
        }
    }

    /// Convert this enum into a [`BasicCredential`]
    ///
    /// Returns `None` if this credential is any other type.
    pub fn as_basic(&self) -> Option<&BasicCredential> {
        match self {
            Credential::Basic(basic) => Some(basic),
            _ => None,
        }
    }

    /// Convert this enum into a [`CertificateChain`]
    ///
    /// Returns `None` if this credential is any other type.
    #[cfg(feature = "x509")]
    pub fn as_x509(&self) -> Option<&CertificateChain> {
        match self {
            Credential::X509(chain) => Some(chain),
            _ => None,
        }
    }

    /// Convert this enum into a [`CustomCredential`]
    ///
    /// Returns `None` if this credential is any other type.
    pub fn as_custom(&self) -> Option<&CustomCredential> {
        match self {
            Credential::Custom(custom) => Some(custom),
            _ => None,
        }
    }
}

impl MlsSize for Credential {
    fn mls_encoded_len(&self) -> usize {
        let inner_len = match self {
            Credential::Basic(c) => c.mls_encoded_len(),
            #[cfg(feature = "x509")]
            Credential::X509(c) => c.mls_encoded_len(),
            Credential::Custom(c) => aws_mls_codec::byte_vec::mls_encoded_len(&c.data),
        };

        self.credential_type().mls_encoded_len() + inner_len
    }
}

impl MlsEncode for Credential {
    fn mls_encode(&self, writer: &mut Vec<u8>) -> Result<(), aws_mls_codec::Error> {
        self.credential_type().mls_encode(writer)?;

        match self {
            Credential::Basic(c) => c.mls_encode(writer),
            #[cfg(feature = "x509")]
            Credential::X509(c) => c.mls_encode(writer),
            Credential::Custom(c) => aws_mls_codec::byte_vec::mls_encode(&c.data, writer),
        }
    }
}

impl MlsDecode for Credential {
    fn mls_decode(reader: &mut &[u8]) -> Result<Self, aws_mls_codec::Error> {
        let credential_type = CredentialType::mls_decode(reader)?;

        Ok(match credential_type {
            CredentialType::BASIC => Credential::Basic(BasicCredential::mls_decode(reader)?),
            #[cfg(feature = "x509")]
            CredentialType::X509 => Credential::X509(CertificateChain::mls_decode(reader)?),
            custom => Credential::Custom(CustomCredential {
                credential_type: custom,
                data: aws_mls_codec::byte_vec::mls_decode(reader)?,
            }),
        })
    }
}

/// Trait that provides a conversion between an underlying credential type and
/// the [`Credential`] enum.
pub trait MlsCredential: Sized {
    /// Conversion error type.
    type Error;

    /// Credential type represented by this type.
    fn credential_type() -> CredentialType;

    /// Function to convert this type into a [`Credential`] enum.
    fn into_credential(self) -> Result<Credential, Self::Error>;
}
