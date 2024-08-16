use anyhow::Result as AnyResult;

use cw20::MinterResponse;
use palomadex::asset::AssetInfo;
use palomadex::factory::{
    DefaultStakeConfig, ExecuteMsg as FactoryExecuteMsg, InstantiateMsg as FactoryInstantiateMsg,
    PairConfig, PairType, PartialStakeConfig, QueryMsg as FactoryQueryMsg,
};
use palomadex::fee_config::FeeConfig;
use palomadex::pair::{PairInfo, PoolResponse, QueryMsg as PairQueryMsg};

use crate::msg::{MigrateMsg, OrigMigrateMsg, QueryMsg};
use cosmwasm_std::{coin, to_binary, Addr, Coin, Decimal, Uint128};
use cw20_base::msg::InstantiateMsg as Cw20BaseInstantiateMsg;
use cw_multi_test::{next_block, App, AppResponse, BankSudo, ContractWrapper, Executor, SudoMsg};
use stake_cw20::msg::{ClaimsResponse, InstantiateMsg as StakeCw20IntantiateMsg};
use wasmswap::msg::{
    ExecuteMsg as WasmswapExecuteMsg, InfoResponse, InstantiateMsg as WasmswapInstantiateMsg,
    QueryMsg as WasmswapQueryMsg,
};
use wasmswap_cw20::{Cw20ExecuteMsg, Denom};
use wasmswap_cw_utils::Duration;

pub const ONE_DAY: u64 = 86_400;

pub fn store_stake_cw20(app: &mut App) -> u64 {
    let contract = Box::new(ContractWrapper::new(
        stake_cw20::contract::execute,
        stake_cw20::contract::instantiate,
        stake_cw20::contract::query,
    ));
    app.store_code(contract)
}

pub fn store_junoswap_pool(app: &mut App) -> u64 {
    let contract = Box::new(
        ContractWrapper::new(
            wasmswap::contract::execute,
            wasmswap::contract::instantiate,
            wasmswap::contract::query,
        )
        .with_reply_empty(wasmswap::contract::reply),
    );
    app.store_code(contract)
}

pub fn store_cw20(app: &mut App) -> u64 {
    let contract = Box::new(ContractWrapper::new(
        wasmswap_cw20_base::contract::execute,
        wasmswap_cw20_base::contract::instantiate,
        wasmswap_cw20_base::contract::query,
    ));
    app.store_code(contract)
}

pub fn store_palomadex_staking(app: &mut App) -> u64 {
    let contract = Box::new(ContractWrapper::new(
        palomadex_stake::contract::execute,
        palomadex_stake::contract::instantiate,
        palomadex_stake::contract::query,
    ));
    app.store_code(contract)
}

fn store_palomadex_factory(app: &mut App) -> u64 {
    let factory_contract = Box::new(
        ContractWrapper::new_with_empty(
            palomadex_factory::contract::execute,
            palomadex_factory::contract::instantiate,
            palomadex_factory::contract::query,
        )
        .with_reply_empty(palomadex_factory::contract::reply),
    );

    app.store_code(factory_contract)
}

fn store_palomadex_pair(app: &mut App) -> u64 {
    let factory_contract = Box::new(
        ContractWrapper::new_with_empty(
            palomadex_pair::contract::execute,
            palomadex_pair::contract::instantiate,
            palomadex_pair::contract::query,
        )
        .with_reply_empty(palomadex_pair::contract::reply),
    );

    app.store_code(factory_contract)
}

pub fn store_migrator(app: &mut App) -> u64 {
    let contract = Box::new(
        ContractWrapper::new(
            crate::contract::execute,
            crate::contract::instantiate,
            crate::contract::query,
        )
        .with_reply(crate::contract::reply),
    );
    app.store_code(contract)
}

/// Helper to be able to specify a non-existing cw20 token
#[derive(Debug, Clone)]
pub enum PoolDenom {
    Native(String),
    /// The string is not the contract address, but the symbol / name of the token.
    /// A new token will be instantiated using this name.
    Cw20(String),
}

