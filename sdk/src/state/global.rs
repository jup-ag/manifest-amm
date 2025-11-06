use bytemuck::{Pod, Zeroable};
use hypertree::{
    DataIndex, Get, HyperTreeReadOperations, NIL, RBNode, RedBlackTree, RedBlackTreeReadOnly,
    get_helper,
};
use solana_program_error::{ProgramError, ProgramResult};
use solana_pubkey::Pubkey;
use std::cmp::Ordering;

use crate::{
    constants::GLOBAL_FIXED_DISCRIMINANT,
    get_global_address, get_global_vault_address,
    quantities::GlobalAtoms,
    require,
    state::{DerefOrBorrow, DynamicAccount},
    validation::ManifestAccount,
};

pub type GlobalTraderTree<'a> = RedBlackTree<'a, GlobalTrader>;
pub type GlobalTraderTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, GlobalTrader>;
pub type GlobalDepositTree<'a> = RedBlackTree<'a, GlobalDeposit>;
pub type GlobalDepositTreeReadOnly<'a> = RedBlackTreeReadOnly<'a, GlobalDeposit>;

/// Fully owned Global, used in clients that can copy.
pub type GlobalValue = DynamicAccount<GlobalFixed, Vec<u8>>;
/// Full global reference type.
pub type GlobalRef<'a> = DynamicAccount<&'a GlobalFixed, &'a [u8]>;
/// Full global reference type.
pub type GlobalRefMut<'a> = DynamicAccount<&'a mut GlobalFixed, &'a mut [u8]>;

impl<Fixed: DerefOrBorrow<GlobalFixed>, Dynamic: DerefOrBorrow<[u8]>>
    DynamicAccount<Fixed, Dynamic>
{
    fn borrow_global(&self) -> GlobalRef {
        GlobalRef {
            fixed: self.fixed.deref_or_borrow(),
            dynamic: self.dynamic.deref_or_borrow(),
        }
    }

    pub fn get_balance_atoms(&self, trader: &Pubkey) -> GlobalAtoms {
        let DynamicAccount { fixed, dynamic } = self.borrow_global();
        // If the trader got evicted, then they wont be found.
        let global_balance_or: Option<&GlobalDeposit> = get_global_deposit(fixed, dynamic, trader);
        if let Some(global_deposit) = global_balance_or {
            global_deposit.balance_atoms
        } else {
            GlobalAtoms::ZERO
        }
    }
}

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod)]
pub struct GlobalFixed {
    /// Discriminant for identifying this type of account.
    pub discriminant: u64,

    /// Mint for this global
    mint: Pubkey,

    /// Vault address
    vault: Pubkey,

    /// Red-black tree root representing the global orders for the bank.
    global_traders_root_index: DataIndex,

    /// Red-black tree root representing the global deposits sorted by amount.
    global_deposits_root_index: DataIndex,
    /// Max, because the Hypertree provides access to max, but the sort key is
    /// reversed so this is the smallest balance.
    global_deposits_max_index: DataIndex,

    /// LinkedList representing all free blocks that could be used for ClaimedSeats or RestingOrders
    free_list_head_index: DataIndex,

    /// Number of bytes allocated so far.
    num_bytes_allocated: DataIndex,

    vault_bump: u8,

    /// Unused, but this byte wasnt being used anyways.
    global_bump: u8,

    num_seats_claimed: u16,
}

impl Get for GlobalFixed {}

impl GlobalFixed {
    pub fn new_empty(mint: &Pubkey) -> Self {
        let (vault, vault_bump) = get_global_vault_address(mint);
        let (_, global_bump) = get_global_address(mint);
        GlobalFixed {
            discriminant: GLOBAL_FIXED_DISCRIMINANT,
            mint: *mint,
            vault,
            global_traders_root_index: NIL,
            global_deposits_root_index: NIL,
            global_deposits_max_index: NIL,
            free_list_head_index: NIL,
            num_bytes_allocated: 0,
            vault_bump,
            global_bump,
            num_seats_claimed: 0,
        }
    }
    pub fn get_mint(&self) -> &Pubkey {
        &self.mint
    }
    pub fn get_vault(&self) -> &Pubkey {
        &self.vault
    }
    pub fn get_vault_bump(&self) -> u8 {
        self.vault_bump
    }
}

impl ManifestAccount for GlobalFixed {
    fn verify_discriminant(&self) -> ProgramResult {
        // Check the discriminant to make sure it is a global account.
        require!(
            self.discriminant == GLOBAL_FIXED_DISCRIMINANT,
            ProgramError::InvalidAccountData,
            "Invalid market discriminant actual: {} expected: {}",
            self.discriminant,
            GLOBAL_FIXED_DISCRIMINANT
        )?;
        Ok(())
    }
}

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod)]
pub struct GlobalDeposit {
    /// Trader who controls this global trader.
    trader: Pubkey,

    /// Token balance in the global account for this trader. The tokens received
    /// in trades stay in the market.
    balance_atoms: GlobalAtoms,
    _padding: u64,
}

impl Ord for GlobalDeposit {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed order so that the max according to the tree is actually the min.
        (other.balance_atoms).cmp(&(self.balance_atoms))
    }
}
impl PartialOrd for GlobalDeposit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for GlobalDeposit {
    fn eq(&self, other: &Self) -> bool {
        (self.trader) == (other.trader)
    }
}
impl Eq for GlobalDeposit {}
impl std::fmt::Display for GlobalDeposit {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.trader)
    }
}

#[repr(C)]
#[derive(Default, Copy, Clone, Zeroable, Pod)]
pub struct GlobalTrader {
    /// Trader who controls this global trader.
    trader: Pubkey,

    deposit_index: DataIndex,
    _padding: u32,
    _padding2: u64,
}

impl GlobalTrader {
    pub fn new_empty(trader: &Pubkey, deposit_index: DataIndex) -> Self {
        GlobalTrader {
            trader: *trader,
            deposit_index,
            _padding: 0,
            _padding2: 0,
        }
    }
}

impl Ord for GlobalTrader {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.trader).cmp(&(other.trader))
    }
}
impl PartialOrd for GlobalTrader {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for GlobalTrader {
    fn eq(&self, other: &Self) -> bool {
        (self.trader) == (other.trader)
    }
}
impl Eq for GlobalTrader {}
impl std::fmt::Display for GlobalTrader {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.trader)
    }
}

fn get_global_deposit<'a>(
    fixed: &'a GlobalFixed,
    dynamic: &'a [u8],
    trader: &'a Pubkey,
) -> Option<&'a GlobalDeposit> {
    let global_trader_tree: GlobalTraderTreeReadOnly =
        GlobalTraderTreeReadOnly::new(dynamic, fixed.global_traders_root_index, NIL);
    let global_trader_index: DataIndex =
        global_trader_tree.lookup_index(&GlobalTrader::new_empty(trader, NIL));
    if global_trader_index == NIL {
        return None;
    }
    let global_trader: &GlobalTrader =
        get_helper::<RBNode<GlobalTrader>>(dynamic, global_trader_index).get_value();
    let global_deposit_index: DataIndex = global_trader.deposit_index;
    Some(get_helper::<RBNode<GlobalDeposit>>(dynamic, global_deposit_index).get_value())
}
