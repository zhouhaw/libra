// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    constants, error::Error, layout::Layout, secure_backend::StorageLocation::RemoteStorage,
    SingleBackend,
};
use libra_crypto::ed25519::Ed25519PublicKey;
use libra_global_constants::{ASSOCIATION_KEY, OPERATOR_KEY, OWNER_KEY};
use libra_secure_storage::KVStorage;
use libra_types::{
    account_address,
    account_address::AccountAddress,
    transaction::{Transaction, TransactionPayload},
};
use std::{fs::File, io::Write, path::PathBuf};
use structopt::StructOpt;
use vm_genesis::{OperatorAssignment, OperatorRegistration};

/// Note, it is implicitly expected that the storage supports
/// a namespace but one has not been set.
#[derive(Debug, StructOpt)]
pub struct Genesis {
    #[structopt(flatten)]
    pub backend: SingleBackend,
    #[structopt(long)]
    pub path: Option<PathBuf>,
}

impl Genesis {
    pub fn execute(self) -> Result<Transaction, Error> {
        let layout = self.layout()?;
        let association_key = self.association_key(&layout)?;
        let operator_assignments = self.operator_assignments(&layout)?;
        let operator_registrations = self.operator_registrations(&layout)?;

        let genesis = vm_genesis::encode_genesis_transaction(
            association_key,
            &operator_assignments,
            &operator_registrations,
            Some(libra_types::on_chain_config::VMPublishingOption::open()),
        );

        if let Some(path) = self.path {
            let mut file = File::create(path).map_err(|e| {
                Error::UnexpectedError(format!("Unable to create genesis file: {}", e.to_string()))
            })?;
            let bytes = lcs::to_bytes(&genesis).map_err(|e| {
                Error::UnexpectedError(format!("Unable to serialize genesis: {}", e.to_string()))
            })?;
            file.write_all(&bytes).map_err(|e| {
                Error::UnexpectedError(format!("Unable to write genesis file: {}", e.to_string()))
            })?;
        }

        Ok(genesis)
    }

    /// Retrieves the association key from the remote storage. Note, at this point in time, genesis
    /// only supports a single association key.
    pub fn association_key(&self, layout: &Layout) -> Result<Ed25519PublicKey, Error> {
        let association_config = self.backend.backend.clone();
        let association_config = association_config.set_namespace(layout.association[0].clone());

        let association_storage = association_config.create_storage(RemoteStorage)?;

        let association_key = association_storage
            .get(ASSOCIATION_KEY)
            .map_err(|e| Error::RemoteStorageReadError(ASSOCIATION_KEY, e.to_string()))?;
        association_key
            .value
            .ed25519_public_key()
            .map_err(|e| Error::RemoteStorageReadError(ASSOCIATION_KEY, e.to_string()))
    }

    /// Retrieves a layout from the remote storage.
    pub fn layout(&self) -> Result<Layout, Error> {
        let common_config = self.backend.backend.clone();
        let common_config = common_config.set_namespace(constants::COMMON_NS.into());

        let common_storage = common_config.create_storage(RemoteStorage)?;

        let layout = common_storage
            .get(constants::LAYOUT)
            .and_then(|v| v.value.string())
            .map_err(|e| Error::RemoteStorageReadError(constants::LAYOUT, e.to_string()))?;
        Layout::parse(&layout)
            .map_err(|e| Error::RemoteStorageReadError(constants::LAYOUT, e.to_string()))
    }

    /// Produces a set of OperatorAssignments from the remote storage.
    pub fn operator_assignments(&self, layout: &Layout) -> Result<Vec<OperatorAssignment>, Error> {
        let mut operator_assignments = Vec::new();

        for owner in layout.owners.iter() {
            let owner_config = self.backend.backend.clone();
            let owner_config = owner_config.set_namespace(owner.into());

            let owner_storage = owner_config.create_storage(RemoteStorage)?;

            let owner_key = owner_storage
                .get(OWNER_KEY)
                .map_err(|e| Error::RemoteStorageReadError(OWNER_KEY, e.to_string()))?
                .value
                .ed25519_public_key()
                .map_err(|e| Error::RemoteStorageReadError(OWNER_KEY, e.to_string()))?;

            let operator_name = owner_storage
                .get(constants::VALIDATOR_OPERATOR)
                .map_err(|e| {
                    Error::RemoteStorageReadError(constants::VALIDATOR_OPERATOR, e.to_string())
                })?
                .value
                .string()
                .unwrap();

            let operator_account = self.fetch_operator_account(operator_name)?;
            let set_operator_script =
                transaction_builder::encode_set_validator_operator_script(operator_account);

            operator_assignments.push((owner_key, set_operator_script));
        }

        Ok(operator_assignments)
    }

    /// Retrieves the operator key from the remote storage using the given operator name, and uses
    /// this key to derive an operator account address.
    fn fetch_operator_account(&self, operator_name: String) -> Result<AccountAddress, Error> {
        let operator_config = self.backend.backend.clone();
        let operator_config = operator_config.set_namespace(operator_name);

        let operator_storage = operator_config.create_storage(RemoteStorage)?;

        let operator_key = operator_storage
            .get(OPERATOR_KEY)
            .map_err(|e| Error::RemoteStorageReadError(OPERATOR_KEY, e.to_string()))?
            .value
            .ed25519_public_key()
            .map_err(|e| Error::RemoteStorageReadError(OPERATOR_KEY, e.to_string()))?;
        Ok(account_address::from_public_key(&operator_key))
    }

    /// Produces a set of OperatorRegistrations from the remote storage.
    pub fn operator_registrations(
        &self,
        layout: &Layout,
    ) -> Result<Vec<OperatorRegistration>, Error> {
        let mut registrations = Vec::new();
        for operator in layout.operators.iter() {
            let operator_config = self.backend.backend.clone();
            let operator_config = operator_config.set_namespace(operator.into());

            let operator_storage = operator_config.create_storage(RemoteStorage)?;

            let operator_key = operator_storage
                .get(OPERATOR_KEY)
                .map_err(|e| Error::RemoteStorageReadError(OPERATOR_KEY, e.to_string()))?
                .value
                .ed25519_public_key()
                .map_err(|e| Error::RemoteStorageReadError(OPERATOR_KEY, e.to_string()))?;

            let validator_config_tx = operator_storage
                .get(constants::VALIDATOR_CONFIG)
                .map_err(|e| {
                    Error::RemoteStorageReadError(constants::VALIDATOR_CONFIG, e.to_string())
                })?
                .value;
            let validator_config_tx = validator_config_tx.transaction().unwrap();
            let validator_config_tx = validator_config_tx.as_signed_user_txn().unwrap().payload();
            let validator_config_tx =
                if let TransactionPayload::Script(script) = validator_config_tx {
                    script.clone()
                } else {
                    return Err(Error::UnexpectedError("Found invalid registration".into()));
                };

            registrations.push((operator_key, validator_config_tx));
        }

        Ok(registrations)
    }
}