impl PoolDenom {
    pub fn into_denom(
        self,
        app: &mut App,
        owner: impl Into<String> + Clone,
        cw20_code_id: u64,
    ) -> Denom {
        match self {
            PoolDenom::Native(denom) => Denom::Native(denom),
            PoolDenom::Cw20(symbol) => {
                // create cw20 token
                let cw20_token = app
                    .instantiate_contract(
                        cw20_code_id,
                        Addr::unchecked(owner.clone()),
                        &Cw20BaseInstantiateMsg {
                            name: symbol.clone(),
                            symbol: symbol.clone(),
                            decimals: 6,
                            initial_balances: vec![],
                            mint: Some(MinterResponse {
                                minter: owner.into(),
                                cap: None,
                            }),
                            marketing: None,
                        },
                        &[],
                        symbol,
                        None,
                    )
                    .unwrap();
                Denom::Cw20(cw20_token)
            }
        }
    }
}

#[derive(Debug)]
pub struct SuiteBuilder {
    funds: Vec<(Addr, Vec<Coin>)>,
    unbonding_periods: Vec<u64>,
    pool_denom1: PoolDenom,
    pool_denom2: PoolDenom,
}

impl SuiteBuilder {
    pub fn new() -> SuiteBuilder {
        SuiteBuilder {
            unbonding_periods: vec![100, 200, 300],
            pool_denom1: PoolDenom::Native("ujuno".to_string()),
            pool_denom2: PoolDenom::Native("uluna".to_string()),
            funds: vec![],
        }
    }

    /// Specify the pool denoms. For cw20 denoms, the
    pub fn with_denoms(mut self, denom1: PoolDenom, denom2: PoolDenom) -> Self {
        self.pool_denom1 = denom1;
        self.pool_denom2 = denom2;
        self
    }

    #[track_caller]
    pub fn build(self) -> Suite {
        let mut app = App::default();
        let owner = Addr::unchecked("owner");

        let cw20_code_id = store_cw20(&mut app);

        let pool_denom1 = self
            .pool_denom1
            .into_denom(&mut app, owner.clone(), cw20_code_id);
        let pool_denom2 = self
            .pool_denom2
            .into_denom(&mut app, owner.clone(), cw20_code_id);

        // Instantiate junoswap pool
        let junoswap_pool_code_id = store_junoswap_pool(&mut app);
        let junoswap_pool_contract = app
            .instantiate_contract(
                junoswap_pool_code_id,
                owner.clone(),
                &WasmswapInstantiateMsg {
                    token1_denom: pool_denom1.clone(),
                    token2_denom: pool_denom2.clone(),
                    lp_token_code_id: cw20_code_id,
                    owner: Some(owner.to_string()),
                    lp_fee_percent: Decimal::zero(),
                    protocol_fee_percent: Decimal::zero(),
                    protocol_fee_recipient: owner.to_string(),
                },
                &[],
                "wasmswap-pool",
                Some(owner.to_string()),
            )
            .unwrap();
        app.update_block(next_block);

        // Check address of created token contract
        let junoswap_token_contract = Addr::unchecked(
            app.wrap()
                .query_wasm_smart::<InfoResponse>(
                    &junoswap_pool_contract,
                    &WasmswapQueryMsg::Info {},
                )
                .unwrap()
                .lp_token_address,
        );

        // Instantiate junoswap staking contract
        let junoswap_staking_code_id = store_stake_cw20(&mut app);
        let junoswap_staking_contract = app
            .instantiate_contract(
                junoswap_staking_code_id,
                owner.clone(),
                &StakeCw20IntantiateMsg {
                    owner: Some(owner.to_string()),
                    manager: Some("manager".to_string()),
                    token_address: junoswap_token_contract.to_string(),
                    unstaking_duration: Some(Duration::Time(ONE_DAY * 14)),
                },
                &[],
                "staking",
                Some(owner.to_string()),
            )
            .unwrap();
        app.update_block(next_block);

        // Instantiate palomadex factory
        let palomadex_stake_code_id = store_palomadex_staking(&mut app);
        let palomadex_pair_code_id = store_palomadex_pair(&mut app);
        let palomadex_factory_code_id = store_palomadex_factory(&mut app);
        let factory_contract = app
            .instantiate_contract(
                palomadex_factory_code_id,
                owner.clone(),
                &FactoryInstantiateMsg {
                    pair_configs: vec![PairConfig {
                        pair_type: PairType::Xyk {},
                        code_id: palomadex_pair_code_id,
                        fee_config: FeeConfig {
                            total_fee_bps: 0,
                            protocol_fee_bps: 0,
                        },
                        is_disabled: false,
                    }],
                    token_code_id: cw20_code_id,
                    fee_address: Some(owner.to_string()),
                    owner: owner.to_string(),
                    max_referral_commission: Decimal::one(),
                    default_stake_config: DefaultStakeConfig {
                        staking_code_id: palomadex_stake_code_id,
                        tokens_per_power: Uint128::new(1000),
                        min_bond: Uint128::new(1000),
                        unbonding_periods: self.unbonding_periods.clone(),
                        max_distributions: 6,
                        converter: None,
                    },
                    trading_starts: None,
                },
                &[],
                "palomadex-factory",
                Some(owner.to_string()),
            )
            .unwrap();

        let wynd_cw20_token = app
            .instantiate_contract(
                cw20_code_id,
                Addr::unchecked(owner.clone()),
                &Cw20BaseInstantiateMsg {
                    name: "PALOMA".to_owned(),
                    symbol: "PALOMA".to_owned(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: owner.to_string(),
                        cap: None,
                    }),
                    marketing: None,
                },
                &[],
                "PALOMA".to_owned(),
                None,
            )
            .unwrap();

