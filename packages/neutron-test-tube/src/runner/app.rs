use std::ffi::CString;
use std::fmt::Debug;

use crate::account::{FeeSetting, SigningAccount};
use crate::bindings::{
    AccountNumber, AccountSequence, FinalizeBlock, GetBlockHeight, GetBlockTime, GetParamSet,
    GetValidatorAddress, GetValidatorPrivateKey, IncreaseTime, InitAccount, InitTestEnv, Query,
    SetParamSet, SetSlinkyPrices, Simulate,
};
use crate::runner::error::{DecodeError, EncodeError, RunnerError};
use crate::runner::result::RawResult;
use crate::runner::result::{RunnerExecuteResult, RunnerResult};
use crate::runner::Runner;
use crate::{redefine_as_go_string, Account};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use cosmrs::crypto::secp256k1::SigningKey;
use cosmrs::proto::tendermint::v0_38::abci::ResponseFinalizeBlock;
use cosmrs::tx;
use cosmrs::tx::{Fee, SignerInfo};
use cosmrs::Any;
use cosmwasm_std::{Coin, Timestamp};
use prost::Message;
use serde::Serialize;

const FEE_DENOM: &str = "untrn";
const NEUTRON_ADDRESS_PREFIX: &str = "neutron";
const CHAIN_ID: &str = "neutron-666";
const DEFAULT_GAS_ADJUSTMENT: f64 = 1.2;
pub const NEUTRON_MIN_GAS_PRICE: u128 = 2_500;

#[derive(Debug, PartialEq)]
pub struct BaseApp {
    id: u64,
    fee_denom: String,
    chain_id: String,
    address_prefix: String,
    default_gas_adjustment: f64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SlinkyPrices {
    pub base: String,
    pub quote: String,
    pub price: u128,
}

impl BaseApp {
    pub fn new(
        fee_denom: &str,
        chain_id: &str,
        address_prefix: &str,
        default_gas_adjustment: f64,
    ) -> Self {
        let id = unsafe { InitTestEnv() };
        BaseApp {
            id,
            fee_denom: fee_denom.to_string(),
            chain_id: chain_id.to_string(),
            address_prefix: address_prefix.to_string(),
            default_gas_adjustment,
        }
    }

    /// Increase the time of the blockchain by the given number of seconds.
    pub fn increase_time(&self, seconds: u64) {
        unsafe {
            IncreaseTime(self.id, seconds.try_into().unwrap());
        }
    }

    /// Sets prices in slinky
    pub fn set_slinky_prices(&self, prices: &[SlinkyPrices]) {
        let prices_json = serde_json::to_string(&prices)
            .map_err(EncodeError::JsonEncodeError)
            .unwrap();
        redefine_as_go_string!(prices_json);

        unsafe {
            SetSlinkyPrices(self.id, prices_json);
        }
    }

