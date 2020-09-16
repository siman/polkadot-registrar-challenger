use super::{Account, NetAccount, AccountType, Challenge, Fatal, PubKey, Result, NetworkAddress};
use crate::db::{Database, ScopedDatabase};
use crossbeam::channel::{unbounded, Receiver, Sender};
use failure::err_msg;
use matrix_sdk::identifiers::RoomId;
use std::collections::HashMap;
use std::convert::TryInto;
use tokio::time::{self, Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnChainIdentity {
    pub network_address: NetworkAddress,
    pub display_name: Option<String>,
    pub legal_name: Option<String>,
    pub email: Option<AccountState>,
    pub web: Option<AccountState>,
    pub twitter: Option<AccountState>,
    pub matrix: Option<AccountState>,
}

impl OnChainIdentity {
    pub fn address(&self) -> &NetAccount {
        &self.network_address.address
    }
    pub fn pub_key(&self) -> &PubKey {
        &self.network_address.pub_key
    }
    pub fn from_json(val: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(&val)?)
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountState {
    account: Account,
    account_ty: AccountType,
    account_validity: AccountValidity,
    pub challenge: Challenge,
    confirmed: bool,
}

impl AccountState {
    pub fn new(account: Account, account_ty: AccountType) -> Self {
        AccountState {
            account: account,
            account_ty: account_ty,
            account_validity: AccountValidity::Unknown,
            challenge: Challenge::gen_random(),
            confirmed: false,
        }
    }
}

#[derive(Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
enum AccountValidity {
    #[serde(rename = "unknown")]
    Unknown,
    #[serde(rename = "valid")]
    Valid,
    #[serde(rename = "invalid")]
    Invalid,
}

pub enum CommsMessage {
    NewOnChainIdentity(OnChainIdentity),
    Inform {
        network_address: NetworkAddress,
        challenge: Challenge,
        room_id: Option<RoomId>,
    },
    ValidAccount {
        network_address: NetworkAddress,
    },
    InvalidAccount {
        network_address: NetworkAddress,
    },
    TrackRoomId {
        pub_key: PubKey,
        room_id: RoomId,
    },
    RequestAccountState {
        account: Account,
        account_ty: AccountType,
    },
    InvalidRequest,
}

#[derive(Debug, Clone)]
pub struct CommsMain {
    sender: Sender<CommsMessage>,
}

impl CommsMain {
    fn inform(
        &self,
        network_address: NetworkAddress,
        challenge: Challenge,
        room_id: Option<RoomId>,
    ) {
        self.sender
            .send(CommsMessage::Inform {
                network_address: network_address,
                challenge: challenge,
                room_id,
            })
            .fatal();
    }
    fn invalid_request(&self) {
        self.sender.send(CommsMessage::InvalidRequest).fatal();
    }
}

#[derive(Debug, Clone)]
pub struct CommsVerifier {
    tx: Sender<CommsMessage>,
    recv: Receiver<CommsMessage>,
    address_ty: AccountType,
}

impl CommsVerifier {
    pub async fn recv(&self) -> CommsMessage {
        let mut interval = time::interval(Duration::from_millis(50));

        loop {
            if let Ok(msg) = self.recv.try_recv() {
                return msg;
            } else {
                interval.tick().await;
            }
        }
    }
    pub fn try_recv(&self) -> Option<CommsMessage> {
        self.recv.try_recv().ok()
    }
    /// Receive a `Inform` message. This is only used by the Matrix client as
    /// any other message type will panic.
    // TODO: Just use `recv` and match directly. Remove this method
    pub async fn recv_inform(&self) -> (NetworkAddress, Challenge, Option<RoomId>) {
        if let CommsMessage::Inform {
            network_address,
            challenge,
            room_id,
        } = self.recv().await
        {
            (network_address, challenge, room_id)
        } else {
            panic!("received invalid message type on Matrix client");
        }
    }
    pub fn request_account_state(&self, account: Account, account_ty: AccountType) {
        self.tx
            .send(CommsMessage::RequestAccountState {
                account: account,
                account_ty: account_ty,
            })
            .fatal();
    }
    pub fn new_on_chain_identity(&self, ident: OnChainIdentity) {
        self.tx
            .send(CommsMessage::NewOnChainIdentity(ident))
            .fatal();
    }
    pub fn valid_feedback(&self, network_address: NetworkAddress) {
        self.tx
            .send(CommsMessage::ValidAccount {
                network_address: network_address
            })
            .fatal();
    }
    pub fn invalid_feedback(&self, network_address: NetworkAddress) {
        self.tx
            .send(CommsMessage::InvalidAccount {
                network_address: network_address,
            })
            .fatal();
    }
    pub fn track_room_id(&self, pub_key: PubKey, room_id: RoomId) {
        self.tx
            .send(CommsMessage::TrackRoomId {
                pub_key: pub_key,
                room_id: room_id,
            })
            .fatal();
    }
}

pub struct IdentityManager {
    idents: Vec<OnChainIdentity>,
    db: Database,
    comms: CommsTable,
}

struct CommsTable {
    to_main: Sender<CommsMessage>,
    listener: Receiver<CommsMessage>,
    pairs: HashMap<AccountType, CommsMain>,
}

impl IdentityManager {
    pub fn new(db: Database) -> Result<Self> {
        let mut idents = vec![];

        // Read pending on-chain identities from storage. Ideally, there are none.
        let db_idents = db.scope("pending_identities");
        for (_, value) in db_idents.all()? {
            idents.push(OnChainIdentity::from_json(&*value)?);
        }

        let (tx1, recv1) = unbounded();

        Ok(IdentityManager {
            idents: idents,
            db: db,
            comms: CommsTable {
                to_main: tx1.clone(),
                listener: recv1,
                pairs: HashMap::new(),
            },
        })
    }
    pub fn register_comms(&mut self, account_ty: AccountType) -> CommsVerifier {
        let (tx, recv) = unbounded();

        self.comms.pairs.insert(
            account_ty.clone(),
            CommsMain {
                sender: tx,
            },
        );

        CommsVerifier {
            tx: self.comms.to_main.clone(),
            recv: recv,
            address_ty: account_ty,
        }
    }
    pub async fn start(mut self) -> Result<()> {
        use CommsMessage::*;
        let mut interval = time::interval(Duration::from_millis(50));

        loop {
            if let Ok(msg) = self.comms.listener.try_recv() {
                match msg {
                    CommsMessage::NewOnChainIdentity(ident) => {
                        self.register_request(ident)?;
                    }
                    ValidAccount { network_address } => {}
                    InvalidAccount { network_address } => {}
                    TrackRoomId { pub_key, room_id } => {
                        let db_rooms = self.db.scope("matrix_rooms");
                        db_rooms.put(pub_key.0, room_id.as_bytes())?;
                    }
                    RequestAccountState {
                        account,
                        account_ty

                        } => {
                        self.request_account_state(account);
                    }
                    _ => panic!("Received unrecognized message type. Report as a bug"),
                }
            } else {
                interval.tick().await;
            }
        }

        Ok(())
    }
    fn register_request(&mut self, ident: OnChainIdentity) -> Result<()> {
        // TODO: Handle updates

        let db_idents = self.db.scope("pending_identities");
        let db_rooms = self.db.scope("matrix_rooms");

        // Save the pending on-chain identity to disk.
        db_idents.put(ident.pub_key().to_bytes(), ident.to_json()?)?;
        self.idents.push(ident);

        let ident = self
            .idents
            .last()
            // TODO: necessary?
            .ok_or(err_msg("last registered identity not found."))?;

        // Only matrix supported for now.
        ident.matrix.as_ref().map::<(), _>(|state| {
            let room_id = if let Some(bytes) = db_rooms.get(&ident.pub_key().to_bytes()).fatal() {
                Some(std::str::from_utf8(&bytes).fatal().try_into().fatal())
            } else {
                None
            };

            self.comms.pairs.get(&state.account_ty).fatal().inform(
                ident.network_address.clone(),
                state.challenge.clone(),
                room_id,
            );
        });

        Ok(())
    }
    // TODO: handle multiple account_ids
    fn request_account_state(&self, account: Account) {
        // Account state requests are always available, since such a request
        // cannot occur if it isn't.

        // Find the identity based on the corresponding Matrix UserId.
        let ident = self.idents.iter().find(|ident| {
            if let Some(state) = ident.matrix.as_ref() {
                state.account == account
            } else {
                false
            }
        });

        let comms = self.comms.pairs.get(&AccountType::ReservedEmitter).fatal();

        if let Some(ident) = ident {
            // Account state was checked in `find` combinator.
            let state = ident.matrix.as_ref().fatal();
            comms.inform(ident.network_address.clone(), state.challenge.clone(), None);
        } else {
            comms.invalid_request();
        }
    }
}