        // Wasmswap is using older version of cw20, so specific From impl
        // would have to be created - IMO not worth it
        let asset_infos = vec![
            match pool_denom1.clone() {
                Denom::Native(s) => AssetInfo::Native(s),
                Denom::Cw20(s) => AssetInfo::Token(s.to_string()),
            },
            AssetInfo::Token(wynd_cw20_token.to_string()),
        ];

        // Instantiate palomadex pair contract through factory
        app.execute_contract(
            owner.clone(),
            factory_contract.clone(),
            &FactoryExecuteMsg::CreatePair {
                pair_type: PairType::Xyk {},
                asset_infos: asset_infos.clone(),
                init_params: None,
                total_fee_bps: None,
                // accept defaults, but ensure there is a staking contract
                staking_config: PartialStakeConfig {
                    staking_code_id: None,
                    tokens_per_power: None,
                    min_bond: None,
                    unbonding_periods: None,
                    max_distributions: None,
                    converter: None,
                },
            },
            &[],
        )
        .unwrap();
        let pair_info = app
            .wrap()
            .query_wasm_smart::<PairInfo>(
                Addr::unchecked(&factory_contract),
                &FactoryQueryMsg::Pair { asset_infos },
            )
            .unwrap();

        let palomadex_pair_contract = pair_info.contract_addr;
        let palomadex_staking_contract = pair_info.staking_addr;
        let palomadex_token_contract = pair_info.liquidity_token;

        // add funds to the contract
        let funds = self.funds;
        app.init_modules(|router, _, storage| -> AnyResult<()> {
            for (addr, coin) in funds {
                router.bank.init_balance(storage, &addr, coin)?;
            }
            Ok(())
        })
        .unwrap();

        let migrator_code_id = store_migrator(&mut app);

        Suite {
            owner,
            app,
            junoswap_token_contract,
            junoswap_pool_contract,
            junoswap_staking_contract,
            factory_contract,
            palomadex_pair_contract,
            palomadex_staking_contract,
            palomadex_token_contract,
            grain_cw20_token: wynd_cw20_token,
            migrator_code_id,
            cw20_code_id,
            pool_denom1,
            pool_denom2,
            unbonding_periods: self.unbonding_periods,
        }
    }
}

