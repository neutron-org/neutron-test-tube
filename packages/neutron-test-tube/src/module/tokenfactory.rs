use neutron_std::types::osmosis::tokenfactory::v1beta1::{
    MsgBurn, MsgBurnResponse, MsgChangeAdmin, MsgChangeAdminResponse, MsgCreateDenom,
    MsgCreateDenomResponse, MsgMint, MsgMintResponse, MsgSetDenomMetadata,
    MsgSetDenomMetadataResponse, MsgUpdateParams, MsgUpdateParamsResponse,
    QueryDenomAuthorityMetadataRequest, QueryDenomAuthorityMetadataResponse,
    QueryDenomsFromCreatorRequest, QueryDenomsFromCreatorResponse, QueryParamsRequest,
    QueryParamsResponse
};

use test_tube_ntrn::module::Module;
use test_tube_ntrn::runner::Runner;
use test_tube_ntrn::{fn_execute, fn_query};

pub struct TokenFactory<'a, R: Runner<'a>> {
    runner: &'a R,
}

impl<'a, R: Runner<'a>> Module<'a, R> for TokenFactory<'a, R> {
    fn new(runner: &'a R) -> Self {
        Self { runner }
    }
}

impl<'a, R> TokenFactory<'a, R>
where
    R: Runner<'a>,
{
    fn_execute! {
        pub create_denom: MsgCreateDenom ["/osmosis.tokenfactory.v1beta1.MsgCreateDenom"] => MsgCreateDenomResponse
    }

    fn_execute! {
        pub mint: MsgMint ["/osmosis.tokenfactory.v1beta1.MsgMint"]  => MsgMintResponse
    }

    fn_execute! {
        pub burn: MsgBurn ["/osmosis.tokenfactory.v1beta1.MsgBurn"] => MsgBurnResponse
    }

    fn_execute! {
        pub change_admin: MsgChangeAdmin ["/osmosis.tokenfactory.v1beta1.MsgChangeAdmin"]  => MsgChangeAdminResponse
    }

    fn_execute! {
        pub set_denom_metadata: MsgSetDenomMetadata  ["/osmosis.tokenfactory.v1beta1.MsgSetDenomMetadata"]  => MsgSetDenomMetadataResponse
    }

    fn_query! {
        pub query_params ["/osmosis.tokenfactory.v1beta1.Query/Params"]: QueryParamsRequest => QueryParamsResponse
    }

    fn_query! {
        pub query_denom_authority_metadata ["/osmosis.tokenfactory.v1beta1.Query/DenomAuthorityMetadata"]: QueryDenomAuthorityMetadataRequest => QueryDenomAuthorityMetadataResponse
    }

    fn_query! {
        pub query_denoms_from_creator ["/osmosis.tokenfactory.v1beta1.Query/DenomsFromCreator"]: QueryDenomsFromCreatorRequest => QueryDenomsFromCreatorResponse
    }

    fn_execute! {
        pub update_params: MsgUpdateParams  ["/osmosis.tokenfactory.v1beta1.MsgUpdateParams"]  => MsgUpdateParamsResponse
    }
}

#[cfg(test)]
mod tests {
    use cosmos_sdk_proto::cosmos::bank::v1beta1::QueryBalanceRequest;
    use cosmwasm_std::{coins, Coin, Uint128};
    use neutron_std::shim::Any;
    use neutron_std::types::cosmos::adminmodule::adminmodule::MsgSubmitProposal;
    use neutron_std::types::osmosis::tokenfactory::v1beta1::{MsgBurn, MsgCreateDenom, MsgMint, MsgUpdateParams, QueryDenomsFromCreatorRequest};
    use neutron_std::types::osmosis::tokenfactory::Params;
    use prost::Message;

    use test_tube_ntrn::Module;

    use crate::{Account, Adminmodule, Bank, NeutronTestApp, TokenFactory};

    #[test]
    fn tokenfactory_integration() {
        let app = NeutronTestApp::new();
        let signer = app
            .init_account(
                &[Coin {
                    denom: "untrn".to_string(),
                    amount: Uint128::new(100_000_000_000_000_000_000),
                }],
                false,
            )
            .unwrap();
        let tokenfactory = TokenFactory::new(&app);
        let bank = Bank::new(&app);

        // create denom
        let subdenom = "udenom";

        // assert_eq!(1, 2);
        let denom = tokenfactory
            .create_denom(
                MsgCreateDenom {
                    sender: signer.address(),
                    subdenom: subdenom.to_owned(),
                },
                &signer,
            )
            .unwrap()
            .data
            .new_token_denom;

        assert_eq!(format!("factory/{}/{}", signer.address(), subdenom), denom);

        // denom from creator
        let denoms = tokenfactory
            .query_denoms_from_creator(&QueryDenomsFromCreatorRequest {
                creator: signer.address(),
            })
            .unwrap()
            .denoms;

        assert_eq!(denoms, [denom.clone()]);
        // TODO mint new denom
        let coin: neutron_std::types::cosmos::base::v1beta1::Coin =
            neutron_std::types::cosmos::base::v1beta1::Coin {
                amount: Uint128::new(1000000000).to_string(),
                denom: denom.clone(),
            };
        tokenfactory
            .mint(
                MsgMint {
                    sender: signer.address(),
                    amount: Some(coin.clone()),
                    mint_to_address: signer.address(),
                },
                &signer,
            )
            .unwrap();

        let balance = bank
            .query_balance(&QueryBalanceRequest {
                address: signer.address(),
                denom: denom.clone(),
            })
            .unwrap()
            .balance
            .unwrap();

        assert_eq!(coin.amount, balance.amount);
        assert_eq!(coin.denom, balance.denom);

        // burn
        tokenfactory
            .burn(
                MsgBurn {
                    sender: signer.address(),
                    amount: Some(coin.clone()),
                    burn_from_address: signer.address(),
                },
                &signer,
            )
            .unwrap();

        let balance = bank
            .query_balance(&QueryBalanceRequest {
                address: signer.address(),
                denom,
            })
            .unwrap()
            .balance
            .unwrap();

        assert_eq!("0", balance.amount);
        assert_eq!(coin.denom, balance.denom);
    }

    #[test]
    fn test_set_bank_hook() {
        let app = NeutronTestApp::default();
        let adm = Adminmodule::new(&app);

        // we creating an addr which could send proposals directly
        let admin = app
            .init_account(&coins(1_000_000_000_000u128, "untrn"), true)
            .unwrap();
        // address of admin moudulele. it is an authority for all modules
        let adminmodule_addr = "neutron1hxskfdxpp5hqgtjj6am6nkjefhfzj359x0ar3z";

        // tokenfactory update params messaage
        let tfmsg = MsgUpdateParams {
            authority: adminmodule_addr.to_string(),
            params: Some(Params {
                // set proper params & hooks below
                denom_creation_fee: vec![],
                denom_creation_gas_consume: Some(0),
                fee_collector_address: "".to_string(),
                whitelisted_hooks: vec![],
            }),
        };

        // encode it to Any
        let tfmsg_any = Any {
            type_url: "/osmosis.tokenfactory.v1beta1.MsgUpdateParams".to_string(),
            value: tfmsg.encode_to_vec(),
        };

        // submit as a proposal
        let msg = MsgSubmitProposal{messages: vec![tfmsg_any], proposer: admin.address()};

        adm.submit_proposal(
            msg,
            &admin,
        )
        .unwrap();
    }
}
