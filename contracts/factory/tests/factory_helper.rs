use anyhow::Result as AnyResult;
use cosmwasm_std::{Addr, Binary, Decimal, Uint128};
use cw20::MinterResponse;
use cw_multi_test::{App, AppResponse, ContractWrapper, Executor};
use palomadex::asset::AssetInfo;
use palomadex::factory::{
    DefaultStakeConfig, PairConfig, PairType, PartialDefaultStakeConfig, PartialStakeConfig,
    QueryMsg,
};
use palomadex::fee_config::FeeConfig;
use palomadex::pair::PairInfo;

pub struct FactoryHelper {
    pub owner: Addr,
    pub astro_token: Addr,
    pub factory: Addr,
    pub cw20_token_code_id: u64,
}

impl FactoryHelper {
    #[allow(dead_code)]
    pub fn init(router: &mut App, owner: &Addr) -> Self {
        Self::instantiate(router, owner, None)
    }

    pub fn instantiate(router: &mut App, owner: &Addr, factory_code_id: Option<u64>) -> Self {
        let astro_token_contract = Box::new(ContractWrapper::new_with_empty(
            cw20_base::contract::execute,
            cw20_base::contract::instantiate,
            cw20_base::contract::query,
        ));

        let cw20_token_code_id = router.store_code(astro_token_contract);

        let msg = cw20_base::msg::InstantiateMsg {
            name: String::from("Astro token"),
            symbol: String::from("ASTRO"),
            decimals: 6,
            initial_balances: vec![],
            mint: Some(MinterResponse {
                minter: owner.to_string(),
                cap: None,
            }),
            marketing: None,
        };

        let astro_token = router
            .instantiate_contract(
                cw20_token_code_id,
                owner.clone(),
                &msg,
                &[],
                String::from("ASTRO"),
                None,
            )
            .unwrap();

        let pair_contract = Box::new(
            ContractWrapper::new_with_empty(
                palomadex_pair::contract::execute,
                palomadex_pair::contract::instantiate,
                palomadex_pair::contract::query,
            )
            .with_reply_empty(palomadex_pair::contract::reply),
        );

        let pair_code_id = router.store_code(pair_contract);

        let factory_code_id = if let Some(factory_code_id) = factory_code_id {
            factory_code_id
        } else {
            let factory_contract = Box::new(
                ContractWrapper::new_with_empty(
                    palomadex_factory::contract::execute,
                    palomadex_factory::contract::instantiate,
                    palomadex_factory::contract::query,
                )
                .with_reply_empty(palomadex_factory::contract::reply),
            );
            router.store_code(factory_contract)
        };

        let staking_contract = Box::new(ContractWrapper::new_with_empty(
            palomadex_stake::contract::execute,
            palomadex_stake::contract::instantiate,
            palomadex_stake::contract::query,
        ));

        let staking_code_id = router.store_code(staking_contract);

        let msg = palomadex::factory::InstantiateMsg {
            pair_configs: vec![PairConfig {
                code_id: pair_code_id,
                pair_type: PairType::Xyk {},
                fee_config: FeeConfig {
                    total_fee_bps: 100,
                    protocol_fee_bps: 10,
                },
                is_disabled: false,
            }],
            token_code_id: cw20_token_code_id,
            fee_address: None,
            owner: owner.to_string(),
            max_referral_commission: Decimal::one(),
            default_stake_config: DefaultStakeConfig {
                staking_code_id,
                tokens_per_power: Uint128::new(1000),
                min_bond: Uint128::new(1000),
                unbonding_periods: vec![1, 2, 3],
                max_distributions: 6,
                converter: None,
            },
            trading_starts: None,
        };

        let factory = router
            .instantiate_contract(
                factory_code_id,
                owner.clone(),
                &msg,
                &[],
                String::from("ASTRO"),
                Some(owner.to_string()),
            )
            .unwrap();

        Self {
            owner: owner.clone(),
            astro_token,
            factory,
            cw20_token_code_id,
        }
    }

    pub fn update_config(
        &mut self,
        router: &mut App,
        sender: &Addr,
        token_code_id: Option<u64>,
        fee_address: Option<String>,
        only_owner_can_create_pairs: Option<bool>,
        default_stake_config: Option<PartialDefaultStakeConfig>,
    ) -> AnyResult<AppResponse> {
        let msg = palomadex::factory::ExecuteMsg::UpdateConfig {
            token_code_id,
            fee_address,
            only_owner_can_create_pairs,
            default_stake_config,
        };

        router.execute_contract(sender.clone(), self.factory.clone(), &msg, &[])
    }

    pub fn create_pair(
        &mut self,
        router: &mut App,
        sender: &Addr,
        pair_type: PairType,
        tokens: [&str; 2],
        init_params: Option<Binary>,
        staking_config: Option<PartialStakeConfig>,
    ) -> AnyResult<AppResponse> {
        let asset_infos = vec![
            AssetInfo::Token(tokens[0].to_owned()),
            AssetInfo::Token(tokens[1].to_owned()),
        ];

        let msg = palomadex::factory::ExecuteMsg::CreatePair {
            pair_type,
            asset_infos,
            init_params,
            staking_config: staking_config.unwrap_or_default(),
            total_fee_bps: None,
        };

        router.execute_contract(sender.clone(), self.factory.clone(), &msg, &[])
    }

    #[allow(dead_code)]
    pub fn deregister_pool_and_staking(
        &mut self,
        router: &mut App,
        sender: &Addr,
        asset_infos: Vec<AssetInfo>,
    ) -> AnyResult<AppResponse> {
        let msg = palomadex::factory::ExecuteMsg::Deregister { asset_infos };

        router.execute_contract(sender.clone(), self.factory.clone(), &msg, &[])
    }

    pub fn create_pair_with_addr(
        &mut self,
        router: &mut App,
        sender: &Addr,
        pair_type: PairType,
        tokens: [&str; 2],
        init_params: Option<Binary>,
    ) -> AnyResult<Addr> {
        self.create_pair(router, sender, pair_type, tokens, init_params, None)?;

        let asset_infos = vec![
            AssetInfo::Token(tokens[0].to_owned()),
            AssetInfo::Token(tokens[1].to_owned()),
        ];

        let res: PairInfo = router
            .wrap()
            .query_wasm_smart(self.factory.clone(), &QueryMsg::Pair { asset_infos })?;

        Ok(res.contract_addr)
    }

    #[allow(dead_code)]
    pub fn update_pair_fees(
        &mut self,
        router: &mut App,
        sender: &Addr,
        asset_infos: Vec<AssetInfo>,
        fee_config: FeeConfig,
    ) -> AnyResult<AppResponse> {
        let msg = palomadex::factory::ExecuteMsg::UpdatePairFees {
            asset_infos,
            fee_config,
        };

        router.execute_contract(sender.clone(), self.factory.clone(), &msg, &[])
    }
}

pub fn instantiate_token(
    app: &mut App,
    token_code_id: u64,
    owner: &Addr,
    token_name: &str,
    decimals: Option<u8>,
) -> Addr {
    let init_msg = cw20_base::msg::InstantiateMsg {
        name: token_name.to_string(),
        symbol: token_name.to_string(),
        decimals: decimals.unwrap_or(6),
        initial_balances: vec![],
        mint: Some(MinterResponse {
            minter: owner.to_string(),
            cap: None,
        }),
        marketing: None,
    };

    app.instantiate_contract(
        token_code_id,
        owner.clone(),
        &init_msg,
        &[],
        token_name,
        Some(owner.to_string()),
    )
    .unwrap()
}