pub struct Suite {
    pub owner: Addr,
    pub app: App,
    pub migrator_code_id: u64,
    pub cw20_code_id: u64,
    pub unbonding_periods: Vec<u64>,

    pub junoswap_token_contract: Addr,
    pub junoswap_pool_contract: Addr,
    pub junoswap_staking_contract: Addr,
    pub grain_cw20_token: Addr,
    pub palomadex_token_contract: Addr,
    pub palomadex_staking_contract: Addr,
    pub palomadex_pair_contract: Addr,
    pub pool_denom1: Denom,
    pub pool_denom2: Denom,

    pub factory_contract: Addr,
}

#[derive(Debug)]
#[allow(dead_code)]
struct SuiteInfo<'a> {
    pub junoswap_token_contract: &'a Addr,
    pub junoswap_pool_contract: &'a Addr,
    pub junoswap_staking_contract: &'a Addr,
    pub factory_contract: &'a Addr,
    pub palomadex_token_contract: &'a Addr,
    pub palomadex_staking_contract: &'a Addr,
    pub palomadex_pair_contract: &'a Addr,
}

impl Suite {
    // for debugging tests
    #[allow(dead_code)]
    pub fn info(&self) {
        let info = SuiteInfo {
            junoswap_token_contract: &self.junoswap_token_contract,
            junoswap_pool_contract: &self.junoswap_pool_contract,
            junoswap_staking_contract: &self.junoswap_staking_contract,
            factory_contract: &self.factory_contract,
            palomadex_token_contract: &self.palomadex_token_contract,
            palomadex_staking_contract: &self.palomadex_staking_contract,
            palomadex_pair_contract: &self.palomadex_pair_contract,
        };
        println!("{:?}", info);
    }

    pub fn migration_unbonding_period(&self) -> u64 {
        self.unbonding_periods[1]
    }

    /// Returns true if migration is finished
    /// Only makes sense to call after the junoswap staking contract has been migrated
    pub fn migration_finished(&self) -> AnyResult<bool> {
        self.app
            .wrap()
            .query_wasm_smart(
                self.junoswap_staking_contract.clone(),
                &QueryMsg::MigrationFinished {},
            )
            .map_err(Into::into)
    }

    /// Migrates the junoswap staking contract to our migration contract and migrates the tokens
    pub fn migrate_tokens(
        &mut self,
        migrator: Option<Addr>,
        palomadex_pair_migrate: Option<Addr>,
        palomadex_pair: Option<Addr>,
        raw_to_wynd_exchange_rate: Decimal,
        raw_address: Addr,
    ) -> AnyResult<AppResponse> {
        // // take RAW token's address by force. We know it is the one.
        // let raw_address = match self.pool_denom2.clone() {
        //     Denom::Cw20(address) => address,
        //     _ => todo!(),
        // };
        // first set up the migration
        self.app.migrate_contract(
            self.owner.clone(),
            self.junoswap_staking_contract.clone(),
            &MigrateMsg {
                init: Some(OrigMigrateMsg {
                    migrator: migrator.unwrap_or_else(|| self.owner.clone()).to_string(),
                    unbonding_period: self.migration_unbonding_period(),
                    junoswap_pool: self.junoswap_pool_contract.to_string(),
                    factory: self.factory_contract.to_string(),
                    palomadex_pool: palomadex_pair_migrate.map(|p| p.to_string()),
                    raw_to_grain_exchange_rate: raw_to_wynd_exchange_rate,
                    raw_address: raw_address.to_string(),
                    grain_address: self.grain_cw20_token.to_string(),
                }),
            },
            self.migrator_code_id,
        )?;

        // then migrate again (self-migrate) to ensure it works
        self.app.migrate_contract(
            self.owner.clone(),
            self.junoswap_staking_contract.clone(),
            &MigrateMsg { init: None },
            self.migrator_code_id,
        )?;

        // then trigger the actual migration
        self.app.execute_contract(
            self.owner.clone(),
            self.junoswap_staking_contract.clone(),
            &crate::msg::ExecuteMsg::MigrateTokens {
                palomadex_pool: palomadex_pair
                    .unwrap_or_else(|| self.palomadex_pair_contract.clone())
                    .to_string(),
            },
            &[],
        )
    }

