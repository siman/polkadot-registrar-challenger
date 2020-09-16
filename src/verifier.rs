use crate::identity::AccountContext;
use crate::{Challenge, Signature};
use schnorrkel::sign::Signature as SchnorrkelSignature;

#[derive(Debug, Fail)]
pub enum VerifierError {
    #[fail(display = "This is not a valid signature output.")]
    InvalidSignature,
    #[fail(
        display = "The signature is INVALID. Please sign the challenge with the key which belongs to the on-chain identity address."
    )]
    SignatureNok,
}

pub struct Verifier {
    context: AccountContext,
    challenge: Challenge,
}

impl Verifier {
    pub fn new(context: AccountContext, challenge: Challenge) -> Self {
        Verifier {
            context: context,
            challenge: challenge,
        }
    }
    pub fn verify(&self, response: &str) -> Result<String, VerifierError> {
        let sig = Signature(
            SchnorrkelSignature::from_bytes(
                &hex::decode(response).map_err(|_| VerifierError::InvalidSignature)?,
            )
            .map_err(|_| VerifierError::InvalidSignature)?,
        );

        if self.challenge.verify_challenge(&self.context.pub_key, &sig) {
            Ok("The signature is VALID. This account is confirmed.".to_string())
        } else {
            Err(VerifierError::SignatureNok)
        }
    }
}