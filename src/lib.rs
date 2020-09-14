#[macro_use]
extern crate futures;
#[macro_use]
extern crate serde;

use rand::{thread_rng, Rng};
use schnorrkel::keys::PublicKey as SchnorrkelPubKey;
use schnorrkel::sign::Signature as SchnorrkelSignature;
use serde::de::Error as SerdeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::result::Result as StdResult;

pub mod adapters;
pub mod db;
pub mod identity;

type Result<T> = StdResult<T, failure::Error>;

#[derive(Eq, PartialEq)]
pub struct PubKey(SchnorrkelPubKey);
pub struct Signature(SchnorrkelSignature);
#[derive(Eq, PartialEq, Serialize, Deserialize)]
pub struct Address(String);

#[derive(Eq, PartialEq, Serialize, Deserialize)]
pub enum AddressType {
    Email,
    Web,
    Twitter,
    Riot,
}

#[derive(Serialize, Deserialize)]
pub struct Challenge(String);

impl Challenge {
    fn gen_random() -> Challenge {
        let random: [u8; 16] = thread_rng().gen();
        Challenge(hex::encode(random))
    }
}

impl Serialize for PubKey {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(self.0.to_bytes()))
    }
}

impl<'de> Deserialize<'de> for PubKey {
    fn deserialize<D>(deserializer: D) -> StdResult<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = <String as Deserialize>::deserialize(deserializer)?;
        Ok(PubKey(
            SchnorrkelPubKey::from_bytes(
                &hex::decode(hex_str)
                    .map_err(|_| SerdeError::custom("failed to decode public key from hex"))?,
            )
            .map_err(|_| SerdeError::custom("failed creating public key from bytes"))?,
        ))
    }
}