    /// Migrates the next `limit` staker's LP tokens.
    /// Only makes sense to call after the junoswap staking contract has been migrated.
    pub fn migrate_stakers(&mut self, limit: u32) -> AnyResult<AppResponse> {
        self.app.execute_contract(
            self.owner.clone(),
            self.junoswap_staking_contract.clone(),
            &crate::msg::ExecuteMsg::MigrateStakers { limit },
            &[],
        )
    }

    pub fn migrate_to_palomadex(
        &mut self,
        migrator: Option<Addr>,
        palomadex_pair_migrate: Option<Addr>,
        palomadex_pair: Option<Addr>,
        raw_to_wynd_exchange_rate: Decimal,
        raw_address: Addr,
    ) -> AnyResult<()> {
        self.migrate_tokens(
            migrator,
            palomadex_pair_migrate,
            palomadex_pair,
            raw_to_wynd_exchange_rate,
            raw_address,
        )?;

        // now migrate all the stakers
        while !self.migration_finished()? {
            self.migrate_stakers(10)?;
        }

        Ok(())
    }

    fn increase_allowance(
        &mut self,
        owner: &str,
        contract: &Addr,
        spender: &str,
        amount: u128,
    ) -> AnyResult<AppResponse> {
        self.app.execute_contract(
            Addr::unchecked(owner),
            contract.clone(),
            &Cw20ExecuteMsg::IncreaseAllowance {
                spender: spender.to_owned(),
                amount: amount.into(),
                expires: None,
            },
            &[],
        )
    }

    pub fn mint_cw20(
        &mut self,
        owner: &str,
        token: &Addr,
        amount: u128,
        recipient: &str,
    ) -> AnyResult<AppResponse> {
        self.app.execute_contract(
            Addr::unchecked(owner),
            token.clone(),
            &Cw20ExecuteMsg::Mint {
                recipient: recipient.to_owned(),
                amount: amount.into(),
            },
            &[],
        )
    }

    pub fn junoswap_lp(&mut self, user: &str, lp_contract: Option<&Addr>) -> AnyResult<Uint128> {
        let query = cw20::Cw20QueryMsg::Balance {
            address: user.to_string(),
        };
        let cw20::BalanceResponse { balance } = self
            .app
            .wrap()
            .query_wasm_smart(lp_contract.unwrap_or(&self.junoswap_token_contract), &query)?;
        Ok(balance)
    }

    /// Requirement: if using native token provide coins to sent as last argument
    #[allow(clippy::too_many_arguments)]
    pub fn provide_liquidity_to_junoswap_pool(
        &mut self,
        user: &str,
        first_asset: u128,
        second_asset: u128,
        first_denom: Option<Denom>,
        second_denom: Option<Denom>,
        native_tokens: Vec<Coin>,
    ) -> AnyResult<AppResponse> {
        let owner = self.owner.to_string();

        let assets = vec![
            (
                first_denom.unwrap_or_else(|| self.pool_denom1.clone()),
                first_asset,
            ),
            (
                second_denom.unwrap_or_else(|| self.pool_denom2.clone()),
                second_asset,
            ),
        ];
        for (denom, amount) in assets {
            match denom {
                Denom::Cw20(addr) => {
                    // Mint some initial balances for whale user
                    self.mint_cw20(&owner, &addr, amount, user).unwrap();
                    // Increases allowances for given LP contracts in order to provide liquidity to pool
                    let spender = self.junoswap_pool_contract.to_string();
                    self.increase_allowance(user, &addr, &spender, amount)
                        .unwrap();
                }
                Denom::Native(denom) => {
                    self.app
                        .sudo(SudoMsg::Bank(BankSudo::Mint {
                            to_address: user.to_owned(),
                            amount: vec![coin(amount, denom)],
                        }))
                        .unwrap();
                }
            }
        }

        self.app.execute_contract(
            Addr::unchecked(user),
            self.junoswap_pool_contract.clone(),
            &WasmswapExecuteMsg::AddLiquidity {
                token1_amount: first_asset.into(),
                min_liquidity: Uint128::new(100),
                max_token2: second_asset.into(),
                expiration: None,
            },
            &native_tokens,
        )
    }

