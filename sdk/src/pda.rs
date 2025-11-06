use solana_pubkey::Pubkey;

#[macro_export]
macro_rules! market_vault_seeds {
    ( $market:expr, $mint:expr ) => {
        &[b"vault", $market.as_ref(), $mint.as_ref()]
    };
}

#[macro_export]
macro_rules! market_vault_seeds_with_bump {
    ( $market:expr, $mint:expr, $bump:expr ) => {
        &[&[b"vault", $market.as_ref(), $mint.as_ref(), &[$bump]]]
    };
}

#[macro_export]
macro_rules! global_vault_seeds {
    ( $mint:expr ) => {
        &[b"global-vault", $mint.as_ref()]
    };
}

#[macro_export]
macro_rules! global_vault_seeds_with_bump {
    ( $mint:expr, $bump:expr ) => {
        &[&[b"global-vault", $mint.as_ref(), &[$bump]]]
    };
}

pub fn get_vault_address(market: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(market_vault_seeds!(market, mint), &crate::ID)
}

pub fn get_global_vault_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(global_vault_seeds!(mint), &crate::ID)
}

macro_rules! global_seeds {
    ( $mint:expr ) => {
        &[b"global", $mint.as_ref()]
    };
}

#[macro_export]
macro_rules! global_seeds_with_bump {
    ( $mint:expr, $bump:expr ) => {
        &[&[b"global", $mint.as_ref(), &[$bump]]]
    };
}

pub fn get_global_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(global_seeds!(mint), &crate::ID)
}
