use std::cell::RefMut;

use const_crypto::sha3::Keccak256;
use hypertree::{Get, get_mut_helper};
use solana_pubkey::Pubkey;

use crate::{
    quantities::GlobalAtoms,
    state::{DynamicAccount, GlobalRefMut},
    validation::GlobalTradeAccounts,
};

pub trait TypeName {
    const NAME: &'static str;
}

/// Canonical discriminant of the given struct. It is the hash of program ID and
/// the name of the type.
pub const fn get_discriminant<T: TypeName>() -> u64 {
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = Keccak256::new()
        .update(crate::ID.as_array())
        .update(T::NAME.as_bytes())
        .finalize();

    u64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, b7])
}

pub(crate) fn can_back_order<'a, 'info>(
    global_trade_accounts_opt: &'a Option<GlobalTradeAccounts<'a, 'info>>,
    resting_order_trader: &Pubkey,
    desired_global_atoms: GlobalAtoms,
) -> bool {
    let Some(global_trade_accounts) = global_trade_accounts_opt.as_ref() else {
        return false;
    };
    let GlobalTradeAccounts { global, .. } = global_trade_accounts;

    let Ok(mut global_data_ref) = global.try_borrow_mut_data() else {
        // If the account data is already borrowed, conservatively disallow the back order.
        return false;
    };
    let global_data: &mut RefMut<&mut [u8]> = &mut global_data_ref;
    let global_dynamic_account: GlobalRefMut = get_mut_dynamic_account(global_data);

    let num_deposited_atoms: GlobalAtoms =
        global_dynamic_account.get_balance_atoms(resting_order_trader);
    return desired_global_atoms <= num_deposited_atoms;
}

/// Generic get mutable dynamic account from the data bytes of the account.
pub fn get_mut_dynamic_account<'a, T: Get>(
    data: &'a mut RefMut<'_, &mut [u8]>,
) -> DynamicAccount<&'a mut T, &'a mut [u8]> {
    let (fixed_data, dynamic) = data.split_at_mut(size_of::<T>());
    let fixed: &mut T = get_mut_helper::<T>(fixed_data, 0_u32);

    let dynamic_account: DynamicAccount<&'a mut T, &'a mut [u8]> =
        DynamicAccount { fixed, dynamic };
    dynamic_account
}

#[test]
fn test_get_discriminant() {
    // Update this when updating program id.
    assert_eq!(
        get_discriminant::<crate::state::MarketFixed>(),
        4859840929024028656
    );
}