    pub fn stake_junoswap_lp(
        &mut self,
        user: &str,
        amount: Uint128,
        lp_contract: Option<&Addr>,
        staking_contract: Option<&Addr>,
    ) -> AnyResult<AppResponse> {
        let msg = to_binary(&stake_cw20::msg::ReceiveMsg::Stake {})?;
        self.app.execute_contract(
            Addr::unchecked(user),
            lp_contract
                .unwrap_or(&self.junoswap_token_contract.clone())
                .to_owned(),
            &cw20::Cw20ExecuteMsg::Send {
                contract: staking_contract
                    .unwrap_or(&self.junoswap_staking_contract.clone())
                    .to_string(),
                amount,
                msg,
            },
            &[],
        )
    }

    pub fn unstake_junoswap_lp(
        &mut self,
        user: &str,
        amount: Uint128,
        staking_contract: Option<&Addr>,
    ) -> AnyResult<AppResponse> {
        self.app.execute_contract(
            Addr::unchecked(user),
            staking_contract
                .unwrap_or(&self.junoswap_staking_contract)
                .clone(),
            &stake_cw20::msg::ExecuteMsg::Unstake { amount },
            &[],
        )
    }

    #[allow(dead_code)]
    pub fn query_stake_claims_for_pair(&mut self, address: String) -> ClaimsResponse {
        let resp: ClaimsResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.junoswap_staking_contract,
                &stake_cw20::msg::QueryMsg::Claims { address },
            )
            .unwrap();
        resp
    }

    pub fn query_palomadex_pool(&mut self) -> PoolResponse {
        self.app
            .wrap()
            .query_wasm_smart::<PoolResponse>(&self.palomadex_pair_contract, &PairQueryMsg::Pool {})
            .unwrap()
    }

    pub fn total_palomadex_lp(&mut self) -> u128 {
        let cw20::TokenInfoResponse { total_supply, .. } = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.palomadex_token_contract,
                &cw20::Cw20QueryMsg::TokenInfo {},
            )
            .unwrap();

        total_supply.u128()
    }

    pub fn palomadex_lp(&mut self, user: &str) -> u128 {
        let cw20::BalanceResponse { balance } = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.palomadex_token_contract,
                &cw20::Cw20QueryMsg::Balance {
                    address: user.to_string(),
                },
            )
            .unwrap();

        balance.u128()
    }

    // for debugging tests
    #[allow(dead_code)]
    pub fn palomadex_lp_holders(&mut self) -> Vec<(String, u128)> {
        let cw20::AllAccountsResponse { accounts } = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.palomadex_token_contract,
                &cw20::Cw20QueryMsg::AllAccounts {
                    start_after: None,
                    limit: None,
                },
            )
            .unwrap();
        accounts
            .into_iter()
            .map(|addr| (addr.clone(), self.palomadex_lp(&addr)))
            .collect()
    }

    pub fn total_palomadex_staked(&mut self) -> u128 {
        let addr = self.palomadex_staking_contract.clone();
        self.palomadex_lp(addr.as_str())
    }

    pub fn palomadex_staked(&mut self, user: &str, unbonding_period: u64) -> u128 {
        let palomadex_stake::msg::StakedResponse { stake, .. } = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.palomadex_staking_contract,
                &palomadex_stake::msg::QueryMsg::Staked {
                    address: user.to_string(),
                    unbonding_period,
                },
            )
            .unwrap();

        stake.u128()
    }
}
