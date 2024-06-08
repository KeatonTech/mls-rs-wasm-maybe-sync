// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// Copyright by contributors to this project.
// SPDX-License-Identifier: (Apache-2.0 OR MIT)

pub use crate::{aead::MockAeadType, dh::MockDhType, kdf::MockKdfType, kem::MockKemType};

use maybe_sync::dyn_maybe_send_sync;

#[derive(Debug)]
pub struct TestError {}

impl std::error::Error for TestError {}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestError")
    }
}

impl mls_rs_core::error::IntoAnyError for TestError {
    fn into_dyn_error(self) -> Result<Box<dyn_maybe_send_sync!(std::error::Error)>, Self> {
        Ok(self.into())
    }
}
