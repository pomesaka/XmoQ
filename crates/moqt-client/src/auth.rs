//! `--auth-token-hex`: the AUTHORIZATION TOKEN (§10.2.2, §10.3.1.4) of this
//! client's connection. It is given once and goes into the SETUP and into
//! every request the client sends that §10.2.2 lets it be in: PUBLISH,
//! `PUBLISH_NAMESPACE`, SUBSCRIBE and FETCH, a Joining FETCH among them. (The
//! client sends none of the others of that list: `REQUEST_UPDATE`,
//! `SUBSCRIBE_NAMESPACE`, `SUBSCRIBE_TRACKS`, `TRACK_STATUS`.)

use anyhow::{Context, Result};
use xmoq_moqt::kvp::KeyValuePair;
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

    /// The Setup Option with the token (0x03), where the flag gave one.
    pub fn setup_options(&self) -> Result<Vec<KeyValuePair>> {
        self.0
            .iter()
            .map(|token| Ok(KeyValuePair::bytes(0x03, token.clone())?))
            .collect()
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
