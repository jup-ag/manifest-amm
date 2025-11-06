use solana_pubkey::Pubkey;

use crate::{
    state::GlobalFixed,
    validation::{
        ManifestAccountInfo, MintAccountInfo, Program, Signer, TokenAccountInfo, TokenProgram,
    },
};

/// Accounts needed to make a global trade. Scope is beyond just crate so
/// clients can place orders on markets in testing.
pub struct GlobalTradeAccounts<'a, 'info> {
    /// Required if this is a token22 token.
    pub mint_opt: Option<MintAccountInfo<'a, 'info>>,
    pub global: ManifestAccountInfo<'a, 'info, GlobalFixed>,

    // These are required when matching a global order, not necessarily when
    // cancelling since tokens dont move in that case.
    pub global_vault_opt: Option<TokenAccountInfo<'a, 'info>>,
    pub market_vault_opt: Option<TokenAccountInfo<'a, 'info>>,
    pub token_program_opt: Option<TokenProgram<'a, 'info>>,

    pub system_program: Option<Program<'a, 'info>>,

    // Trader is sending or cancelling the order. They are the one who will pay
    // or receive gas prepayments.
    pub gas_payer_opt: Option<Signer<'a, 'info>>,
    pub gas_receiver_opt: Option<Signer<'a, 'info>>,
    pub market: Pubkey,
}
