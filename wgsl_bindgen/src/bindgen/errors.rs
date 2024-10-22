use miette::Diagnostic;
use thiserror::Error;

use crate::{CreateModuleError, WgslBindgenOptionBuilderError};

/// Enum representing the possible errors that can occur in the `wgsl_bindgen` process.
///
/// This enum is used to represent all the different kinds of errors that can occur
/// when parsing WGSL shaders, generating Rust bindings, or performing other operations
/// in `wgsl_bindgen`.
#[derive(Debug, Error, Diagnostic)]
pub enum WgslBindgenError {
    #[error("All required fields need to be set upfront: {0}")]
    OptionBuilderError(#[from] WgslBindgenOptionBuilderError),

    #[error("Failed to compose modules with file name `{file_name}`\n{msg}")]
    NagaModuleComposeError {
        file_name: String,
        msg: String,
        inner: naga::front::wgsl::ParseError,
    },

    #[error(transparent)]
    NagaValidationError(#[from] naga::WithSpan<naga::valid::ValidationError>),

    #[error(transparent)]
    ModuleCreationError(#[from] CreateModuleError),
}
