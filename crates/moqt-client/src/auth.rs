//! `--auth-token-hex`: an AUTHORIZATION TOKEN (§10.2.2) for the requests
//! this client sends. An experiment: how the relay authorizes is an open
//! question of the project.

use anyhow::{Context, Result};
use xmoq_moqt::params::Parameter;

/// The Token structure of `--auth-token-hex`, as the bytes that go into the
/// parameter: this client makes no token, it carries one.
#[derive(Debug, Default)]
pub struct AuthFlag(Option<Vec<u8>>);

impl AuthFlag {
    /// Reads a flag that takes a value. Whether it was this one.
    pub fn read(&mut self, flag: &str, value: &str) -> Result<bool> {
        if flag != "--auth-token-hex" {
            return Ok(false);
        }
        self.0 = Some(crate::hex::decode(value).context("--auth-token-hex")?);
        Ok(true)
    }

    /// The parameters of a PUBLISH, a `PUBLISH_NAMESPACE`, a SUBSCRIBE or a
    /// FETCH, with the token where the flag gave one.
    pub fn with(&self, mut parameters: Vec<Parameter>) -> Vec<Parameter> {
        if let Some(token) = &self.0 {
            parameters.push(Parameter::AuthorizationToken(token.clone()));
        }
        parameters
    }
}