    /// Get the first validator address
    pub fn get_first_validator_address(&self) -> RunnerResult<String> {
        let addr = unsafe {
            let addr = GetValidatorAddress(self.id, 0);
            CString::from_raw(addr)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        Ok(addr)
    }

    /// Get the first validator private key
    pub fn get_first_validator_private_key(&self) -> RunnerResult<String> {
        let pkey = unsafe {
            let pkey = GetValidatorPrivateKey(self.id, 0);
            CString::from_raw(pkey)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        Ok(pkey)
    }

    /// Get the first validator signing account
    pub fn get_first_validator_signing_account(
        &self,
        denom: String,
        gas_adjustment: f64,
    ) -> RunnerResult<SigningAccount> {
        let pkey = unsafe {
            let pkey = GetValidatorPrivateKey(self.id, 0);
            CString::from_raw(pkey)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        let secp256k1_priv = BASE64_STANDARD
            .decode(pkey)
            .map_err(DecodeError::Base64DecodeError)?;

        let signing_key = SigningKey::from_slice(&secp256k1_priv).unwrap();

        let validator = SigningAccount::new(
            self.address_prefix.to_string(),
            signing_key,
            FeeSetting::Auto {
                gas_price: Coin::new(NEUTRON_MIN_GAS_PRICE, denom),
                gas_adjustment,
            },
        );

        Ok(validator)
    }

    /// Get the current block time
    pub fn get_block_time_nanos(&self) -> i64 {
        unsafe { GetBlockTime(self.id) }
    }

    /// Get the current block height
    pub fn get_block_height(&self) -> i64 {
        unsafe { GetBlockHeight(self.id) }
    }
    /// Initialize account with initial balance of any coins.
    /// This function mints new coins and send to newly created account
    pub fn init_account(&self, coins: &[Coin]) -> RunnerResult<SigningAccount> {
        let mut coins = coins.to_vec();

        // invalid coins if denom are unsorted
        coins.sort_by(|a, b| a.denom.cmp(&b.denom));

        let coins_json = serde_json::to_string(&coins).map_err(EncodeError::JsonEncodeError)?;
        redefine_as_go_string!(coins_json);

        let empty_tx = "".to_string();
        redefine_as_go_string!(empty_tx);

        let base64_priv = unsafe {
            let addr = InitAccount(self.id, coins_json);
            FinalizeBlock(self.id, empty_tx);
            CString::from_raw(addr)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        let secp256k1_priv = BASE64_STANDARD
            .decode(base64_priv)
            .map_err(DecodeError::Base64DecodeError)?;

        let signing_key = SigningKey::from_slice(&secp256k1_priv).map_err(|e| {
            let msg = e.to_string();
            DecodeError::SigningKeyDecodeError { msg }
        })?;

        Ok(SigningAccount::new(
            self.address_prefix.clone(),
            signing_key,
            FeeSetting::Auto {
                gas_price: Coin::new(NEUTRON_MIN_GAS_PRICE, self.fee_denom.clone()),
                gas_adjustment: self.default_gas_adjustment,
            },
        ))
    }

    /// Convenience function to create multiple accounts with the same
    /// Initial coins balance
    pub fn init_accounts(&self, coins: &[Coin], count: u64) -> RunnerResult<Vec<SigningAccount>> {
        (0..count).map(|_| self.init_account(coins)).collect()
    }

    fn create_signed_tx<I>(
        &self,
        msgs: I,
        signer: &SigningAccount,
        fee: Fee,
    ) -> RunnerResult<Vec<u8>>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        let tx_body = tx::Body::new(msgs, "", 0u32);
        let addr = signer.address();

        redefine_as_go_string!(addr);

        let seq = unsafe { AccountSequence(self.id, addr) };

        let account_number = unsafe { AccountNumber(self.id, addr) };

        let signer_info = SignerInfo::single_direct(Some(signer.public_key()), seq);
        let auth_info = signer_info.auth_info(fee);
        let sign_doc = tx::SignDoc::new(
            &tx_body,
            &auth_info,
            &(self
                .chain_id
                .parse()
                .expect("parse const str of chain id should never fail")),
            account_number,
        )
        .map_err(|e| match e.downcast::<prost::EncodeError>() {
            Ok(encode_err) => EncodeError::ProtoEncodeError(encode_err),
            Err(e) => panic!("expect `prost::EncodeError` but got {:?}", e),
        })?;

        let tx_raw = sign_doc.sign(signer.signing_key()).unwrap();

        tx_raw
            .to_bytes()
            .map_err(|e| match e.downcast::<prost::EncodeError>() {
                Ok(encode_err) => EncodeError::ProtoEncodeError(encode_err),
                Err(e) => panic!("expect `prost::EncodeError` but got {:?}", e),
            })
            .map_err(RunnerError::EncodeError)
    }

    pub fn simulate_tx<I>(
        &self,
        msgs: I,
        signer: &SigningAccount,
    ) -> RunnerResult<cosmrs::proto::cosmos::base::abci::v1beta1::GasInfo>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        let zero_fee = Fee::from_amount_and_gas(
            cosmrs::Coin {
                denom: self.fee_denom.parse().unwrap(),
                amount: NEUTRON_MIN_GAS_PRICE,
            },
            0u64,
        );

        let tx = self.create_signed_tx(msgs, signer, zero_fee)?;
        let base64_tx_bytes = BASE64_STANDARD.encode(tx);

        redefine_as_go_string!(base64_tx_bytes);

        unsafe {
            let res = Simulate(self.id, base64_tx_bytes);
            let res = RawResult::from_non_null_ptr(res).into_result()?;

            cosmrs::proto::cosmos::base::abci::v1beta1::GasInfo::decode(res.as_slice())
                .map_err(DecodeError::ProtoDecodeError)
                .map_err(RunnerError::DecodeError)
        }
    }
    fn estimate_fee<I>(&self, msgs: I, signer: &SigningAccount) -> RunnerResult<Fee>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        let res = match &signer.fee_setting() {
            FeeSetting::Auto {
                gas_price,
                gas_adjustment,
            } => {
                let gas_info = self.simulate_tx(msgs, signer)?;
                let gas_limit = ((gas_info.gas_used as f64) * (gas_adjustment)).ceil() as u64;

                let amount = cosmrs::Coin {
                    denom: self.fee_denom.parse().unwrap(),
                    amount: (((gas_limit as f64)
                        * (gas_price.amount.to_string().parse::<u64>()? as f64))
                        .ceil() as u64)
                        .into(),
                };
                Ok(Fee::from_amount_and_gas(amount, gas_limit))
            }
            FeeSetting::Custom { .. } => {
                panic!("estimate fee is a private function and should never be called when fee_setting is Custom");
            }
        };

        res
    }

    /// Set parameter set for a given subspace.
    pub fn set_param_set(&self, subspace: &str, pset: impl Into<Any>) -> RunnerResult<()> {
        unsafe {
            let pset = Message::encode_to_vec(&pset.into());
            let pset = BASE64_STANDARD.encode(pset);
            redefine_as_go_string!(pset);
            redefine_as_go_string!(subspace);
            let res = SetParamSet(self.id, subspace, pset);

            // Just move one block forward
            IncreaseTime(self.id, 1u64.try_into().unwrap());

            // returns empty bytes if success
            RawResult::from_non_null_ptr(res).into_result()?;
            Ok(())
        }
    }

    /// Get parameter set for a given subspace.
    pub fn get_param_set<P: Message + Default>(
        &self,
        subspace: &str,
        type_url: &str,
    ) -> RunnerResult<P> {
        unsafe {
            redefine_as_go_string!(subspace);
            redefine_as_go_string!(type_url);
            let pset = GetParamSet(self.id, subspace, type_url);
            let pset = RawResult::from_non_null_ptr(pset).into_result()?;
            let pset = P::decode(pset.as_slice()).map_err(DecodeError::ProtoDecodeError)?;
            Ok(pset)
        }
    }
}

impl<'a> Runner<'a> for BaseApp {
    fn execute_multiple<M, R>(
        &self,
        msgs: &[(M, &str)],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        M: ::prost::Message,
        R: ::prost::Message + Default,
    {
        let msgs = msgs
            .iter()
            .map(|(msg, type_url)| {
                let mut buf = Vec::new();
                M::encode(msg, &mut buf).map_err(EncodeError::ProtoEncodeError)?;

                Ok(cosmrs::Any {
                    type_url: type_url.to_string(),
                    value: buf,
                })
            })
            .collect::<Result<Vec<cosmrs::Any>, RunnerError>>()?;

        self.execute_multiple_raw(msgs, signer)
    }

    fn execute_multiple_raw<R>(
        &self,
        msgs: Vec<cosmrs::Any>,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        R: ::prost::Message + Default,
    {
        unsafe {
            let fee = match &signer.fee_setting() {
                FeeSetting::Auto { .. } => self.estimate_fee(msgs.clone(), signer)?,
                FeeSetting::Custom { amount, gas_limit } => Fee::from_amount_and_gas(
                    cosmrs::Coin {
                        denom: amount.denom.parse().unwrap(),
                        amount: amount.amount.to_string().parse().unwrap(),
                    },
                    *gas_limit,
                ),
            };

            let tx = self.create_signed_tx(msgs.clone(), signer, fee)?;
            let base64_tx_bytes = BASE64_STANDARD.encode(tx);

            redefine_as_go_string!(base64_tx_bytes);

            let res = FinalizeBlock(self.id, base64_tx_bytes);
            let res = RawResult::from_non_null_ptr(res).into_result()?;

            let res = ResponseFinalizeBlock::decode(res.as_slice())
                .unwrap()
                .try_into();

            // println!("{:#?}", res);

            res
        }
    }

    fn query<Q, R>(&self, path: &str, q: &Q) -> RunnerResult<R>
    where
        Q: ::prost::Message,
        R: ::prost::Message + Default,
    {
        let mut buf = Vec::new();

        Q::encode(q, &mut buf).map_err(EncodeError::ProtoEncodeError)?;

        let base64_query_msg_bytes = BASE64_STANDARD.encode(buf);

        redefine_as_go_string!(path);
        redefine_as_go_string!(base64_query_msg_bytes);

        unsafe {
            let res = Query(self.id, path, base64_query_msg_bytes);
            let res = RawResult::from_non_null_ptr(res).into_result()?;
            R::decode(res.as_slice())
                .map_err(DecodeError::ProtoDecodeError)
                .map_err(RunnerError::DecodeError)
        }
    }

    fn execute<M, R>(
        &self,
        msg: M,
        type_url: &str,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        M: prost::Message,
        R: prost::Message + Default,
    {
        self.execute_multiple(&[(msg, type_url)], signer)
    }

    fn execute_cosmos_msgs<S>(
        &self,
        msgs: &[cosmwasm_std::CosmosMsg],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<S>
    where
        S: prost::Message + Default,
    {
        let msgs = msgs
            .iter()
            .map(|msg| match msg {
                cosmwasm_std::CosmosMsg::Bank(msg) => crate::utils::bank_msg_to_any(msg, signer),
                #[allow(deprecated)]
                cosmwasm_std::CosmosMsg::Stargate { type_url, value } => Ok(cosmrs::Any {
                    type_url: type_url.clone(),
                    value: value.to_vec(),
                }),
                cosmwasm_std::CosmosMsg::Any(msg) => Ok(cosmrs::Any {
                    type_url: msg.type_url.clone(),
                    value: msg.value.to_vec(),
                }),
                cosmwasm_std::CosmosMsg::Wasm(msg) => crate::utils::wasm_msg_to_any(msg, signer),
                _ => std::todo!("unsupported cosmos msg variant"),
            })
            .collect::<Result<Vec<_>, RunnerError>>()?;

        self.execute_multiple_raw(msgs, signer)
    }
}

#[derive(Debug, PartialEq)]
pub struct NeutronTestApp {
    inner: BaseApp,
}

impl Default for NeutronTestApp {
    fn default() -> Self {
        NeutronTestApp::new()
    }
}

impl NeutronTestApp {
    pub fn new() -> Self {
        Self {
            inner: BaseApp::new(
                FEE_DENOM,
                CHAIN_ID,
                NEUTRON_ADDRESS_PREFIX,
                DEFAULT_GAS_ADJUSTMENT,
            ),
        }
    }

    /// Get the current block time as a timestamp
    pub fn get_block_timestamp(&self) -> Timestamp {
        Timestamp::from_nanos(self.inner.get_block_time_nanos().try_into().unwrap())
    }

    /// Get the current block time in nanoseconds
    pub fn get_block_time_nanos(&self) -> i64 {
        self.inner.get_block_time_nanos()
    }

    /// Get the current block time in seconds
    pub fn get_block_time_seconds(&self) -> i64 {
        self.inner.get_block_time_nanos() / 1_000_000_000i64
    }

    /// Get the current block height
    pub fn get_block_height(&self) -> i64 {
        self.inner.get_block_height()
    }

    /// Get the first validator address
    pub fn get_first_validator_address(&self) -> RunnerResult<String> {
        self.inner.get_first_validator_address()
    }

    /// Get the first validator private key
    pub fn get_first_validator_private_key(&self) -> RunnerResult<String> {
        self.inner.get_first_validator_private_key()
    }

    /// Get the first validator signing account
    pub fn get_first_validator_signing_account(
        &self,
        denom: String,
        gas_adjustment: f64,
    ) -> RunnerResult<SigningAccount> {
        self.inner
            .get_first_validator_signing_account(denom, gas_adjustment)
    }

    /// Increase the time of the blockchain by the given number of seconds.
    pub fn increase_time(&self, seconds: u64) {
        self.inner.increase_time(seconds)
    }
    /// Set the slinky prices
    pub fn set_slinky_prices(&self, prices: &[SlinkyPrices]) {
        self.inner.set_slinky_prices(prices)
    }

    /// Initialize account with initial balance of any coins.
    /// This function mints new coins and send to newly created account
    pub fn init_account(&self, coins: &[Coin]) -> RunnerResult<SigningAccount> {
        self.inner.init_account(coins)
    }
    /// Convinience function to create multiple accounts with the same
    /// Initial coins balance
    pub fn init_accounts(&self, coins: &[Coin], count: u64) -> RunnerResult<Vec<SigningAccount>> {
        self.inner.init_accounts(coins, count)
    }

    /// Simulate transaction execution and return gas info
    pub fn simulate_tx<I>(
        &self,
        msgs: I,
        signer: &SigningAccount,
    ) -> RunnerResult<cosmrs::proto::cosmos::base::abci::v1beta1::GasInfo>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        self.inner.simulate_tx(msgs, signer)
    }

    // /// Set parameter set for a given subspace.
    // pub fn set_param_set(&self, subspace: &str, pset: impl Into<Any>) -> RunnerResult<()> {
    //     self.inner.set_param_set(subspace, pset)
    // }

    /// Get parameter set for a given subspace.
    pub fn get_param_set<P: Message + Default>(
        &self,
        subspace: &str,
        type_url: &str,
    ) -> RunnerResult<P> {
        self.inner.get_param_set(subspace, type_url)
    }
}

impl<'a> Runner<'a> for NeutronTestApp {
    fn execute_multiple<M, R>(
        &self,
        msgs: &[(M, &str)],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        M: ::prost::Message,
        R: ::prost::Message + Default,
    {
        self.inner.execute_multiple(msgs, signer)
    }

    fn query<Q, R>(&self, path: &str, q: &Q) -> RunnerResult<R>
    where
        Q: ::prost::Message,
        R: ::prost::Message + Default,
    {
        self.inner.query(path, q)
    }

    fn execute_multiple_raw<R>(
        &self,
        msgs: Vec<cosmrs::Any>,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        R: prost::Message + Default,
    {
        self.inner.execute_multiple_raw(msgs, signer)
    }
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{coins, Coin};
    use neutron_std::types::osmosis::tokenfactory::v1beta1::{
        MsgCreateDenom, MsgCreateDenomResponse, QueryParamsRequest, QueryParamsResponse,
    };

    use crate::module::Wasm;
    use crate::runner::app::NeutronTestApp;

    use crate::account::Account;
    use crate::module::Module;
    use crate::runner::*;
    use crate::ExecuteResponse;

    #[test]
    fn test_init_account() {
        let app = NeutronTestApp::default();

        // Just check it doesn't panic
        app.init_account(&coins(100_000_000_000, "untrn")).unwrap();
    }

    #[test]
    fn test_init_accounts() {
        let app = NeutronTestApp::default();

        let accounts = app
            .init_accounts(&coins(100_000_000_000, "untrn"), 3)
            .unwrap();

        assert!(accounts.get(0).is_some());
        assert!(accounts.get(1).is_some());
        assert!(accounts.get(2).is_some());
        assert!(accounts.get(3).is_none());
    }

    #[test]
    fn test_get_and_set_block_timestamp() {
        let app = NeutronTestApp::default();

        let block_time_nanos = app.get_block_time_nanos();
        let block_time_seconds = app.get_block_time_seconds();

        app.increase_time(10u64);

        assert_eq!(
            app.get_block_time_nanos(),
            block_time_nanos + 10_000_000_000
        );
        assert_eq!(app.get_block_time_seconds(), block_time_seconds + 10);
    }

    #[test]
    fn test_get_block_height() {
        let app = NeutronTestApp::default();

        assert_eq!(app.get_block_height(), 1i64);

        app.increase_time(10u64);

        assert_eq!(app.get_block_height(), 2i64);
    }

    #[test]
    fn test_execute() {
        let app = NeutronTestApp::default();

        assert_eq!(app.get_block_height(), 1i64);

        let acc = app
            .init_account(&coins(100_000_000_000_000_000_000u128, "untrn")) // 100 inj
            .unwrap();
        let addr = acc.address();

        let msg = MsgCreateDenom {
            sender: acc.address(),
            subdenom: "newdenom".to_string(),
        };

        let res: ExecuteResponse<MsgCreateDenomResponse> = app
            .execute(msg, "/osmosis.tokenfactory.v1beta1.MsgCreateDenom", &acc)
            .unwrap();

        let create_denom_attrs = &res.data.new_token_denom;
        assert_eq!(
            create_denom_attrs,
            &format!("factory/{}/{}", &addr, "newdenom")
        );

        // execute on more time to excercise account sequence
        let msg = MsgCreateDenom {
            sender: acc.address(),
            subdenom: "newerdenom".to_string(),
        };

        let res: ExecuteResponse<MsgCreateDenomResponse> = app
            .execute(msg, "/osmosis.tokenfactory.v1beta1.MsgCreateDenom", &acc)
            .unwrap();

        let create_denom_attrs = &res.data.new_token_denom;
        assert_eq!(
            create_denom_attrs,
            &format!("factory/{}/{}", &addr, "newerdenom")
        );

        // execute on more time to excercise account sequence
        let msg = MsgCreateDenom {
            sender: acc.address(),
            subdenom: "multidenom_1".to_string(),
        };

        let msg_2 = MsgCreateDenom {
            sender: acc.address(),
            subdenom: "multidenom_2".to_string(),
        };

        assert_eq!(app.get_block_height(), 4i64);

        let _res: ExecuteResponse<MsgCreateDenomResponse> = app
            .execute_multiple(
                &[
                    (msg, "/osmosis.tokenfactory.v1beta1.MsgCreateDenom"),
                    (msg_2, "/osmosis.tokenfactory.v1beta1.MsgCreateDenom"),
                ],
                &acc,
            )
            .unwrap();

        assert_eq!(app.get_block_height(), 5i64);
    }

    #[test]
    fn test_query() {
        let app = NeutronTestApp::default();

        let denom_creation_fee = app
            .query::<QueryParamsRequest, QueryParamsResponse>(
                "/osmosis.tokenfactory.v1beta1.Query/Params",
                &QueryParamsRequest {},
            )
            .unwrap()
            .params
            .unwrap()
            .denom_creation_fee;

        // fee is no longer set
        assert_eq!(denom_creation_fee, [])
    }

    #[test]
    fn test_wasm_execute_and_query() {
        use cw1_whitelist::msg::*;

        let app = NeutronTestApp::default();
        let accs = app
            .init_accounts(
                &[
                    Coin::new(1_000_000_000_000u128, "uatom"),
                    Coin::new(1_000_000_000_000u128, "untrn"),
                ],
                2,
            )
            .unwrap();
        let admin = &accs[0];
        let new_admin = &accs[1];

        let wasm = Wasm::new(&app);
        let wasm_byte_code = std::fs::read("./test_artifacts/cw1_whitelist.wasm").unwrap();
        let code_id = wasm
            .store_code(&wasm_byte_code, None, admin)
            .unwrap()
            .data
            .code_id;
        assert_eq!(code_id, 1);

        // initialize admins and check if the state is correct
        let init_admins = vec![admin.address()];
        let contract_addr = wasm
            .instantiate(
                code_id,
                &InstantiateMsg {
                    admins: init_admins.clone(),
                    mutable: true,
                },
                Some(&admin.address()),
                Some("Test label"),
                &[],
                admin,
            )
            .unwrap()
            .data
            .address;
        let admin_list = wasm
            .query::<QueryMsg, AdminListResponse>(&contract_addr, &QueryMsg::AdminList {})
            .unwrap();
        assert_eq!(admin_list.admins, init_admins);
        assert!(admin_list.mutable);

        // update admin and check again
        let new_admins = vec![new_admin.address()];
        wasm.execute::<ExecuteMsg>(
            &contract_addr,
            &ExecuteMsg::UpdateAdmins {
                admins: new_admins.clone(),
            },
            &[],
            admin,
        )
        .unwrap();

        let admin_list = wasm
            .query::<QueryMsg, AdminListResponse>(&contract_addr, &QueryMsg::AdminList {})
            .unwrap();

        assert_eq!(admin_list.admins, new_admins);
        assert!(admin_list.mutable);
    }
}
