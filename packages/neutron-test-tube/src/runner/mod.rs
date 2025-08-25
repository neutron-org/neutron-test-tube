use cosmwasm_std::CosmosMsg;

use crate::account::SigningAccount;
use crate::runner::result::{RunnerExecuteResult, RunnerResult};
use crate::utils::{bank_msg_to_any, wasm_msg_to_any};
use crate::RunnerError;

pub mod app;
pub mod error;
pub mod result;

pub trait Runner<'a> {
    fn execute<M, R>(
        &self,
        msg: M,
        type_url: &str,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        M: ::prost::Message,
        R: ::prost::Message + Default,
    {
        self.execute_multiple(&[(msg, type_url)], signer)
    }

    fn execute_multiple<M, R>(
        &self,
        msgs: &[(M, &str)],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        M: ::prost::Message,
        R: ::prost::Message + Default;

    fn execute_multiple_raw<R>(
        &self,
        msgs: Vec<cosmrs::Any>,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        R: ::prost::Message + Default;

    fn execute_cosmos_msgs<S>(
        &self,
        msgs: &[CosmosMsg],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<S>
    where
        S: ::prost::Message + Default,
    {
        let msgs = msgs
            .iter()
            .map(|msg| match msg {
                CosmosMsg::Bank(msg) => bank_msg_to_any(msg, signer),
                #[allow(deprecated)]
                CosmosMsg::Stargate { type_url, value } => Ok(cosmrs::Any {
                    type_url: type_url.clone(),
                    value: value.to_vec(),
                }),
                CosmosMsg::Any(msg) => Ok(cosmrs::Any {
                    type_url: msg.type_url.clone(),
                    value: msg.value.to_vec(),
                }),
                CosmosMsg::Wasm(msg) => wasm_msg_to_any(msg, signer),
                _ => todo!("unsupported cosmos msg variant"),
            })
            .collect::<Result<Vec<_>, RunnerError>>()?;

        self.execute_multiple_raw(msgs, signer)
    }

    fn query<Q, R>(&self, path: &str, query: &Q) -> RunnerResult<R>
    where
        Q: ::prost::Message,
        R: ::prost::Message + Default;
}

#[cfg(test)]
mod tests {

    use super::app::NeutronTestApp;
    use crate::runner::error::RunnerError::QueryError;
    use crate::runner::result::RawResult;
    use crate::{Account, Bank, Module, Runner, Wasm};
    use base64::Engine;
    use cosmwasm_std::{to_json_binary, BankMsg, Coin, CosmosMsg, Empty, Event, WasmMsg};
    use cw1_whitelist::msg::{ExecuteMsg, InstantiateMsg};
    use std::ffi::CString;

    use neutron_std::types::osmosis::tokenfactory::v1beta1::{
        MsgCreateDenom, MsgCreateDenomResponse,
    };
    use neutron_std::types::{
        cosmos::bank::v1beta1::{MsgSendResponse, QueryBalanceRequest},
        cosmwasm::wasm::v1::{MsgExecuteContractResponse, MsgInstantiateContractResponse},
    };

    #[derive(::prost::Message)]
    struct AdhocRandomQueryRequest {
        #[prost(uint64, tag = "1")]
        id: u64,
    }

    #[derive(::prost::Message)]
    struct AdhocRandomQueryResponse {
        #[prost(string, tag = "1")]
        msg: String,
    }

    #[test]
    fn test_query_error_no_route() {
        let app = NeutronTestApp::default();
        let res = app.query::<AdhocRandomQueryRequest, AdhocRandomQueryResponse>(
            "/neutron.random.v1beta1.Query/AdhocRandom",
            &AdhocRandomQueryRequest { id: 1 },
        );

        let err = res.unwrap_err();
        assert_eq!(
            err,
            QueryError {
                msg: "No route found for `/neutron.random.v1beta1.Query/AdhocRandom`".to_string()
            }
        );
    }

    #[test]
    fn test_raw_result_ptr_with_0_bytes_in_content_should_not_error() {
        let base64_string =
            base64::engine::general_purpose::STANDARD.encode([vec![0u8], vec![0u8]].concat());
        let res = unsafe { RawResult::from_ptr(CString::new(base64_string).unwrap().into_raw()) }
            .unwrap()
            .into_result()
            .unwrap();

        assert_eq!(res, vec![0u8]);
    }

    #[test]
    fn test_execute_cosmos_msgs() {
        let app = NeutronTestApp::new();
        let signer = app
            .init_account(&[Coin::new(10_000_000_000u128, "untrn")])
            .unwrap();

        let bank = Bank::new(&app);

        // BankMsg::Send
        let to = app.init_account(&[]).unwrap();
        let coin = Coin::new(100u128, "untrn");
        let send_msg = CosmosMsg::Bank(BankMsg::Send {
            to_address: to.address(),
            amount: vec![coin],
        });
        app.execute_cosmos_msgs::<MsgSendResponse>(&[send_msg], &signer)
            .unwrap();
        let balance = bank
            .query_balance(&QueryBalanceRequest {
                address: to.address(),
                denom: "untrn".to_string(),
            })
            .unwrap()
            .balance;
        assert_eq!(balance.clone().unwrap().amount, "100".to_string());
        assert_eq!(balance.unwrap().denom, "untrn".to_string());

        // WasmMsg, first upload a contract
        let wasm = Wasm::new(&app);
        let wasm_byte_code = std::fs::read("./test_artifacts/cw1_whitelist.wasm").unwrap();
        let code_id = wasm
            .store_code(&wasm_byte_code, None, &signer)
            .unwrap()
            .data
            .code_id;
        assert_eq!(code_id, 1);

        // Wasm::Instantiate
        let instantiate_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id,
            msg: to_json_binary(&InstantiateMsg {
                admins: vec![signer.address()],
                mutable: true,
            })
            .unwrap(),
            funds: vec![],
            label: "test".to_string(),
            admin: None,
        });
        let init_res = app
            .execute_cosmos_msgs::<MsgInstantiateContractResponse>(&[instantiate_msg], &signer)
            .unwrap();
        let contract_address = init_res.data.address;
        assert_ne!(contract_address, "".to_string());

        // Wasm::Execute
        let execute_msg: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: contract_address.clone(),
            msg: to_json_binary(&ExecuteMsg::<Empty>::Freeze {}).unwrap(),
            funds: vec![],
        });
        let execute_res = app
            .execute_cosmos_msgs::<MsgExecuteContractResponse>(&[execute_msg], &signer)
            .unwrap();
        let events = execute_res.events;

        let wasm_events: Vec<Event> = events.into_iter().filter(|x| x.ty == "wasm").collect();
        for event in wasm_events.iter() {
            assert_eq!(event.attributes[0].key, "_contract_address");
            assert_eq!(event.attributes[0].value, contract_address);
            assert_eq!(event.attributes[1].key, "action");
            assert_eq!(event.attributes[1].value, "freeze");
        }

        // Stargate
        let denom = "test".to_string();
        let create_denom_msg: CosmosMsg = MsgCreateDenom {
            sender: signer.address(),
            subdenom: denom.clone(),
        }
        .into();
        let create_denom_res = app
            .execute_cosmos_msgs::<MsgCreateDenomResponse>(&[create_denom_msg], &signer)
            .unwrap();
        assert_eq!(
            create_denom_res.data.new_token_denom,
            format!("factory/{}/{}", signer.address(), denom)
        );
    }
}
